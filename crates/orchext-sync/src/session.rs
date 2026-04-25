//! Login / tenant-list helpers.
//!
//! Clients typically follow this flow on first remote-workspace setup:
//!
//! 1. `login(server_url, email, password)` → `LoginOutcome { session, account }`
//! 2. `list_tenants(server_url, &session.secret)` → `Vec<Tenant>`
//! 3. pick one tenant, persist `(server_url, tenant_id, session_token)`
//! 4. construct `RemoteConfig` + `RemoteVaultDriver`
//!
//! `list_tenants` is implemented as a thin standalone helper rather than
//! a method on `RemoteClient` because the client needs a tenant_id to
//! construct, but the caller doesn't have one yet at this stage.

use crate::error::{Result, SyncError};
use chrono::{DateTime, Utc};
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct LoginInput {
    pub email: String,
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginOutcome {
    pub account: Account,
    pub session: SessionIssued,
}

#[derive(Debug, Deserialize)]
pub struct Account {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct SessionIssued {
    pub id: Uuid,
    pub secret: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Tenant {
    pub tenant_id: Uuid,
    pub name: String,
    pub kind: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct TenantsResponse {
    memberships: Vec<Tenant>,
}

pub async fn login(server_url: &Url, input: &LoginInput) -> Result<LoginOutcome> {
    let client = reqwest::Client::new();
    let url = server_url.join("v1/auth/login")?;
    let resp = client.post(url).json(input).send().await?;
    let status = resp.status();
    if status.is_success() {
        Ok(resp.json().await?)
    } else if status == StatusCode::UNAUTHORIZED {
        Err(SyncError::Unauthorized)
    } else {
        Err(crate::client::translate_error(status, resp).await)
    }
}

pub async fn list_tenants(server_url: &Url, session_token: &str) -> Result<Vec<Tenant>> {
    let client = reqwest::Client::new();
    let url = server_url.join("v1/tenants")?;
    let resp = client
        .request(Method::GET, url)
        .bearer_auth(session_token)
        .send()
        .await?;
    let status = resp.status();
    if status.is_success() {
        let r: TenantsResponse = resp.json().await?;
        Ok(r.memberships)
    } else {
        Err(crate::client::translate_error(status, resp).await)
    }
}
