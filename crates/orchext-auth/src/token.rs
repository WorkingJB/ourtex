use crate::scope::{Mode, Scope};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub max_docs: u32,
    pub max_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_docs: 20,
            max_bytes: 64 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredToken {
    pub id: String,
    pub label: String,
    pub hash: String,
    pub scope: Scope,
    pub mode: Mode,
    pub limits: Limits,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Redacted public view of a token. Used by the UI and anywhere the hash
/// should not travel.
#[derive(Debug, Clone, Serialize)]
pub struct PublicTokenInfo {
    pub id: String,
    pub label: String,
    pub scope: Vec<String>,
    pub mode: Mode,
    pub limits: Limits,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl From<&StoredToken> for PublicTokenInfo {
    fn from(t: &StoredToken) -> Self {
        Self {
            id: t.id.clone(),
            label: t.label.clone(),
            scope: t.scope.labels().map(str::to_string).collect(),
            mode: t.mode,
            limits: t.limits,
            created_at: t.created_at,
            expires_at: t.expires_at,
            last_used: t.last_used,
            revoked_at: t.revoked_at,
        }
    }
}

/// The result of a successful `authenticate`. Holds everything the caller
/// needs to evaluate a request; never holds the secret or the hash.
#[derive(Debug, Clone)]
pub struct AuthenticatedToken {
    pub id: String,
    pub label: String,
    pub scope: Scope,
    pub mode: Mode,
    pub limits: Limits,
    pub expires_at: DateTime<Utc>,
}

impl From<&StoredToken> for AuthenticatedToken {
    fn from(t: &StoredToken) -> Self {
        Self {
            id: t.id.clone(),
            label: t.label.clone(),
            scope: t.scope.clone(),
            mode: t.mode,
            limits: t.limits,
            expires_at: t.expires_at,
        }
    }
}
