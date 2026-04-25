use crate::error::{AuthError, Result};
use crate::scope::{Mode, Scope};
use crate::secret::TokenSecret;
use crate::token::{AuthenticatedToken, Limits, PublicTokenInfo, StoredToken};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

const DEFAULT_EXPIRY_DAYS: i64 = 90;
const MAX_EXPIRY_DAYS: i64 = 365;
const FILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct IssueRequest {
    pub label: String,
    pub scope: Scope,
    pub mode: Mode,
    pub limits: Limits,
    pub ttl: Option<Duration>,
}

pub struct IssuedToken {
    pub secret: TokenSecret,
    pub info: PublicTokenInfo,
}

pub struct TokenService {
    path: PathBuf,
    state: Mutex<State>,
}

struct State {
    tokens: Vec<StoredToken>,
}

#[derive(Serialize, Deserialize)]
struct OnDisk {
    version: u32,
    tokens: Vec<StoredToken>,
}

impl TokenService {
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        let tokens = match tokio::fs::read(&path).await {
            Ok(bytes) if bytes.is_empty() => Vec::new(),
            Ok(bytes) => {
                let parsed: OnDisk = serde_json::from_slice(&bytes)?;
                parsed.tokens
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(AuthError::Io(e)),
        };
        Ok(Self {
            path,
            state: Mutex::new(State { tokens }),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn issue(&self, request: IssueRequest) -> Result<IssuedToken> {
        let ttl = clamp_ttl(request.ttl);
        let secret = TokenSecret::generate();
        let hash = hash_secret(secret.expose())?;
        let id = generate_token_id();

        let now = Utc::now();
        let stored = StoredToken {
            id: id.clone(),
            label: request.label,
            hash,
            scope: request.scope,
            mode: request.mode,
            limits: request.limits,
            created_at: now,
            expires_at: now + ttl,
            last_used: None,
            revoked_at: None,
        };

        let info = PublicTokenInfo::from(&stored);
        {
            let mut state = self.state.lock().await;
            state.tokens.push(stored);
            persist(&self.path, &state.tokens).await?;
        }
        Ok(IssuedToken { secret, info })
    }

    pub async fn authenticate(&self, presented: &str) -> Result<AuthenticatedToken> {
        let presented = TokenSecret::from_str(presented)?;
        let state = self.state.lock().await;
        let now = Utc::now();

        for token in &state.tokens {
            if verify_secret(presented.expose(), &token.hash)? {
                if token.revoked_at.is_some() {
                    return Err(AuthError::Revoked);
                }
                if token.expires_at <= now {
                    return Err(AuthError::Expired);
                }
                return Ok(AuthenticatedToken::from(token));
            }
        }
        Err(AuthError::UnknownToken)
    }

    pub async fn revoke(&self, id: &str) -> Result<()> {
        let mut state = self.state.lock().await;
        let token = state
            .tokens
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| AuthError::NotFound(id.to_string()))?;
        if token.revoked_at.is_none() {
            token.revoked_at = Some(Utc::now());
        }
        persist(&self.path, &state.tokens).await?;
        Ok(())
    }

    pub async fn mark_used(&self, id: &str, at: DateTime<Utc>) -> Result<()> {
        let mut state = self.state.lock().await;
        let token = state
            .tokens
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| AuthError::NotFound(id.to_string()))?;
        token.last_used = Some(at);
        persist(&self.path, &state.tokens).await?;
        Ok(())
    }

    pub async fn list(&self) -> Vec<PublicTokenInfo> {
        let state = self.state.lock().await;
        state.tokens.iter().map(PublicTokenInfo::from).collect()
    }
}

fn clamp_ttl(ttl: Option<Duration>) -> Duration {
    let max = Duration::days(MAX_EXPIRY_DAYS);
    let default = Duration::days(DEFAULT_EXPIRY_DAYS);
    match ttl {
        None => default,
        Some(d) if d > max => max,
        Some(d) if d <= Duration::zero() => default,
        Some(d) => d,
    }
}

fn generate_token_id() -> String {
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("tok_{}", URL_SAFE_NO_PAD.encode(bytes))
}

fn hash_secret(secret: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon = Argon2::default();
    let hash = argon
        .hash_password(secret.as_bytes(), &salt)
        .map_err(|e| AuthError::Argon2(e.to_string()))?;
    Ok(hash.to_string())
}

fn verify_secret(secret: &str, stored_hash: &str) -> Result<bool> {
    let parsed = match PasswordHash::new(stored_hash) {
        Ok(h) => h,
        Err(e) => return Err(AuthError::Argon2(e.to_string())),
    };
    match Argon2::default().verify_password(secret.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(AuthError::Argon2(e.to_string())),
    }
}

async fn persist(path: &Path, tokens: &[StoredToken]) -> Result<()> {
    let on_disk = OnDisk {
        version: FILE_SCHEMA_VERSION,
        tokens: tokens.to_vec(),
    };
    let bytes = serde_json::to_vec_pretty(&on_disk)?;
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}
