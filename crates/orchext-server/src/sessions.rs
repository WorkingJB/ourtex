//! Session tokens: opaque `otx_*` secrets, Argon2id-hashed at rest.
//!
//! Matches the `ourtex-auth` token shape (D15). The raw secret is
//! returned to the caller exactly once at issuance; subsequent
//! validation looks up by `token_prefix` and verifies the full secret
//! against the stored Argon2id hash.
//!
//! An in-memory cache (60s TTL) sits in front of the DB lookup. A
//! revocation is therefore live up to `CACHE_TTL` after the revoke row
//! write — acceptable at our scale and much cheaper than hitting
//! Argon2 on every request.

use crate::{error::ApiError, password};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use sqlx::{FromRow, PgPool};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration as StdDuration, Instant},
};
use uuid::Uuid;

const TOKEN_PREFIX: &str = "otx_";
const TOKEN_BYTES: usize = 32;
const DEFAULT_TTL_DAYS: i64 = 30;
const CACHE_TTL: StdDuration = StdDuration::from_secs(60);
const PREFIX_LOOKUP_LEN: usize = TOKEN_PREFIX.len() + 8; // "otx_" + 8 chars

/// Returned once to the caller. The `secret` field is the only moment
/// the raw token is visible in the clear; persist it carefully.
#[derive(Debug)]
pub struct IssuedSession {
    pub id: Uuid,
    pub account_id: Uuid,
    pub secret: String,
    pub expires_at: DateTime<Utc>,
}

/// How the caller proved their session on this request. Distinguishes
/// `Bearer` (Authorization header — desktop / native clients / agents)
/// from `Cookie` (browser session cookie — `apps/web`). The CSRF guard
/// only enforces double-submit for `Cookie`-authed state-changing
/// requests; `Bearer` is the authority on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSource {
    Bearer,
    Cookie,
}

/// Authenticated principal attached to the request after session
/// middleware runs.
#[derive(Debug, Clone)]
pub struct SessionContext {
    pub session_id: Uuid,
    pub account_id: Uuid,
    pub auth_source: AuthSource,
}

#[derive(Debug, FromRow)]
struct SessionLookup {
    id: Uuid,
    account_id: Uuid,
    token_hash: String,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

#[derive(Clone)]
struct CachedSession {
    session_id: Uuid,
    account_id: Uuid,
    inserted_at: Instant,
}

pub struct SessionService {
    db: PgPool,
    cache: Mutex<HashMap<String, CachedSession>>,
}

impl SessionService {
    pub fn new(db: PgPool) -> Self {
        SessionService {
            db,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Issue a new session for an account, returning the raw secret to
    /// the caller exactly once.
    pub async fn issue(
        &self,
        account_id: Uuid,
        label: Option<String>,
    ) -> Result<IssuedSession, ApiError> {
        let secret = generate_secret();
        let prefix = secret[..PREFIX_LOOKUP_LEN].to_string();
        let hash =
            password::hash(&secret).map_err(|e| ApiError::Internal(Box::new(e)))?;
        let expires_at = Utc::now() + Duration::days(DEFAULT_TTL_DAYS);
        let label = label.unwrap_or_else(|| "web session".into());

        let row: (Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO sessions
                (account_id, token_prefix, token_hash, label, expires_at)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id
            "#,
        )
        .bind(account_id)
        .bind(&prefix)
        .bind(&hash)
        .bind(&label)
        .bind(expires_at)
        .fetch_one(&self.db)
        .await?;

        Ok(IssuedSession {
            id: row.0,
            account_id,
            secret,
            expires_at,
        })
    }

    /// Validate a session secret presented either via `Authorization:
    /// Bearer` or via the `ourtex_session` cookie. Returns the session
    /// context tagged with the source; any mismatch — unknown prefix,
    /// wrong secret, expired, revoked — collapses to `Unauthorized`.
    pub async fn authenticate(
        &self,
        bearer: &str,
        source: AuthSource,
    ) -> Result<SessionContext, ApiError> {
        if !bearer.starts_with(TOKEN_PREFIX) {
            return Err(ApiError::Unauthorized);
        }

        if let Some(cached) = self.cache_get(bearer) {
            return Ok(SessionContext {
                session_id: cached.session_id,
                account_id: cached.account_id,
                auth_source: source,
            });
        }

        if bearer.len() < PREFIX_LOOKUP_LEN {
            return Err(ApiError::Unauthorized);
        }
        let prefix = &bearer[..PREFIX_LOOKUP_LEN];

        let row: Option<SessionLookup> = sqlx::query_as(
            r#"
            SELECT id, account_id, token_hash, expires_at, revoked_at
            FROM sessions
            WHERE token_prefix = $1
            "#,
        )
        .bind(prefix)
        .fetch_optional(&self.db)
        .await?;

        let Some(row) = row else {
            return Err(ApiError::Unauthorized);
        };
        if row.revoked_at.is_some() {
            return Err(ApiError::Unauthorized);
        }
        if row.expires_at <= Utc::now() {
            return Err(ApiError::Unauthorized);
        }

        let ok = password::verify(bearer, &row.token_hash)
            .map_err(|e| ApiError::Internal(Box::new(e)))?;
        if !ok {
            return Err(ApiError::Unauthorized);
        }

        // Touch last_used. Fire-and-log; a failed update shouldn't
        // deny the request.
        if let Err(e) = sqlx::query("UPDATE sessions SET last_used_at = now() WHERE id = $1")
            .bind(row.id)
            .execute(&self.db)
            .await
        {
            tracing::warn!(error = %e, "failed to update last_used_at");
        }

        self.cache_put(bearer, row.id, row.account_id);

        Ok(SessionContext {
            session_id: row.id,
            account_id: row.account_id,
            auth_source: source,
        })
    }

    /// Revoke a session by id. Idempotent.
    pub async fn revoke(&self, session_id: Uuid) -> Result<(), ApiError> {
        sqlx::query(
            "UPDATE sessions SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL",
        )
        .bind(session_id)
        .execute(&self.db)
        .await?;
        self.cache_invalidate_by_session(session_id);
        Ok(())
    }

    /// List non-revoked sessions for an account, newest first.
    pub async fn list_for_account(
        &self,
        account_id: Uuid,
    ) -> Result<Vec<SessionSummary>, ApiError> {
        let rows: Vec<SessionSummary> = sqlx::query_as(
            r#"
            SELECT id, label, created_at, expires_at, last_used_at
            FROM sessions
            WHERE account_id = $1 AND revoked_at IS NULL
            ORDER BY created_at DESC
            "#,
        )
        .bind(account_id)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    fn cache_get(&self, bearer: &str) -> Option<CachedSession> {
        let mut c = self.cache.lock().ok()?;
        let entry = c.get(bearer).cloned()?;
        if entry.inserted_at.elapsed() > CACHE_TTL {
            c.remove(bearer);
            return None;
        }
        Some(entry)
    }

    fn cache_put(&self, bearer: &str, session_id: Uuid, account_id: Uuid) {
        if let Ok(mut c) = self.cache.lock() {
            // Bound memory — drop everything if the cache gets huge.
            if c.len() > 10_000 {
                c.clear();
            }
            c.insert(
                bearer.to_string(),
                CachedSession {
                    session_id,
                    account_id,
                    inserted_at: Instant::now(),
                },
            );
        }
    }

    fn cache_invalidate_by_session(&self, session_id: Uuid) {
        if let Ok(mut c) = self.cache.lock() {
            c.retain(|_, v| v.session_id != session_id);
        }
    }
}

#[derive(Debug, serde::Serialize, FromRow)]
pub struct SessionSummary {
    pub id: Uuid,
    pub label: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

fn generate_secret() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("{TOKEN_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_secret_shape() {
        let s = generate_secret();
        assert!(s.starts_with(TOKEN_PREFIX));
        // base64url-no-pad of 32 bytes is 43 chars, plus "otx_" = 47.
        assert_eq!(s.len(), TOKEN_PREFIX.len() + 43);
        assert!(!s.contains('='));
    }

    #[test]
    fn distinct_secrets() {
        let a = generate_secret();
        let b = generate_secret();
        assert_ne!(a, b);
    }

    #[test]
    fn prefix_lookup_len_consistent() {
        // A real token must be longer than the lookup prefix.
        let s = generate_secret();
        assert!(s.len() > PREFIX_LOOKUP_LEN);
    }
}
