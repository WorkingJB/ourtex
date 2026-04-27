//! Server-level org / auth helpers used by native clients.
//!
//! Phase 3 platform Slice 1 added `/v1/orgs/*` (org metadata, member
//! admin, pending-signup queue, invitations) and the `/v1/auth/me`
//! probe. Web hits these with cookie auth; desktop hits them with the
//! workspace's bearer secret.
//!
//! These are standalone functions taking `(server_url, session_token,
//! ...)` rather than methods on `RemoteClient` because the calls are
//! server-scoped, not tenant-scoped — `RemoteClient` carries a
//! `tenant_id` and a `RemoteConfig::tenant_url(...)` builder that
//! doesn't apply here. Mirrors how `list_tenants` is shaped.
//!
//! Body shapes match the server's `crates/orchext-server/src/orgs.rs`
//! and `auth.rs` exactly.

use crate::client::translate_error;
use crate::error::Result;
use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use url::Url;
use uuid::Uuid;

// ---------- DTOs (mirror server) ----------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountInfo {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeResponse {
    pub account: AccountInfo,
    pub session_id: Uuid,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Organization {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub logo_url: Option<String>,
    pub allowed_domains: JsonValue,
    pub settings: JsonValue,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrgMembership {
    pub org_id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub logo_url: Option<String>,
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PendingSignup {
    pub id: Uuid,
    pub org_id: Uuid,
    pub org_name: String,
    pub requested_role: String,
    pub status: String,
    pub requested_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrgsListResponse {
    pub memberships: Vec<OrgMembership>,
    pub pending: Vec<PendingSignup>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemberDetail {
    pub account_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MembersResponse {
    pub members: Vec<MemberDetail>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PendingDetail {
    pub id: Uuid,
    pub account_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub requested_role: String,
    pub status: String,
    pub note: Option<String>,
    pub requested_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PendingResponse {
    pub pending: Vec<PendingDetail>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Invitation {
    pub id: Uuid,
    pub org_id: Uuid,
    pub email: String,
    pub role: String,
    pub invited_by: Uuid,
    pub invited_at: DateTime<Utc>,
    pub redeemed_at: Option<DateTime<Utc>>,
    pub redeemed_by: Option<Uuid>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InvitationsResponse {
    pub invitations: Vec<Invitation>,
}

// ---------- request bodies ----------

#[derive(Debug, Serialize)]
pub struct CreateOrgInput<'a> {
    pub name: &'a str,
}

#[derive(Debug, Default, Serialize)]
pub struct UpdateOrgInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<JsonValue>,
}

#[derive(Debug, Serialize)]
struct PatchMemberInput<'a> {
    role: &'a str,
}

#[derive(Debug, Serialize)]
struct ApproveInput<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct CreateInvitationInput<'a> {
    email: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'a str>,
}

// ---------- helpers ----------

async fn get_json<T: serde::de::DeserializeOwned>(url: Url, token: &str) -> Result<T> {
    let resp = reqwest::Client::new()
        .request(Method::GET, url)
        .bearer_auth(token)
        .send()
        .await?;
    let status = resp.status();
    if status.is_success() {
        Ok(resp.json().await?)
    } else {
        Err(translate_error(status, resp).await)
    }
}

async fn send_json<B: Serialize, T: serde::de::DeserializeOwned>(
    method: Method,
    url: Url,
    token: &str,
    body: &B,
) -> Result<T> {
    let resp = reqwest::Client::new()
        .request(method, url)
        .bearer_auth(token)
        .json(body)
        .send()
        .await?;
    let status = resp.status();
    if status.is_success() {
        Ok(resp.json().await?)
    } else {
        Err(translate_error(status, resp).await)
    }
}

async fn delete_no_body(url: Url, token: &str) -> Result<()> {
    let resp = reqwest::Client::new()
        .request(Method::DELETE, url)
        .bearer_auth(token)
        .send()
        .await?;
    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(translate_error(status, resp).await)
    }
}

async fn post_no_body_no_resp(url: Url, token: &str) -> Result<()> {
    let resp = reqwest::Client::new()
        .request(Method::POST, url)
        .bearer_auth(token)
        .send()
        .await?;
    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(translate_error(status, resp).await)
    }
}

// ---------- /v1/auth ----------

pub async fn auth_me(server_url: &Url, token: &str) -> Result<MeResponse> {
    get_json(server_url.join("v1/auth/me")?, token).await
}

pub async fn auth_logout(server_url: &Url, token: &str) -> Result<()> {
    delete_no_body(server_url.join("v1/auth/logout")?, token).await
}

// ---------- /v1/orgs ----------

pub async fn orgs_list(server_url: &Url, token: &str) -> Result<OrgsListResponse> {
    get_json(server_url.join("v1/orgs")?, token).await
}

pub async fn org_create(
    server_url: &Url,
    token: &str,
    name: &str,
) -> Result<Organization> {
    send_json(
        Method::POST,
        server_url.join("v1/orgs")?,
        token,
        &CreateOrgInput { name },
    )
    .await
}

pub async fn org_get(server_url: &Url, token: &str, org_id: Uuid) -> Result<Organization> {
    get_json(server_url.join(&format!("v1/orgs/{org_id}"))?, token).await
}

pub async fn org_update(
    server_url: &Url,
    token: &str,
    org_id: Uuid,
    input: &UpdateOrgInput,
) -> Result<Organization> {
    send_json(
        Method::PATCH,
        server_url.join(&format!("v1/orgs/{org_id}"))?,
        token,
        input,
    )
    .await
}

// ---------- /v1/orgs/:id/members ----------

pub async fn org_members(
    server_url: &Url,
    token: &str,
    org_id: Uuid,
) -> Result<MembersResponse> {
    get_json(
        server_url.join(&format!("v1/orgs/{org_id}/members"))?,
        token,
    )
    .await
}

pub async fn org_member_update(
    server_url: &Url,
    token: &str,
    org_id: Uuid,
    account_id: Uuid,
    role: &str,
) -> Result<MemberDetail> {
    send_json(
        Method::PATCH,
        server_url.join(&format!("v1/orgs/{org_id}/members/{account_id}"))?,
        token,
        &PatchMemberInput { role },
    )
    .await
}

pub async fn org_member_remove(
    server_url: &Url,
    token: &str,
    org_id: Uuid,
    account_id: Uuid,
) -> Result<()> {
    delete_no_body(
        server_url.join(&format!("v1/orgs/{org_id}/members/{account_id}"))?,
        token,
    )
    .await
}

// ---------- /v1/orgs/:id/pending ----------

pub async fn org_pending(
    server_url: &Url,
    token: &str,
    org_id: Uuid,
    status: Option<&str>,
) -> Result<PendingResponse> {
    let path = match status {
        Some(s) => format!("v1/orgs/{org_id}/pending?status={s}"),
        None => format!("v1/orgs/{org_id}/pending"),
    };
    get_json(server_url.join(&path)?, token).await
}

pub async fn org_pending_approve(
    server_url: &Url,
    token: &str,
    org_id: Uuid,
    account_id: Uuid,
    role: Option<&str>,
) -> Result<MemberDetail> {
    send_json(
        Method::POST,
        server_url.join(&format!(
            "v1/orgs/{org_id}/pending/{account_id}/approve"
        ))?,
        token,
        &ApproveInput { role },
    )
    .await
}

pub async fn org_pending_reject(
    server_url: &Url,
    token: &str,
    org_id: Uuid,
    account_id: Uuid,
) -> Result<()> {
    post_no_body_no_resp(
        server_url.join(&format!(
            "v1/orgs/{org_id}/pending/{account_id}/reject"
        ))?,
        token,
    )
    .await
}

// ---------- /v1/orgs/:id/invitations ----------

pub async fn org_invitations(
    server_url: &Url,
    token: &str,
    org_id: Uuid,
    status: Option<&str>,
) -> Result<InvitationsResponse> {
    let path = match status {
        Some(s) => format!("v1/orgs/{org_id}/invitations?status={s}"),
        None => format!("v1/orgs/{org_id}/invitations"),
    };
    get_json(server_url.join(&path)?, token).await
}

pub async fn org_invite(
    server_url: &Url,
    token: &str,
    org_id: Uuid,
    email: &str,
    role: Option<&str>,
) -> Result<Invitation> {
    send_json(
        Method::POST,
        server_url.join(&format!("v1/orgs/{org_id}/invitations"))?,
        token,
        &CreateInvitationInput { email, role },
    )
    .await
}

pub async fn org_invitation_delete(
    server_url: &Url,
    token: &str,
    org_id: Uuid,
    invitation_id: Uuid,
) -> Result<()> {
    delete_no_body(
        server_url.join(&format!(
            "v1/orgs/{org_id}/invitations/{invitation_id}"
        ))?,
        token,
    )
    .await
}

