//! Client-side wrappers for the server's crypto control-plane
//! endpoints (`/v1/t/:tid/vault/crypto`, `init-crypto`, `session-key`).
//!
//! Sits alongside `RemoteVaultDriver` — these calls share the same
//! bearer-authed `RemoteClient` but aren't part of the `VaultDriver`
//! trait because crypto setup is a control concern, not a data op.

use crate::{client::RemoteClient, error::Result};
use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct CryptoState {
    pub seeded: bool,
    pub kdf_salt: Option<String>,
    pub wrapped_content_key: Option<String>,
    pub key_version: Option<i32>,
    pub unlocked: bool,
}

#[derive(Debug, Serialize)]
struct InitCryptoRequest {
    kdf_salt: String,
    wrapped_content_key: String,
}

#[derive(Debug, Deserialize)]
pub struct InitCryptoResponse {
    pub key_version: i32,
}

#[derive(Debug, Serialize)]
struct PublishRequest {
    key: String,
}

#[derive(Debug, Deserialize)]
pub struct PublishResponse {
    pub expires_at: DateTime<Utc>,
    pub ttl_seconds: i64,
}

impl RemoteClient {
    pub async fn get_crypto_state(&self) -> Result<CryptoState> {
        let url = self.config.tenant_url("vault/crypto")?;
        self.request_json::<(), _>(Method::GET, url, None).await
    }

    pub async fn init_crypto(
        &self,
        kdf_salt: &str,
        wrapped_content_key: &str,
    ) -> Result<InitCryptoResponse> {
        let url = self.config.tenant_url("vault/init-crypto")?;
        let body = InitCryptoRequest {
            kdf_salt: kdf_salt.to_string(),
            wrapped_content_key: wrapped_content_key.to_string(),
        };
        self.request_json(Method::POST, url, Some(&body)).await
    }

    /// Publish the raw content key to the server's short-TTL store.
    /// The key crosses the wire here — always over TLS in production.
    pub async fn publish_session_key(&self, key_wire: &str) -> Result<PublishResponse> {
        let url = self.config.tenant_url("session-key")?;
        let body = PublishRequest {
            key: key_wire.to_string(),
        };
        self.request_json(Method::POST, url, Some(&body)).await
    }

    pub async fn revoke_session_key(&self) -> Result<()> {
        let url = self.config.tenant_url("session-key")?;
        self.request_empty(Method::DELETE, url).await
    }
}
