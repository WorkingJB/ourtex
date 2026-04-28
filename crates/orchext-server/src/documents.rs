//! Vault document CRUD.
//!
//! The wire format is the canonical orchext-vault document source — a
//! YAML frontmatter block plus a markdown body — sent as a single
//! `source` string. That keeps the server's serialization identical to
//! what `orchext-vault` already parses/produces on disk, so the content
//! version hash (sha256 over the canonical form) matches bit-for-bit
//! whether computed by the local client or by the server.
//!
//! Writes run in a single transaction that (a) takes a row lock on the
//! existing doc for the base-version check, (b) upserts the doc, (c)
//! replaces its tag + link fans, (d) appends one audit entry. If any
//! step fails the whole transaction rolls back — including the audit
//! entry, which is the right outcome: a rolled-back mutation must not
//! leave a "it happened" trail.

use crate::{
    audit::{self, Actor, AppendRecord, Outcome},
    error::ApiError,
    sessions::SessionContext,
    tenants::TenantContext,
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
    Extension, Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use orchext_vault::Document;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/vault/docs", get(list_docs))
        .route(
            "/vault/docs/:doc_id",
            get(read_doc).put(write_doc).delete(delete_doc),
        )
        .route("/vault/doc-count", get(doc_count))
}

// ---------- DTOs ----------

#[derive(Debug, Serialize)]
pub struct ListEntry {
    pub doc_id: String,
    pub type_: String,
    pub visibility: String,
    pub title: String,
    pub updated: Option<NaiveDate>,
    pub tags: Vec<String>,
    /// Team binding for `visibility = 'team'` docs (Phase 3 platform
    /// Slice 2). `None` for org / personal / public / private rows.
    /// The DB CHECK constraint pins the strict coupling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct ListResponse {
    entries: Vec<ListEntry>,
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    #[serde(rename = "type")]
    type_: Option<String>,
    /// Restrict the listing to a single team's documents. Combined with
    /// the existing visibility filter — non-members of the requested
    /// team see an empty list, not a 404, so the endpoint doesn't leak
    /// "does this team exist" info.
    team_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct DocResponse {
    doc_id: String,
    type_: String,
    visibility: String,
    version: String,
    updated_at: DateTime<Utc>,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct WriteResponse {
    doc_id: String,
    type_: String,
    visibility: String,
    version: String,
    updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
struct WriteRequest {
    /// Canonical document source (frontmatter YAML + body markdown).
    source: String,
    /// If present, the write fails with `version_conflict` unless the
    /// current stored version equals this value. Omit to force-write
    /// without a precondition.
    #[serde(default)]
    base_version: Option<String>,
    /// Team this doc belongs to. Required when the parsed
    /// frontmatter's `visibility == 'team'`; rejected for any other
    /// visibility (the DB CHECK constraint also enforces the
    /// coupling). The team must belong to the org tenant the request
    /// targets.
    #[serde(default)]
    team_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
struct DeleteQuery {
    base_version: Option<String>,
}

#[derive(Debug, Serialize)]
struct DocCountResponse {
    count: i64,
}

// ---------- handlers ----------

async fn list_docs(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    #[derive(FromRow)]
    struct Row {
        doc_id: String,
        type_: String,
        visibility: String,
        title: String,
        updated: Option<NaiveDate>,
        team_id: Option<Uuid>,
    }

    // Visibility filter has two clauses:
    //   * private: only the author (or NULL legacy rows) may see it.
    //   * team: only org admins/owners or the team's members may see
    //     it. `$N::bool` short-circuits on admins so the membership
    //     subquery doesn't fire for them.
    //
    // Optional filters (`type_`, `team_id`) use `$N::T IS NULL OR …` so
    // a single query covers both "filter present" and "absent" without
    // branching. Postgres folds the constant null check at plan time.
    let is_org_admin = tc.is_admin();
    let rows: Vec<Row> = sqlx::query_as(
        r#"
        SELECT doc_id, type_, visibility, title,
               (frontmatter->>'updated')::date AS updated,
               team_id
        FROM documents
        WHERE tenant_id = $1
          AND ($2::text IS NULL OR type_ = $2)
          AND ($3::uuid IS NULL OR team_id = $3)
          AND (visibility != 'private'
               OR author_account_id = $4
               OR author_account_id IS NULL)
          AND (visibility != 'team'
               OR $5::bool
               OR team_id IN (
                   SELECT team_id FROM team_memberships WHERE account_id = $4
               ))
        ORDER BY updated_at DESC, doc_id ASC
        "#,
    )
    .bind(tc.tenant_id)
    .bind(q.type_.as_deref())
    .bind(q.team_id)
    .bind(tc.account_id)
    .bind(is_org_admin)
    .fetch_all(&state.db)
    .await?;

    let ids: Vec<String> = rows.iter().map(|r| r.doc_id.clone()).collect();
    let tags_by_doc = load_tags_for_docs(&state, tc.tenant_id, &ids).await?;

    let entries = rows
        .into_iter()
        .map(|r| {
            let tags = tags_by_doc.get(&r.doc_id).cloned().unwrap_or_default();
            ListEntry {
                doc_id: r.doc_id,
                type_: r.type_,
                visibility: r.visibility,
                title: r.title,
                updated: r.updated,
                tags,
                team_id: r.team_id,
            }
        })
        .collect();
    Ok(Json(ListResponse { entries }))
}

async fn read_doc(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
    Extension(sc): Extension<SessionContext>,
    Path((_tid, doc_id)): Path<(Uuid, String)>,
) -> Result<Json<DocResponse>, ApiError> {
    validate_doc_id(&doc_id)?;
    let is_org_admin = tc.is_admin();
    let row: Option<DocRow> = sqlx::query_as(
        r#"
        SELECT type_, visibility, frontmatter, body, body_ciphertext,
               version, updated_at, team_id
        FROM documents
        WHERE tenant_id = $1 AND doc_id = $2
          AND (visibility != 'private'
               OR author_account_id = $3
               OR author_account_id IS NULL)
          AND (visibility != 'team'
               OR $4::bool
               OR team_id IN (
                   SELECT team_id FROM team_memberships WHERE account_id = $3
               ))
        "#,
    )
    .bind(tc.tenant_id)
    .bind(&doc_id)
    .bind(tc.account_id)
    .bind(is_org_admin)
    .fetch_optional(&state.db)
    .await?;

    let Some(row) = row else {
        return Err(ApiError::NotFound);
    };

    let body_plaintext = resolve_body(&state, tc.tenant_id, sc.session_id, &row)?;
    let source = rebuild_source(&row.frontmatter, &body_plaintext)?;

    // Audit reads — denied cases never reach here (tenant guard covers
    // the not-a-member case, and private-floor enforcement is handled
    // by the caller's local scope evaluation, not by the server yet:
    // session users see everything their membership grants).
    let mut tx = state.db.begin().await?;
    audit::append(
        &mut tx,
        tc.tenant_id,
        AppendRecord {
            actor: Actor::Account(tc.account_id),
            action: "vault.read".into(),
            document_id: Some(doc_id.clone()),
            scope_used: Vec::new(),
            outcome: Outcome::Ok,
        },
    )
    .await?;
    tx.commit().await?;

    Ok(Json(DocResponse {
        doc_id,
        type_: row.type_,
        visibility: row.visibility,
        version: row.version,
        updated_at: row.updated_at,
        source,
        team_id: row.team_id,
    }))
}

async fn write_doc(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
    Extension(sc): Extension<SessionContext>,
    Path((_tid, doc_id)): Path<(Uuid, String)>,
    Json(req): Json<WriteRequest>,
) -> Result<Json<WriteResponse>, ApiError> {
    validate_doc_id(&doc_id)?;

    let doc = Document::parse(&req.source)
        .map_err(|e| ApiError::InvalidArgument(format!("invalid document source: {e}")))?;
    if doc.frontmatter.id.as_str() != doc_id {
        return Err(ApiError::InvalidArgument(format!(
            "frontmatter id {:?} does not match url doc id {:?}",
            doc.frontmatter.id.as_str(),
            doc_id
        )));
    }
    // Org-context write gate (D17g). If either the new doc or any
    // existing doc at this id is type=org, owner/admin/org_editor is
    // required. Members can `context.propose` against org docs but
    // not write them directly.
    if doc.frontmatter.type_ == "org" && !tc.can_write_org() {
        return Err(ApiError::Forbidden);
    }
    // Team binding gate. Strict coupling: visibility=team ⇔ team_id
    // is supplied. The DB CHECK constraint enforces the same
    // invariant; we validate here too so the error message is useful
    // (and so the membership/team-existence checks below have a
    // canonical team_id to work with).
    let new_visibility = doc.frontmatter.visibility.as_label().to_string();
    let team_id = req.team_id;
    if new_visibility == "team" && team_id.is_none() {
        return Err(ApiError::InvalidArgument(
            "team_id is required when visibility = 'team'".into(),
        ));
    }
    if new_visibility != "team" && team_id.is_some() {
        return Err(ApiError::InvalidArgument(
            "team_id is only allowed when visibility = 'team'".into(),
        ));
    }
    if let Some(tid) = team_id {
        // Team must belong to the org tenant the request targets, and
        // the caller must be either an org admin/owner or a manager
        // of this team. Plain team membership is enough to read team
        // docs but not to write them.
        let row: Option<(Uuid, Option<String>)> = sqlx::query_as(
            r#"
            SELECT t.id,
                   (SELECT tm.role FROM team_memberships tm
                    WHERE tm.team_id = t.id AND tm.account_id = $3) AS viewer_role
            FROM teams t
            JOIN organizations o ON o.id = t.org_id
            WHERE t.id = $1 AND o.tenant_id = $2
            "#,
        )
        .bind(tid)
        .bind(tc.tenant_id)
        .bind(tc.account_id)
        .fetch_optional(&state.db)
        .await?;
        let viewer_role = match row {
            Some((_id, role)) => role,
            None => {
                return Err(ApiError::InvalidArgument(
                    "team does not belong to this org".into(),
                ));
            }
        };
        let is_team_manager = viewer_role.as_deref() == Some("manager");
        if !tc.is_admin() && !is_team_manager {
            return Err(ApiError::Forbidden);
        }
    }
    let canonical = doc
        .serialize()
        .map_err(|e| ApiError::Internal(Box::new(e)))?;
    let new_version = doc
        .version()
        .map_err(|e| ApiError::Internal(Box::new(e)))?;

    // Split the canonical source for storage. We store the parsed
    // `frontmatter` as JSONB (for structured filters) and the raw
    // markdown `body`; the original YAML block is reconstructed on
    // read via `Document::serialize()`.
    let (_frontmatter_yaml, body) = split_canonical(&canonical)?;
    let frontmatter_json: JsonValue = serde_json::to_value(&doc.frontmatter)
        .map_err(|e| ApiError::Internal(Box::new(e)))?;
    let title = extract_title(&body, &doc_id);
    let type_ = doc.frontmatter.type_.clone();
    let visibility = new_visibility.clone();

    // One transaction spans: version check, upsert, tag/link replace,
    // audit append.
    let mut tx = state.db.begin().await?;

    let existing: Option<(String, String, String, Option<Uuid>, Option<Uuid>)> =
        sqlx::query_as(
            "SELECT version, type_, visibility, author_account_id, team_id \
             FROM documents \
             WHERE tenant_id = $1 AND doc_id = $2 FOR UPDATE",
        )
        .bind(tc.tenant_id)
        .bind(&doc_id)
        .fetch_optional(&mut *tx)
        .await?;

    // Catch the downgrade case: existing doc at this id is type=org,
    // and the writer doesn't have can_write_org. Even if the new doc
    // is type=task, this is overwriting an org doc.
    if let Some((_, existing_type, _, _, _)) = existing.as_ref() {
        if existing_type == "org" && !tc.can_write_org() {
            return Err(ApiError::Forbidden);
        }
    }
    // Visibility-private privacy: the existing doc belongs to a
    // different author and is private — pretend it doesn't exist.
    // This keeps the write surface aligned with the read surface
    // (both 404 the same way) so a non-author can't probe for the
    // existence of a private doc.
    if let Some((_, _, existing_visibility, existing_author, _)) = existing.as_ref() {
        if existing_visibility == "private"
            && existing_author.is_some()
            && existing_author != &Some(tc.account_id)
        {
            return Err(ApiError::NotFound);
        }
    }
    // Team-doc downgrade gate: if an existing row at this id is
    // already team-bound, the writer must be either an org admin/owner
    // or a manager of the *existing* team. This blocks a stranger
    // from overwriting a team doc by reposting it with a non-team
    // visibility. The membership check above already covered the new
    // team_id; this check covers the old one.
    if let Some((_, _, existing_visibility, _, existing_team_id)) = existing.as_ref() {
        if existing_visibility == "team" {
            if let Some(prev_team_id) = existing_team_id {
                let prev_role: Option<(String,)> = sqlx::query_as(
                    "SELECT role FROM team_memberships \
                     WHERE team_id = $1 AND account_id = $2",
                )
                .bind(prev_team_id)
                .bind(tc.account_id)
                .fetch_optional(&mut *tx)
                .await?;
                let is_prev_manager = matches!(
                    prev_role.as_ref().map(|(r,)| r.as_str()),
                    Some("manager")
                );
                if !tc.is_admin() && !is_prev_manager {
                    return Err(ApiError::Forbidden);
                }
            }
        }
    }

    if let Some(expected) = req.base_version.as_ref() {
        match &existing {
            Some((stored, _, _, _, _)) if stored != expected => {
                return Err(ApiError::Conflict("version_conflict"));
            }
            None => {
                // Base version was provided but the doc doesn't exist
                // yet — treat as conflict since the caller clearly
                // thinks there's a version to compete with.
                return Err(ApiError::Conflict("version_conflict"));
            }
            _ => {}
        }
    }

    // Decide the storage mode: if this tenant has seeded crypto, encrypt
    // the body server-side using the currently-live content key. If
    // crypto is seeded but no key is live, the write can't proceed —
    // 423 Locked. If crypto isn't seeded at all, store plaintext (2b.2
    // behaviour).
    let seeded: (bool,) = sqlx::query_as(
        "SELECT (kdf_salt IS NOT NULL) FROM tenants WHERE id = $1",
    )
    .bind(tc.tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    let (stored_body, stored_ciphertext, stored_key_version): (Option<String>, Option<String>, Option<i32>) =
        if seeded.0 {
            let key = state
                .session_keys
                .get(tc.tenant_id, sc.session_id)
                .ok_or(ApiError::VaultLocked)?;
            let sealed = orchext_crypto::seal(body.as_bytes(), &key)
                .map_err(|e| ApiError::Internal(Box::new(e)))?;
            (None, Some(sealed.to_wire()), Some(1))
        } else {
            (Some(body.clone()), None, None)
        };

    let now = Utc::now();
    // author_account_id is set on first INSERT and **preserved** on
    // UPDATE — the original author keeps ownership for the purposes
    // of visibility=private filtering, even if a co-owner with
    // can_write_org edits later. (The downgrade path above already
    // refuses non-author writes to private docs, so this UPDATE
    // branch only fires when the writer either is the author or the
    // doc isn't private.)
    sqlx::query(
        r#"
        INSERT INTO documents
            (tenant_id, doc_id, type_, visibility, title,
             frontmatter, body, body_ciphertext, key_version,
             version, author_account_id, team_id, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $13)
        ON CONFLICT (tenant_id, doc_id) DO UPDATE SET
            type_           = EXCLUDED.type_,
            visibility      = EXCLUDED.visibility,
            title           = EXCLUDED.title,
            frontmatter     = EXCLUDED.frontmatter,
            body            = EXCLUDED.body,
            body_ciphertext = EXCLUDED.body_ciphertext,
            key_version     = EXCLUDED.key_version,
            version         = EXCLUDED.version,
            team_id         = EXCLUDED.team_id,
            updated_at      = EXCLUDED.updated_at
        "#,
    )
    .bind(tc.tenant_id)
    .bind(&doc_id)
    .bind(&type_)
    .bind(&visibility)
    .bind(&title)
    .bind(&frontmatter_json)
    .bind(&stored_body)
    .bind(&stored_ciphertext)
    .bind(stored_key_version)
    .bind(&new_version)
    .bind(tc.account_id)
    .bind(team_id)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    replace_tags(&mut tx, tc.tenant_id, &doc_id, &doc.frontmatter.tags).await?;
    replace_links(&mut tx, tc.tenant_id, &doc_id, &doc.frontmatter.links).await?;

    audit::append(
        &mut tx,
        tc.tenant_id,
        AppendRecord {
            actor: Actor::Account(tc.account_id),
            action: "vault.write".into(),
            document_id: Some(doc_id.clone()),
            scope_used: Vec::new(),
            outcome: Outcome::Ok,
        },
    )
    .await?;

    tx.commit().await?;

    Ok(Json(WriteResponse {
        doc_id,
        type_,
        visibility,
        version: new_version,
        updated_at: now,
        team_id,
    }))
}

async fn delete_doc(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
    Path((_tid, doc_id)): Path<(Uuid, String)>,
    Query(q): Query<DeleteQuery>,
) -> Result<StatusCode, ApiError> {
    validate_doc_id(&doc_id)?;

    let mut tx = state.db.begin().await?;

    let existing: Option<(String, String, String, Option<Uuid>, Option<Uuid>)> =
        sqlx::query_as(
            "SELECT version, type_, visibility, author_account_id, team_id \
             FROM documents \
             WHERE tenant_id = $1 AND doc_id = $2 FOR UPDATE",
        )
        .bind(tc.tenant_id)
        .bind(&doc_id)
        .fetch_optional(&mut *tx)
        .await?;

    let Some((version, existing_type, existing_visibility, existing_author, existing_team_id)) =
        existing
    else {
        return Err(ApiError::NotFound);
    };
    // Visibility-private privacy: a non-author can't even know the
    // doc exists. Same 404 the read path returns.
    if existing_visibility == "private"
        && existing_author.is_some()
        && existing_author != Some(tc.account_id)
    {
        return Err(ApiError::NotFound);
    }
    // Org-context delete gate (D17g). Members can't delete an org doc.
    if existing_type == "org" && !tc.can_write_org() {
        return Err(ApiError::Forbidden);
    }
    // Team-context delete gate. Mirrors the write gate above:
    // org admin/owner OR a manager of the existing doc's team. Plain
    // team membership is enough to read but not delete.
    if existing_visibility == "team" {
        if let Some(prev_team_id) = existing_team_id {
            let prev_role: Option<(String,)> = sqlx::query_as(
                "SELECT role FROM team_memberships \
                 WHERE team_id = $1 AND account_id = $2",
            )
            .bind(prev_team_id)
            .bind(tc.account_id)
            .fetch_optional(&mut *tx)
            .await?;
            let is_prev_manager = matches!(
                prev_role.as_ref().map(|(r,)| r.as_str()),
                Some("manager")
            );
            if !tc.is_admin() && !is_prev_manager {
                return Err(ApiError::Forbidden);
            }
        }
    }
    if let Some(expected) = q.base_version.as_ref() {
        if *expected != version {
            return Err(ApiError::Conflict("version_conflict"));
        }
    }

    sqlx::query("DELETE FROM documents WHERE tenant_id = $1 AND doc_id = $2")
        .bind(tc.tenant_id)
        .bind(&doc_id)
        .execute(&mut *tx)
        .await?;

    audit::append(
        &mut tx,
        tc.tenant_id,
        AppendRecord {
            actor: Actor::Account(tc.account_id),
            action: "vault.delete".into(),
            document_id: Some(doc_id.clone()),
            scope_used: Vec::new(),
            outcome: Outcome::Ok,
        },
    )
    .await?;

    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn doc_count(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
) -> Result<Json<DocCountResponse>, ApiError> {
    let is_org_admin = tc.is_admin();
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM documents \
         WHERE tenant_id = $1 \
           AND (visibility != 'private' \
                OR author_account_id = $2 \
                OR author_account_id IS NULL) \
           AND (visibility != 'team' \
                OR $3::bool \
                OR team_id IN ( \
                    SELECT team_id FROM team_memberships WHERE account_id = $2 \
                ))",
    )
    .bind(tc.tenant_id)
    .bind(tc.account_id)
    .bind(is_org_admin)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(DocCountResponse { count }))
}

// ---------- helpers ----------

#[derive(FromRow)]
struct DocRow {
    type_: String,
    visibility: String,
    frontmatter: JsonValue,
    /// Plaintext body for pre-crypto (2b.2) rows and unencrypted
    /// tenants. `None` when the row stores ciphertext.
    body: Option<String>,
    /// SealedBlob wire-form. `None` for plaintext rows.
    body_ciphertext: Option<String>,
    version: String,
    updated_at: DateTime<Utc>,
    team_id: Option<Uuid>,
}

/// Pick the plaintext body for a read: either the stored `body` (for
/// unencrypted rows) or the decrypted `body_ciphertext` using the
/// calling session's live key. `vault_locked` when the row is
/// encrypted but the calling session has no key published.
fn resolve_body(
    state: &AppState,
    tenant_id: Uuid,
    session_id: Uuid,
    row: &DocRow,
) -> Result<String, ApiError> {
    match (&row.body, &row.body_ciphertext) {
        (Some(plain), None) => Ok(plain.clone()),
        (None, Some(ct_wire)) => {
            let key = state
                .session_keys
                .get(tenant_id, session_id)
                .ok_or(ApiError::VaultLocked)?;
            let blob = orchext_crypto::SealedBlob::from_wire(ct_wire)
                .map_err(|e| ApiError::Internal(Box::new(e)))?;
            let plain = orchext_crypto::open(&blob, &key).map_err(|_| {
                // A decryption failure here means either the live key
                // doesn't match this row's `key_version` or the
                // ciphertext is corrupt. The caller's only remedy is
                // republishing the right key, so surface as locked.
                ApiError::VaultLocked
            })?;
            String::from_utf8(plain).map_err(|e| ApiError::Internal(Box::new(e)))
        }
        // CHECK constraint pins exactly one side — these branches
        // are unreachable in a valid row.
        _ => Err(ApiError::Internal(
            "documents row violates body xor body_ciphertext invariant".into(),
        )),
    }
}

async fn load_tags_for_docs(
    state: &AppState,
    tenant_id: Uuid,
    doc_ids: &[String],
) -> Result<std::collections::HashMap<String, Vec<String>>, ApiError> {
    use std::collections::HashMap;
    if doc_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(String, String)> = sqlx::query_as(
        r#"
        SELECT doc_id, tag
        FROM doc_tags
        WHERE tenant_id = $1 AND doc_id = ANY($2)
        ORDER BY doc_id, tag
        "#,
    )
    .bind(tenant_id)
    .bind(doc_ids)
    .fetch_all(&state.db)
    .await?;
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for (doc_id, tag) in rows {
        map.entry(doc_id).or_default().push(tag);
    }
    Ok(map)
}

async fn replace_tags(
    tx: &mut Transaction<'_, Postgres>,
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
    // De-dupe client-side in case the caller sent repeats — the PK
    // would otherwise reject the batch.
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
    tx: &mut Transaction<'_, Postgres>,
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

fn validate_doc_id(s: &str) -> Result<(), ApiError> {
    // Keep in lockstep with `orchext_vault::DocumentId::is_valid`. We
    // parse-and-throw to avoid a dependency on that private helper.
    orchext_vault::DocumentId::new(s)
        .map_err(|_| ApiError::InvalidArgument(format!("invalid doc id {s:?}")))?;
    Ok(())
}

/// Splits a canonical document `---\n<yaml>---\n<body>` back into its
/// two halves. Assumes the input came from `Document::serialize()` so
/// the shape is guaranteed.
fn split_canonical(source: &str) -> Result<(String, String), ApiError> {
    let after_open = source
        .strip_prefix("---\n")
        .ok_or_else(|| ApiError::Internal("canonical source missing open fence".into()))?;
    let end = after_open
        .find("\n---\n")
        .ok_or_else(|| ApiError::Internal("canonical source missing close fence".into()))?;
    let yaml = &after_open[..end];
    let body_start = end + "\n---\n".len();
    let body = &after_open[body_start..];
    Ok((yaml.to_string(), body.to_string()))
}

fn rebuild_source(frontmatter_json: &JsonValue, body: &str) -> Result<String, ApiError> {
    // Reconstruct the Document from the stored JSONB frontmatter, then
    // serialize to canonical form. Going through orchext_vault guarantees
    // the output matches the wire format produced on write.
    let frontmatter: orchext_vault::Frontmatter =
        serde_json::from_value(frontmatter_json.clone())
            .map_err(|e| ApiError::Internal(Box::new(e)))?;
    let doc = Document {
        frontmatter,
        body: body.to_string(),
    };
    doc.serialize().map_err(|e| ApiError::Internal(Box::new(e)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_canonical_round_trip() {
        let src = "---\nid: a\ntype: preferences\nvisibility: work\n---\n# Hello\nbody\n";
        let (y, b) = split_canonical(src).unwrap();
        // The `\n` before `---` is consumed by the separator, so the
        // yaml half has no trailing newline; callers that want one
        // back add it when rejoining.
        assert_eq!(y, "id: a\ntype: preferences\nvisibility: work");
        assert_eq!(b, "# Hello\nbody\n");
    }

    #[test]
    fn extract_title_basic() {
        assert_eq!(extract_title("# Hello\nbody\n", "fallback"), "Hello");
        assert_eq!(extract_title("no heading\n", "fallback"), "fallback");
        assert_eq!(extract_title("   # Indented\n", "x"), "Indented");
    }
}
