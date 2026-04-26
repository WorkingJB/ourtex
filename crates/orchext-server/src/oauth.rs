//! OAuth 2.1 authorization-code grant with PKCE — agent token issuance.
//!
//! Two endpoints:
//! - `POST /v1/oauth/authorize` (session-authed) — a logged-in user
//!   approves an agent's request for a tenant-scoped token. Returns the
//!   one-time `code` for the client to redeem at `/token`. We don't
//!   render a consent UI here; the desktop / web client is the consent
//!   surface and posts JSON when the user clicks "approve."
//! - `POST /v1/oauth/token` (no session auth) — exchanges
//!   (`code`, `code_verifier`, `redirect_uri`) for an opaque `ocx_*`
//!   bearer token row in `mcp_tokens`. PKCE is mandatory and only S256
//!   is accepted (OAuth 2.1 §7.5.2 — `plain` is forbidden).
//!
//! D15 (opaque tokens) and D16 (rolled, no library) carry through
//! unchanged. The token returned here is a normal `mcp_tokens` row, so
//! `revoke_token`, `list_tokens`, scope evaluation, etc. all work
//! without further changes.

use crate::{
    error::ApiError, password, sessions::SessionContext, tokens, AppState,
};
use axum::{
    extract::State,
    http::StatusCode,
    routing::post,
    Extension, Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;

/// Authorization codes carry an `oac_` prefix to make them visually
/// distinct from `ocx_*` session/token secrets in logs and error reports.
const CODE_PREFIX: &str = "oac_";
const CODE_BYTES: usize = 32;
const PREFIX_LOOKUP_LEN: usize = CODE_PREFIX.len() + 8;
const CODE_TTL_SECS: i64 = 600; // 10 min — matches OAuth 2.1 guidance.
const VERIFIER_MIN_LEN: usize = 43; // RFC 7636 §4.1
const VERIFIER_MAX_LEN: usize = 128;

/// Public router. The `/authorize` route is session-authed by the caller
/// (`router(state)` mounts both routes; `/token` skips session auth
/// because the agent client doesn't have a user session — only the
/// auth code).
pub fn router() -> Router<AppState> {
    // Both routes are POST. The `/authorize` route is wrapped with
    // session auth at the lib.rs `nest` site; `/token` is intentionally
    // unauthenticated (the auth code itself is the credential).
    Router::new().route("/token", post(token_handler))
}

pub fn authorize_router() -> Router<AppState> {
    Router::new().route("/authorize", post(authorize_handler))
}

// ---------- /authorize ----------

#[derive(Debug, Deserialize)]
struct AuthorizeRequest {
    /// Audience: the tenant the agent will operate against. Caller must
    /// be a member.
    tenant_id: Uuid,
    /// Free-form display name shown back in `mcp_tokens.label`. The
    /// client picks it; the user sees it in the token list.
    client_label: String,
    /// Where the issued auth code is delivered. Must be one of:
    /// - `http://127.0.0.1:<port>/...` or `http://localhost:<port>/...`
    ///   (loopback — desktop apps that bind a temporary listener)
    /// - `https://<host>/...` (web SPAs registered server-side later)
    /// Anything else is rejected — OAuth 2.1 §3.1.2 forbids non-HTTPS
    /// redirect URIs except for loopback.
    redirect_uri: String,
    /// Scope labels (visibility names — `public`, `work`, `personal`,
    /// or any custom label that round-trips through `Visibility`).
    scope: Vec<String>,
    /// `read` or `read_propose`. Defaults to `read`.
    #[serde(default)]
    mode: Option<String>,
    /// Token TTL in days. Defaults to 90; clamped to [1, 365] like
    /// other token issuance paths.
    #[serde(default)]
    ttl_days: Option<i64>,
    /// Per-token retrieval limit (documents). Defaults to 20.
    #[serde(default)]
    max_docs: Option<i32>,
    /// Per-token retrieval limit (bytes). Defaults to 65 536.
    #[serde(default)]
    max_bytes: Option<i64>,
    /// Base64url-encoded SHA-256 hash of the client's code verifier.
    /// Length is invariant: SHA-256 is 32 bytes → 43 chars unpadded.
    code_challenge: String,
    /// Must be `S256`. `plain` is rejected per OAuth 2.1.
    code_challenge_method: String,
}

#[derive(Debug, Serialize)]
struct AuthorizeResponse {
    /// The one-time auth code. Client posts this back to `/token`
    /// alongside the verifier. Single-use; expires in 10 minutes.
    code: String,
    /// Echoed for the client's convenience — same as the request's
    /// `redirect_uri`. Helps clients that round-trip through a browser
    /// and want to reconstruct the callback URL.
    redirect_uri: String,
    /// Seconds until `code` expires.
    expires_in: i64,
}

async fn authorize_handler(
    State(state): State<AppState>,
    Extension(session): Extension<SessionContext>,
    Json(req): Json<AuthorizeRequest>,
) -> Result<(StatusCode, Json<AuthorizeResponse>), ApiError> {
    if req.code_challenge_method != "S256" {
        return Err(ApiError::InvalidArgument(
            "code_challenge_method must be S256".into(),
        ));
    }
    validate_code_challenge(&req.code_challenge)?;
    validate_redirect_uri(&req.redirect_uri)?;
    tokens::validate_label(&req.client_label)?;
    let scope = tokens::normalize_scope(req.scope)?;
    let mode = tokens::normalize_mode(req.mode.as_deref())?;
    let ttl_days = tokens::clamp_ttl_days(req.ttl_days);
    let max_docs = req.max_docs.unwrap_or(20).max(1);
    let max_bytes = req.max_bytes.unwrap_or(64 * 1024).max(1024);

    // Confirm the caller is a member of the requested tenant. Same
    // not-found-on-mismatch shape as `tenant_auth` so we don't leak
    // tenant existence to non-members.
    let member: Option<(String,)> = sqlx::query_as(
        "SELECT role FROM memberships WHERE tenant_id = $1 AND account_id = $2",
    )
    .bind(req.tenant_id)
    .bind(session.account_id)
    .fetch_optional(&state.db)
    .await?;
    if member.is_none() {
        return Err(ApiError::NotFound);
    }

    let code = generate_code();
    let prefix = code[..PREFIX_LOOKUP_LEN].to_string();
    let hash = password::hash(&code).map_err(|e| ApiError::Internal(Box::new(e)))?;
    let expires_at = Utc::now() + Duration::seconds(CODE_TTL_SECS);

    sqlx::query(
        r#"
        INSERT INTO oauth_authorization_codes
            (code_prefix, code_hash, account_id, tenant_id, client_label,
             redirect_uri, scope, mode, max_docs, max_bytes, ttl_days,
             code_challenge, code_challenge_method, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
        "#,
    )
    .bind(&prefix)
    .bind(&hash)
    .bind(session.account_id)
    .bind(req.tenant_id)
    .bind(&req.client_label)
    .bind(&req.redirect_uri)
    .bind(&scope)
    .bind(mode)
    .bind(max_docs)
    .bind(max_bytes)
    .bind(ttl_days as i32)
    .bind(&req.code_challenge)
    .bind(&req.code_challenge_method)
    .bind(expires_at)
    .execute(&state.db)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(AuthorizeResponse {
            code,
            redirect_uri: req.redirect_uri,
            expires_in: CODE_TTL_SECS,
        }),
    ))
}

// ---------- /token ----------

#[derive(Debug, Deserialize)]
struct TokenRequest {
    /// OAuth 2.1 §4.1.3. Must be `authorization_code`.
    grant_type: String,
    code: String,
    code_verifier: String,
    /// Must match the `redirect_uri` posted to `/authorize`.
    redirect_uri: String,
}

#[derive(Debug, Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str, // always "Bearer"
    expires_in: i64,          // seconds until token expires
    scope: String,            // space-separated, OAuth-idiomatic
    /// Tenant the token operates against — non-standard but useful for
    /// agent clients that want to skip a `/v1/tenants` round trip.
    tenant_id: Uuid,
    /// Internal mcp_tokens.id, useful for revocation by the issuing user.
    token_id: String,
}

#[derive(Debug, FromRow)]
struct CodeRow {
    code_hash: String,
    account_id: Uuid,
    tenant_id: Uuid,
    client_label: String,
    redirect_uri: String,
    scope: Vec<String>,
    mode: String,
    max_docs: i32,
    max_bytes: i64,
    ttl_days: i32,
    code_challenge: String,
    expires_at: DateTime<Utc>,
    used_at: Option<DateTime<Utc>>,
}

async fn token_handler(
    State(state): State<AppState>,
    Json(req): Json<TokenRequest>,
) -> Result<Json<TokenResponse>, ApiError> {
    if req.grant_type != "authorization_code" {
        return Err(ApiError::InvalidArgument(
            "grant_type must be authorization_code".into(),
        ));
    }
    if req.code.len() < PREFIX_LOOKUP_LEN || !req.code.starts_with(CODE_PREFIX) {
        return Err(ApiError::Unauthorized);
    }
    validate_verifier(&req.code_verifier)?;

    let prefix = &req.code[..PREFIX_LOOKUP_LEN];

    // Single-statement claim: select the row by prefix, verify the full
    // hash, then UPDATE used_at in a separate transaction-bound step
    // below. The UPDATE itself enforces single-use via `used_at IS NULL`.
    let row: Option<CodeRow> = sqlx::query_as(
        r#"
        SELECT code_hash, account_id, tenant_id, client_label, redirect_uri,
               scope, mode, max_docs, max_bytes, ttl_days, code_challenge,
               expires_at, used_at
        FROM oauth_authorization_codes
        WHERE code_prefix = $1
        "#,
    )
    .bind(prefix)
    .fetch_optional(&state.db)
    .await?;

    let Some(row) = row else {
        return Err(ApiError::Unauthorized);
    };
    if row.used_at.is_some() {
        return Err(ApiError::Unauthorized);
    }
    if row.expires_at <= Utc::now() {
        return Err(ApiError::Unauthorized);
    }

    let secret_ok = password::verify(&req.code, &row.code_hash)
        .map_err(|e| ApiError::Internal(Box::new(e)))?;
    if !secret_ok {
        return Err(ApiError::Unauthorized);
    }

    if !pkce_matches(&req.code_verifier, &row.code_challenge) {
        return Err(ApiError::Unauthorized);
    }

    if !redirect_uri_matches(&req.redirect_uri, &row.redirect_uri) {
        return Err(ApiError::Unauthorized);
    }

    // Atomically mark the code used. Race a parallel redemption: only
    // one UPDATE will see `used_at IS NULL` and affect a row.
    let claimed = sqlx::query(
        "UPDATE oauth_authorization_codes
         SET used_at = now()
         WHERE code_prefix = $1 AND used_at IS NULL",
    )
    .bind(prefix)
    .execute(&state.db)
    .await?
    .rows_affected();
    if claimed == 0 {
        return Err(ApiError::Unauthorized);
    }

    // Issue the token via the same path as the admin tokens endpoint.
    let issued = tokens::issue_via_oauth(
        &state.db,
        tokens::OAuthIssueInput {
            tenant_id: row.tenant_id,
            issued_by: row.account_id,
            label: row.client_label,
            scope: row.scope.clone(),
            mode: row.mode.clone(),
            max_docs: row.max_docs,
            max_bytes: row.max_bytes,
            ttl_days: row.ttl_days as i64,
        },
    )
    .await?;

    Ok(Json(TokenResponse {
        access_token: issued.secret,
        token_type: "Bearer",
        expires_in: (issued.expires_at - Utc::now()).num_seconds().max(0),
        scope: row.scope.join(" "),
        tenant_id: row.tenant_id,
        token_id: issued.id,
    }))
}

// ---------- helpers ----------

fn generate_code() -> String {
    let mut bytes = [0u8; CODE_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("{CODE_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
}

fn validate_code_challenge(s: &str) -> Result<(), ApiError> {
    // SHA-256 → 32 bytes → base64url-no-pad → exactly 43 chars.
    if s.len() != 43 {
        return Err(ApiError::InvalidArgument(
            "code_challenge must be base64url(SHA256(verifier)) — 43 chars".into(),
        ));
    }
    if !s.chars().all(is_base64url_char) {
        return Err(ApiError::InvalidArgument(
            "code_challenge contains non-base64url chars".into(),
        ));
    }
    Ok(())
}

fn validate_verifier(s: &str) -> Result<(), ApiError> {
    if s.len() < VERIFIER_MIN_LEN || s.len() > VERIFIER_MAX_LEN {
        return Err(ApiError::InvalidArgument(format!(
            "code_verifier length must be {VERIFIER_MIN_LEN}..={VERIFIER_MAX_LEN}",
        )));
    }
    // RFC 7636 §4.1: ALPHA / DIGIT / - . _ ~
    let ok = s.bytes().all(|b| {
        b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
    });
    if !ok {
        return Err(ApiError::InvalidArgument(
            "code_verifier contains disallowed characters".into(),
        ));
    }
    Ok(())
}

fn is_base64url_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// SHA-256 the verifier and base64url-encode the result. Compare in
/// constant time against the stored challenge.
fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    let mut h = Sha256::new();
    h.update(verifier.as_bytes());
    let computed = URL_SAFE_NO_PAD.encode(h.finalize());
    use subtle::ConstantTimeEq;
    computed.as_bytes().ct_eq(challenge.as_bytes()).into()
}

/// Exact-match the redirect URI presented at /token against the one
/// stored at /authorize. OAuth 2.1 §3.1.2.3: byte-exact match required.
fn redirect_uri_matches(presented: &str, stored: &str) -> bool {
    use subtle::ConstantTimeEq;
    presented.as_bytes().ct_eq(stored.as_bytes()).into()
}

/// OAuth 2.1 §3.1.2.1 / §10.3.3: redirect URIs must be HTTPS, with
/// loopback HTTP allowed for native apps that bind a local listener.
fn validate_redirect_uri(uri: &str) -> Result<(), ApiError> {
    let lower = uri.to_ascii_lowercase();
    if lower.starts_with("https://") {
        return Ok(());
    }
    // Loopback variants (RFC 8252 §7.3) — match scheme + host + ':'.
    // We don't parse the URL fully; just check the prefix because any
    // other scheme/host combination is rejected.
    if lower.starts_with("http://127.0.0.1:")
        || lower.starts_with("http://127.0.0.1/")
        || lower == "http://127.0.0.1"
        || lower.starts_with("http://localhost:")
        || lower.starts_with("http://localhost/")
        || lower == "http://localhost"
        || lower.starts_with("http://[::1]:")
        || lower.starts_with("http://[::1]/")
        || lower == "http://[::1]"
    {
        return Ok(());
    }
    Err(ApiError::InvalidArgument(
        "redirect_uri must be https or a loopback http URL".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_round_trip() {
        let verifier = "abcDEF123-._~xyzABCDEFGHIJKLMNOPQRSTUVWXYZ012345";
        let mut h = Sha256::new();
        h.update(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(h.finalize());
        assert!(pkce_matches(verifier, &challenge));
    }

    #[test]
    fn pkce_wrong_verifier_rejected() {
        let verifier = "abcDEF123-._~xyzABCDEFGHIJKLMNOPQRSTUVWXYZ012345";
        let other = "xyzDEF123-._~abcABCDEFGHIJKLMNOPQRSTUVWXYZ012345";
        let mut h = Sha256::new();
        h.update(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(h.finalize());
        assert!(!pkce_matches(other, &challenge));
    }

    #[test]
    fn validate_redirect_uri_https() {
        assert!(validate_redirect_uri("https://example.com/cb").is_ok());
        assert!(validate_redirect_uri("HTTPS://EXAMPLE.COM/cb").is_ok());
    }

    #[test]
    fn validate_redirect_uri_loopback() {
        assert!(validate_redirect_uri("http://127.0.0.1:5555/cb").is_ok());
        assert!(validate_redirect_uri("http://localhost:8080/cb").is_ok());
        assert!(validate_redirect_uri("http://[::1]:9000/cb").is_ok());
    }

    #[test]
    fn validate_redirect_uri_rejects_non_loopback_http() {
        assert!(validate_redirect_uri("http://example.com/cb").is_err());
        assert!(validate_redirect_uri("http://192.168.1.1/cb").is_err());
        assert!(validate_redirect_uri("ftp://example.com/cb").is_err());
        assert!(validate_redirect_uri("javascript:alert(1)").is_err());
    }

    #[test]
    fn challenge_length_pinned() {
        let valid = "a".repeat(43);
        assert!(validate_code_challenge(&valid).is_ok());
        let too_short = "a".repeat(42);
        assert!(validate_code_challenge(&too_short).is_err());
        let too_long = "a".repeat(44);
        assert!(validate_code_challenge(&too_long).is_err());
    }

    #[test]
    fn verifier_length_bounds() {
        let min_ok = "a".repeat(VERIFIER_MIN_LEN);
        assert!(validate_verifier(&min_ok).is_ok());
        let max_ok = "a".repeat(VERIFIER_MAX_LEN);
        assert!(validate_verifier(&max_ok).is_ok());
        let too_short = "a".repeat(VERIFIER_MIN_LEN - 1);
        assert!(validate_verifier(&too_short).is_err());
        let too_long = "a".repeat(VERIFIER_MAX_LEN + 1);
        assert!(validate_verifier(&too_long).is_err());
    }

    #[test]
    fn verifier_rejects_disallowed_chars() {
        let bad = "abc/DEF=GHI+JKL".repeat(4);
        assert!(validate_verifier(&bad).is_err());
    }
}
