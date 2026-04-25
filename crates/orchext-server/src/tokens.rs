//! Per-tenant MCP tokens: list, issue, revoke.
//!
//! Same token shape as `ourtex-auth::StoredToken` (opaque `otx_*` secret,
//! Argon2id-hashed at rest, scope as a list of visibility labels, mode =
//! `read` | `read_propose`, retrieval limits). The only difference is
//! that these tokens belong to a tenant and the issuing account, not to
//! a local vault file, so we store them in Postgres instead of the
//! JSON-backed `TokenService`.
//!
//! Issue returns the raw secret exactly once; subsequent `list` calls
//! return only the redacted `PublicTokenInfo`-shaped response. Revoke is
//! idempotent and permission-gated to the issuer (or any admin/owner).

use crate::{error::ApiError, password, tenants::TenantContext, AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete as router_delete, get},
    Extension, Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/tokens", get(list_tokens).post(issue_token))
        .route("/tokens/:token_id", router_delete(revoke_token))
}

const TOKEN_PREFIX: &str = "otx_";
const TOKEN_BYTES: usize = 32;
const PREFIX_LOOKUP_LEN: usize = TOKEN_PREFIX.len() + 8;
const DEFAULT_TTL_DAYS: i64 = 90;
const MAX_TTL_DAYS: i64 = 365;

#[derive(Debug, Serialize, FromRow)]
struct PublicToken {
    id: String,
    label: String,
    scope: Vec<String>,
    mode: String,
    max_docs: i32,
    max_bytes: i64,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct ListResponse {
    tokens: Vec<PublicToken>,
}

#[derive(Debug, Deserialize)]
struct IssueRequest {
    label: String,
    scope: Vec<String>,
    #[serde(default)]
    mode: Option<String>, // "read" | "read_propose"
    #[serde(default)]
    ttl_days: Option<i64>,
    #[serde(default)]
    max_docs: Option<i32>,
    #[serde(default)]
    max_bytes: Option<i64>,
}

#[derive(Debug, Serialize)]
struct IssueResponse {
    /// Returned exactly once. Persist immediately.
    secret: String,
    token: PublicToken,
}

async fn list_tokens(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
) -> Result<Json<ListResponse>, ApiError> {
    let rows: Vec<PublicToken> = sqlx::query_as(
        r#"
        SELECT id, label, scope, mode, max_docs, max_bytes,
               created_at, expires_at, last_used_at, revoked_at
        FROM mcp_tokens
        WHERE tenant_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(tc.tenant_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(ListResponse { tokens: rows }))
}

async fn issue_token(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
    Json(req): Json<IssueRequest>,
) -> Result<(StatusCode, Json<IssueResponse>), ApiError> {
    validate_label(&req.label)?;
    let scope = normalize_scope(req.scope)?;
    let mode = match req.mode.as_deref() {
        None | Some("read") => "read",
        Some("read_propose") => "read_propose",
        Some(other) => {
            return Err(ApiError::InvalidArgument(format!(
                "mode must be 'read' or 'read_propose', got {other:?}"
            )));
        }
    };
    let ttl_days = req
        .ttl_days
        .unwrap_or(DEFAULT_TTL_DAYS)
        .clamp(1, MAX_TTL_DAYS);
    let max_docs = req.max_docs.unwrap_or(20).max(1);
    let max_bytes = req.max_bytes.unwrap_or(64 * 1024).max(1024);

    let secret = generate_secret();
    let prefix = secret[..PREFIX_LOOKUP_LEN].to_string();
    let hash = password::hash(&secret).map_err(|e| ApiError::Internal(Box::new(e)))?;
    let id = generate_token_id();
    let expires_at = Utc::now() + Duration::days(ttl_days);

    let row: PublicToken = sqlx::query_as(
        r#"
        INSERT INTO mcp_tokens
            (id, tenant_id, issued_by, label, token_prefix, token_hash,
             scope, mode, max_docs, max_bytes, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        RETURNING id, label, scope, mode, max_docs, max_bytes,
                  created_at, expires_at, last_used_at, revoked_at
        "#,
    )
    .bind(&id)
    .bind(tc.tenant_id)
    .bind(tc.account_id)
    .bind(&req.label)
    .bind(&prefix)
    .bind(&hash)
    .bind(&scope)
    .bind(mode)
    .bind(max_docs)
    .bind(max_bytes)
    .bind(expires_at)
    .fetch_one(&state.db)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(IssueResponse {
            secret,
            token: row,
        }),
    ))
}

async fn revoke_token(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
    Path((_tid, token_id)): Path<(Uuid, String)>,
) -> Result<StatusCode, ApiError> {
    // A user can revoke their own token; admins can revoke any token
    // in the tenant. The `issued_by = $3 OR is_admin` guard below is
    // what enforces this — the revoke is silently a no-op for a token
    // the caller doesn't own and can't admin, returning 404 so an
    // unprivileged user can't use this endpoint to probe token ids.
    let affected = sqlx::query(
        r#"
        UPDATE mcp_tokens
        SET revoked_at = now()
        WHERE id = $1
          AND tenant_id = $2
          AND revoked_at IS NULL
          AND ($4 OR issued_by = $3)
        "#,
    )
    .bind(&token_id)
    .bind(tc.tenant_id)
    .bind(tc.account_id)
    .bind(tc.is_admin())
    .execute(&state.db)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

fn normalize_scope(raw: Vec<String>) -> Result<Vec<String>, ApiError> {
    if raw.is_empty() {
        return Err(ApiError::InvalidArgument("scope must not be empty".into()));
    }
    // Validate each label via ourtex_vault::Visibility. Keeps us honest
    // about what a scope label means: the literal string that must
    // match a document's `visibility`.
    for label in &raw {
        ourtex_vault::Visibility::from_label(label).map_err(|_| {
            ApiError::InvalidArgument(format!("invalid scope label {label:?}"))
        })?;
    }
    // De-dupe + sort for stable storage.
    let mut out: Vec<String> = raw;
    out.sort();
    out.dedup();
    Ok(out)
}

fn validate_label(s: &str) -> Result<(), ApiError> {
    if s.trim().is_empty() || s.len() > 200 {
        return Err(ApiError::InvalidArgument(
            "label must be 1..=200 chars".into(),
        ));
    }
    Ok(())
}

fn generate_secret() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("{TOKEN_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
}

fn generate_token_id() -> String {
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("tok_{}", URL_SAFE_NO_PAD.encode(bytes))
}
