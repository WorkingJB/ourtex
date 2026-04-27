//! Client-side wrappers for the server's `/v1/t/:tid/proposals` review
//! queue. Used by the desktop's "Proposals" pane against remote
//! workspaces; lives outside the `VaultDriver` trait because the
//! review surface is its own admin concern, not a vault data op.

use crate::{client::RemoteClient, error::Result};
use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Proposal {
    pub id: String,
    pub doc_id: String,
    pub base_version: String,
    pub patch: JsonValue,
    pub reason: Option<String>,
    pub status: String,
    pub actor_token_id: Option<String>,
    pub actor_token_label: String,
    pub actor_account_id: Option<Uuid>,
    pub decided_by: Option<Uuid>,
    pub decided_at: Option<DateTime<Utc>>,
    pub decision_note: Option<String>,
    pub applied_version: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ListResponse {
    pub proposals: Vec<Proposal>,
}

#[derive(Debug, Serialize)]
struct DecideRequest<'a> {
    note: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
pub struct ApproveResponse {
    pub proposal: Proposal,
    pub applied_version: String,
}

impl RemoteClient {
    pub async fn list_proposals(&self, status: &str) -> Result<ListResponse> {
        let mut url = self.config.tenant_url("proposals")?;
        url.query_pairs_mut().append_pair("status", status);
        self.request_json::<(), _>(Method::GET, url, None).await
    }

    pub async fn approve_proposal(
        &self,
        id: &str,
        note: Option<&str>,
    ) -> Result<ApproveResponse> {
        let url = self
            .config
            .tenant_url(&format!("proposals/{}/approve", urlencoding(id)))?;
        let body = DecideRequest { note };
        self.request_json(Method::POST, url, Some(&body)).await
    }

    pub async fn reject_proposal(&self, id: &str, note: Option<&str>) -> Result<Proposal> {
        let url = self
            .config
            .tenant_url(&format!("proposals/{}/reject", urlencoding(id)))?;
        let body = DecideRequest { note };
        self.request_json(Method::POST, url, Some(&body)).await
    }
}

fn urlencoding(s: &str) -> String {
    // Proposal ids are `prop-YYYYMMDD-<8 hex>` — no special characters
    // to escape today, but route this through `Url`'s percent-encoder
    // anyway so a future format change can't open an injection vector.
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
