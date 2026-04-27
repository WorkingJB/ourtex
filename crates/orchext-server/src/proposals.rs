//! Proposal review queue.
//!
//! `context_propose` (the MCP tool, see `mcp.rs`) writes a row here; the
//! routes in this module let an admin list pending proposals, view one,
//! and approve or reject. Approval applies the patch to the target
//! document under the same base-version optimistic-concurrency rule the
//! `documents::write_doc` path uses, then bumps the document's version.
//! Reject is a status update + audit entry; nothing in the vault moves.
//!
//! Permission model: the routes are tenant-scoped, so the standard
//! `tenant_auth` middleware already gates on membership. We additionally
//! require `is_admin()` (owner / admin role per D11). In personal
//! workspaces the lone member is owner so this is a no-op; in team
//! workspaces it matches the spec ("admin review" of member-issued
//! proposals).
//!
//! Encryption interaction: a proposal targets a document that may be
//! stored as `body_ciphertext`. Approval needs the live session key to
//! decrypt → patch → re-encrypt. If no key is published the call fails
//! with `vault_locked`, same as a direct write.

use crate::{
    audit::{self, Actor, AppendRecord, Outcome},
    error::ApiError,
    sessions::SessionContext,
    tenants::TenantContext,
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use orchext_vault::Document;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::FromRow;
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/proposals", get(list_proposals))
        .route("/proposals/:id", get(get_proposal))
        .route("/proposals/:id/approve", post(approve_proposal))
        .route("/proposals/:id/reject", post(reject_proposal))
}

// ---------- DTOs ----------

#[derive(Debug, Serialize, FromRow)]
pub struct ProposalRow {
    pub id: String,
    pub doc_id: String,
    pub base_version: String,
    pub patch: JsonValue,
    pub reason: Option<String>,
    pub status: String,
    pub actor_token_id: Option<String>,
    pub actor_token_label: String,
    pub actor_account_id: Option<Uuid>,
    pub decided_by: Option<Uuid>,
    pub decided_at: Option<DateTime<Utc>>,
    pub decision_note: Option<String>,
    pub applied_version: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct ListResponse {
    proposals: Vec<ProposalRow>,
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    /// `pending` (default), `approved`, `rejected`, or `all`.
    status: Option<String>,
    /// Caps at 200; defaults to 50.
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct DecideRequest {
    /// Optional human-readable note saved alongside the decision.
    /// Useful for "rejected because the patch was too aggressive,
    /// please re-propose narrower" feedback.
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Serialize)]
struct ApproveResponse {
    proposal: ProposalRow,
    /// The new document version after the patch applied. Same shape
    /// as `documents::WriteResponse.version`.
    applied_version: String,
}

// ---------- handlers ----------

async fn list_proposals(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    require_admin(&tc)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let status_filter = match q.status.as_deref() {
        None | Some("pending") => Some("pending"),
        Some("approved") => Some("approved"),
        Some("rejected") => Some("rejected"),
        Some("all") => None,
        Some(other) => {
            return Err(ApiError::InvalidArgument(format!(
                "status must be one of pending|approved|rejected|all, got {other:?}"
            )))
        }
    };

    let rows: Vec<ProposalRow> = if let Some(s) = status_filter {
        sqlx::query_as(
            r#"
            SELECT id, doc_id, base_version, patch, reason, status,
                   actor_token_id, actor_token_label, actor_account_id,
                   decided_by, decided_at, decision_note, applied_version,
                   created_at
            FROM proposals
            WHERE tenant_id = $1 AND status = $2
            ORDER BY created_at DESC
            LIMIT $3
            "#,
        )
        .bind(tc.tenant_id)
        .bind(s)
        .bind(limit)
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query_as(
            r#"
            SELECT id, doc_id, base_version, patch, reason, status,
                   actor_token_id, actor_token_label, actor_account_id,
                   decided_by, decided_at, decision_note, applied_version,
                   created_at
            FROM proposals
            WHERE tenant_id = $1
            ORDER BY created_at DESC
            LIMIT $2
            "#,
        )
        .bind(tc.tenant_id)
        .bind(limit)
        .fetch_all(&state.db)
        .await?
    };
    Ok(Json(ListResponse { proposals: rows }))
}

async fn get_proposal(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
    Path((_tid, id)): Path<(Uuid, String)>,
) -> Result<Json<ProposalRow>, ApiError> {
    require_admin(&tc)?;
    let row: Option<ProposalRow> = sqlx::query_as(
        r#"
        SELECT id, doc_id, base_version, patch, reason, status,
               actor_token_id, actor_token_label, actor_account_id,
               decided_by, decided_at, decision_note, applied_version,
               created_at
        FROM proposals
        WHERE tenant_id = $1 AND id = $2
        "#,
    )
    .bind(tc.tenant_id)
    .bind(&id)
    .fetch_optional(&state.db)
    .await?;
    row.map(Json).ok_or(ApiError::NotFound)
}

async fn approve_proposal(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
    Extension(sc): Extension<SessionContext>,
    Path((_tid, id)): Path<(Uuid, String)>,
    Json(req): Json<DecideRequest>,
) -> Result<Json<ApproveResponse>, ApiError> {
    require_admin(&tc)?;

    let mut tx = state.db.begin().await?;

    // Lock the proposal. `FOR UPDATE` keeps two concurrent admins from
    // both seeing `pending` and both flipping it.
    let prop: Option<ProposalRow> = sqlx::query_as(
        r#"
        SELECT id, doc_id, base_version, patch, reason, status,
               actor_token_id, actor_token_label, actor_account_id,
               decided_by, decided_at, decision_note, applied_version,
               created_at
        FROM proposals
        WHERE tenant_id = $1 AND id = $2
        FOR UPDATE
        "#,
    )
    .bind(tc.tenant_id)
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(prop) = prop else {
        return Err(ApiError::NotFound);
    };
    if prop.status != "pending" {
        return Err(ApiError::Conflict("proposal already decided"));
    }

    // Lock the target document. Either the document moved on (base_version
    // mismatch) or it's gone — in either case, conflict.
    #[derive(FromRow)]
    struct DocRow {
        frontmatter: JsonValue,
        body: Option<String>,
        body_ciphertext: Option<String>,
        version: String,
    }
    let doc_row: Option<DocRow> = sqlx::query_as(
        r#"
        SELECT frontmatter, body, body_ciphertext, version
        FROM documents
        WHERE tenant_id = $1 AND doc_id = $2
        FOR UPDATE
        "#,
    )
    .bind(tc.tenant_id)
    .bind(&prop.doc_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(doc_row) = doc_row else {
        return Err(ApiError::Conflict("version_conflict"));
    };
    if doc_row.version != prop.base_version {
        return Err(ApiError::Conflict("version_conflict"));
    }

    // Resolve plaintext body. Mirrors `documents::resolve_body` but
    // we need it inline because that helper takes `&AppState` while
    // we're inside an open transaction.
    let body_plaintext = match (&doc_row.body, &doc_row.body_ciphertext) {
        (Some(plain), None) => plain.clone(),
        (None, Some(ct_wire)) => {
            let key = state
                .session_keys
                .get(tc.tenant_id, sc.session_id)
                .ok_or(ApiError::VaultLocked)?;
            let blob = orchext_crypto::SealedBlob::from_wire(ct_wire)
                .map_err(|e| ApiError::Internal(Box::new(e)))?;
            let plain = orchext_crypto::open(&blob, &key).map_err(|_| ApiError::VaultLocked)?;
            String::from_utf8(plain).map_err(|e| ApiError::Internal(Box::new(e)))?
        }
        _ => {
            return Err(ApiError::Internal(
                "documents row violates body xor body_ciphertext invariant".into(),
            ))
        }
    };

    // Apply the patch.
    let patch: PatchPayload = serde_json::from_value(prop.patch.clone())
        .map_err(|e| ApiError::Internal(Box::new(e)))?;
    let (new_frontmatter_json, new_body) =
        apply_patch(doc_row.frontmatter.clone(), body_plaintext, &patch)?;

    // Re-serialize via orchext_vault::Document so the version hash is
    // identical to what `documents::write_doc` would have produced for
    // the same logical document.
    let frontmatter: orchext_vault::Frontmatter =
        serde_json::from_value(new_frontmatter_json.clone())
            .map_err(|e| ApiError::InvalidArgument(format!("invalid frontmatter after patch: {e}")))?;
    let new_doc = Document {
        frontmatter: frontmatter.clone(),
        body: new_body.clone(),
    };
    let new_version = new_doc
        .version()
        .map_err(|e| ApiError::Internal(Box::new(e)))?;

    // Encrypt the new body if the source row was encrypted.
    let (stored_body, stored_ciphertext, stored_key_version): (
        Option<String>,
        Option<String>,
        Option<i32>,
    ) = if doc_row.body_ciphertext.is_some() {
        let key = state
            .session_keys
            .get(tc.tenant_id, sc.session_id)
            .ok_or(ApiError::VaultLocked)?;
        let sealed = orchext_crypto::seal(new_body.as_bytes(), &key)
            .map_err(|e| ApiError::Internal(Box::new(e)))?;
        (None, Some(sealed.to_wire()), Some(1))
    } else {
        (Some(new_body.clone()), None, None)
    };

    // Recompute title from the new body so the index stays in sync —
    // same fallback rule as `documents::write_doc`.
    let title = extract_title(&new_body, &prop.doc_id);
    let now = Utc::now();
    let new_visibility = frontmatter.visibility.as_label().to_string();
    let new_type = frontmatter.type_.clone();

    sqlx::query(
        r#"
        UPDATE documents
        SET type_           = $3,
            visibility      = $4,
            title           = $5,
            frontmatter     = $6,
            body            = $7,
            body_ciphertext = $8,
            key_version     = $9,
            version         = $10,
            updated_at      = $11
        WHERE tenant_id = $1 AND doc_id = $2
        "#,
    )
    .bind(tc.tenant_id)
    .bind(&prop.doc_id)
    .bind(&new_type)
    .bind(&new_visibility)
    .bind(&title)
    .bind(&new_frontmatter_json)
    .bind(&stored_body)
    .bind(&stored_ciphertext)
    .bind(stored_key_version)
    .bind(&new_version)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    // Tags / links fans live in their own tables; refresh them to match
    // the patched frontmatter so search and graph queries stay correct.
    replace_tags(&mut tx, tc.tenant_id, &prop.doc_id, &frontmatter.tags).await?;
    replace_links(&mut tx, tc.tenant_id, &prop.doc_id, &frontmatter.links).await?;

    // Mark proposal approved.
    sqlx::query(
        r#"
        UPDATE proposals
        SET status = 'approved',
            decided_by = $3,
            decided_at = $4,
            decision_note = $5,
            applied_version = $6
        WHERE tenant_id = $1 AND id = $2
        "#,
    )
    .bind(tc.tenant_id)
    .bind(&id)
    .bind(tc.account_id)
    .bind(now)
    .bind(req.note.as_deref())
    .bind(&new_version)
    .execute(&mut *tx)
    .await?;

    audit::append(
        &mut tx,
        tc.tenant_id,
        AppendRecord {
            actor: Actor::Account(tc.account_id),
            action: "proposal.approve".into(),
            document_id: Some(prop.doc_id.clone()),
            scope_used: Vec::new(),
            outcome: Outcome::Ok,
        },
    )
    .await?;

    tx.commit().await?;

    // Re-read so the response carries the post-decision row shape.
    let updated: ProposalRow = sqlx::query_as(
        r#"
        SELECT id, doc_id, base_version, patch, reason, status,
               actor_token_id, actor_token_label, actor_account_id,
               decided_by, decided_at, decision_note, applied_version,
               created_at
        FROM proposals
        WHERE tenant_id = $1 AND id = $2
        "#,
    )
    .bind(tc.tenant_id)
    .bind(&id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(ApproveResponse {
        proposal: updated,
        applied_version: new_version,
    }))
}

async fn reject_proposal(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
    Path((_tid, id)): Path<(Uuid, String)>,
    Json(req): Json<DecideRequest>,
) -> Result<Json<ProposalRow>, ApiError> {
    require_admin(&tc)?;

    let mut tx = state.db.begin().await?;

    let prop: Option<ProposalRow> = sqlx::query_as(
        r#"
        SELECT id, doc_id, base_version, patch, reason, status,
               actor_token_id, actor_token_label, actor_account_id,
               decided_by, decided_at, decision_note, applied_version,
               created_at
        FROM proposals
        WHERE tenant_id = $1 AND id = $2
        FOR UPDATE
        "#,
    )
    .bind(tc.tenant_id)
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(prop) = prop else {
        return Err(ApiError::NotFound);
    };
    if prop.status != "pending" {
        return Err(ApiError::Conflict("proposal already decided"));
    }

    let now = Utc::now();
    let updated: ProposalRow = sqlx::query_as(
        r#"
        UPDATE proposals
        SET status = 'rejected',
            decided_by = $3,
            decided_at = $4,
            decision_note = $5
        WHERE tenant_id = $1 AND id = $2
        RETURNING id, doc_id, base_version, patch, reason, status,
                  actor_token_id, actor_token_label, actor_account_id,
                  decided_by, decided_at, decision_note, applied_version,
                  created_at
        "#,
    )
    .bind(tc.tenant_id)
    .bind(&id)
    .bind(tc.account_id)
    .bind(now)
    .bind(req.note.as_deref())
    .fetch_one(&mut *tx)
    .await?;

    audit::append(
        &mut tx,
        tc.tenant_id,
        AppendRecord {
            actor: Actor::Account(tc.account_id),
            action: "proposal.reject".into(),
            document_id: Some(prop.doc_id),
            scope_used: Vec::new(),
            outcome: Outcome::Ok,
        },
    )
    .await?;

    tx.commit().await?;
    Ok(Json(updated))
}

// ---------- patch application ----------

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
struct PatchPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    frontmatter: Option<JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    body_replace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    body_append: Option<String>,
}

/// Apply a patch on top of the current frontmatter (JSONB form) and
/// body. Returns the new (frontmatter, body) pair. Frontmatter merge is
/// shallow per `MCP.md` §5.4 — top-level keys overwrite, with `null`
/// signalling "clear this key".
fn apply_patch(
    mut frontmatter: JsonValue,
    body: String,
    patch: &PatchPayload,
) -> Result<(JsonValue, String), ApiError> {
    if let Some(fm_patch) = &patch.frontmatter {
        let JsonValue::Object(fm_patch_map) = fm_patch else {
            return Err(ApiError::InvalidArgument(
                "patch.frontmatter must be an object".into(),
            ));
        };
        let JsonValue::Object(fm_map) = &mut frontmatter else {
            return Err(ApiError::Internal(
                "stored frontmatter is not an object".into(),
            ));
        };
        for (k, v) in fm_patch_map {
            if v.is_null() {
                fm_map.remove(k);
            } else {
                fm_map.insert(k.clone(), v.clone());
            }
        }
    }

    let new_body = if let Some(replacement) = &patch.body_replace {
        replacement.clone()
    } else if let Some(suffix) = &patch.body_append {
        let mut combined = body;
        combined.push_str(suffix);
        combined
    } else {
        body
    };

    Ok((frontmatter, new_body))
}

// ---------- shared helpers (with documents.rs) ----------

fn require_admin(tc: &TenantContext) -> Result<(), ApiError> {
    // Personal tenants always have a single owner so the gate is a
    // no-op there; team tenants reject member-role callers per D11.
    if tc.is_admin() {
        Ok(())
    } else {
        // Same shape `tenant_auth` uses for non-members — non-admins
        // get a 404 instead of a 403 so the queue's existence isn't
        // observable to roles that aren't supposed to see it.
        Err(ApiError::NotFound)
    }
}

fn extract_title(body: &str, fallback_id: &str) -> String {
    for line in body.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            let title = rest.trim();
            if !title.is_empty() {
                return title.to_string();
            }
        }
    }
    fallback_id.to_string()
}

async fn replace_tags(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    doc_id: &str,
    tags: &[String],
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM doc_tags WHERE tenant_id = $1 AND doc_id = $2")
        .bind(tenant_id)
        .bind(doc_id)
        .execute(&mut **tx)
        .await?;
    if tags.is_empty() {
        return Ok(());
    }
    let mut unique: Vec<&str> = tags.iter().map(String::as_str).collect();
    unique.sort();
    unique.dedup();
    sqlx::query(
        r#"
        INSERT INTO doc_tags (tenant_id, doc_id, tag)
        SELECT $1, $2, UNNEST($3::text[])
        "#,
    )
    .bind(tenant_id)
    .bind(doc_id)
    .bind(unique.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn replace_links(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    source: &str,
    links: &[String],
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM doc_links WHERE tenant_id = $1 AND source = $2")
        .bind(tenant_id)
        .bind(source)
        .execute(&mut **tx)
        .await?;
    if links.is_empty() {
        return Ok(());
    }
    let mut unique: Vec<&str> = links.iter().map(String::as_str).collect();
    unique.sort();
    unique.dedup();
    sqlx::query(
        r#"
        INSERT INTO doc_links (tenant_id, source, target)
        SELECT $1, $2, UNNEST($3::text[])
        "#,
    )
    .bind(tenant_id)
    .bind(source)
    .bind(unique.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn apply_patch_frontmatter_merges_top_level() {
        let fm = json!({"id": "a", "type": "preferences", "visibility": "work", "tags": ["x"]});
        let (out, body) = apply_patch(
            fm,
            "body".into(),
            &PatchPayload {
                frontmatter: Some(json!({"tags": ["x", "y"]})),
                body_replace: None,
                body_append: None,
            },
        )
        .unwrap();
        assert_eq!(out["tags"], json!(["x", "y"]));
        assert_eq!(out["id"], "a");
        assert_eq!(body, "body");
    }

    #[test]
    fn apply_patch_frontmatter_null_clears() {
        let fm = json!({"id": "a", "type": "x", "visibility": "work", "source": "old"});
        let (out, _) = apply_patch(
            fm,
            "".into(),
            &PatchPayload {
                frontmatter: Some(json!({"source": null})),
                body_replace: None,
                body_append: None,
            },
        )
        .unwrap();
        assert!(out.get("source").is_none());
    }

    #[test]
    fn apply_patch_body_append() {
        let (_, body) = apply_patch(
            json!({}),
            "first\n".into(),
            &PatchPayload {
                frontmatter: None,
                body_replace: None,
                body_append: Some("second\n".into()),
            },
        )
        .unwrap();
        assert_eq!(body, "first\nsecond\n");
    }

    #[test]
    fn apply_patch_body_replace_wins() {
        let (_, body) = apply_patch(
            json!({}),
            "old".into(),
            &PatchPayload {
                frontmatter: None,
                body_replace: Some("new".into()),
                body_append: None,
            },
        )
        .unwrap();
        assert_eq!(body, "new");
    }

    #[test]
    fn extract_title_basic() {
        assert_eq!(extract_title("# Hi\nbody", "id"), "Hi");
        assert_eq!(extract_title("nothing", "id"), "id");
    }
}
