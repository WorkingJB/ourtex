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

async fn send_json_no_resp<B: Serialize>(
    method: Method,
    url: Url,
    token: &str,
    body: &B,
) -> Result<()> {
    let resp = reqwest::Client::new()
        .request(method, url)
        .bearer_auth(token)
        .json(body)
        .send()
        .await?;
    let status = resp.status();
    if status.is_success() {
        Ok(())
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

#[derive(Debug, Serialize)]
struct UpdateAccountInput<'a> {
    display_name: &'a str,
}

pub async fn auth_account_update(
    server_url: &Url,
    token: &str,
    display_name: &str,
) -> Result<AccountInfo> {
    send_json(
        Method::PATCH,
        server_url.join("v1/auth/account")?,
        token,
        &UpdateAccountInput { display_name },
    )
    .await
}

#[derive(Debug, Serialize)]
struct ChangePasswordInput<'a> {
    current_password: &'a str,
    new_password: &'a str,
}

pub async fn auth_password_change(
    server_url: &Url,
    token: &str,
    current_password: &str,
    new_password: &str,
) -> Result<()> {
    send_json_no_resp(
        Method::POST,
        server_url.join("v1/auth/password")?,
        token,
        &ChangePasswordInput {
            current_password,
            new_password,
        },
    )
    .await
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

// ---------- /v1/orgs/:id/logo (Phase 3 platform Slice 2) ----------

/// Bytes + mime returned by `org_logo_get`. `None` is encoded as a
/// 404 from the server.
#[derive(Debug, Clone)]
pub struct LogoBytes {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub etag: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogoUploadResponse {
    pub logo_url: String,
    pub content_type: String,
    pub sha256: String,
    pub bytes: usize,
}

pub async fn org_logo_get(
    server_url: &Url,
    token: &str,
    org_id: Uuid,
) -> Result<LogoBytes> {
    let resp = reqwest::Client::new()
        .request(
            Method::GET,
            server_url.join(&format!("v1/orgs/{org_id}/logo"))?,
        )
        .bearer_auth(token)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(translate_error(status, resp).await);
    }
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = resp.bytes().await?.to_vec();
    Ok(LogoBytes {
        bytes,
        content_type,
        etag,
    })
}

pub async fn org_logo_upload(
    server_url: &Url,
    token: &str,
    org_id: Uuid,
    bytes: Vec<u8>,
    filename: &str,
    mime: Option<&str>,
) -> Result<LogoUploadResponse> {
    let part_mime = mime.unwrap_or("application/octet-stream");
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(filename.to_string())
        .mime_str(part_mime)
        .map_err(|e| {
            crate::error::SyncError::InvalidArgument(format!("invalid mime: {e}"))
        })?;
    let form = reqwest::multipart::Form::new().part("file", part);
    let resp = reqwest::Client::new()
        .request(
            Method::POST,
            server_url.join(&format!("v1/orgs/{org_id}/logo"))?,
        )
        .bearer_auth(token)
        .multipart(form)
        .send()
        .await?;
    let status = resp.status();
    if status.is_success() {
        Ok(resp.json().await?)
    } else {
        Err(translate_error(status, resp).await)
    }
}

pub async fn org_logo_delete(
    server_url: &Url,
    token: &str,
    org_id: Uuid,
) -> Result<()> {
    delete_no_body(
        server_url.join(&format!("v1/orgs/{org_id}/logo"))?,
        token,
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

