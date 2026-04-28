//! `VaultDriver` impl that goes over HTTP to `orchext-server`.
//!
//! Same trait, same semantics, different backing store — every local
//! caller of `VaultDriver` (including `orchext-index::Index::reindex_from`
//! and the desktop's existing Tauri commands) works unchanged against a
//! `RemoteVaultDriver`.
//!
//! There is no client-side cache here: the common call pattern is
//! "reindex once into a local `orchext-index::Index`, then serve reads
//! out of that index," so list/read aren't hot paths once the workspace
//! is open. If that assumption changes, a short-TTL `list` cache slots
//! in cleanly at this layer.

use crate::{
    client::RemoteClient,
    error::{Result, SyncError},
};
use async_trait::async_trait;
use orchext_vault::{Document, DocumentId, Entry, Result as VaultResult, VaultDriver, VaultError};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub struct RemoteVaultDriver {
    client: RemoteClient,
}

impl RemoteVaultDriver {
    pub fn new(client: RemoteClient) -> Self {
        Self { client }
    }

    pub fn client(&self) -> &RemoteClient {
        &self.client
    }

    /// Write with an optional base-version precondition. Returns the
    /// new version on success. Surfaces `VersionConflict` when the
    /// precondition fails so callers can drive a merge UI.
    pub async fn write_versioned(
        &self,
        id: &DocumentId,
        doc: &Document,
        base_version: Option<&str>,
    ) -> Result<WriteResponse> {
        self.write_versioned_with_team(id, doc, base_version, None)
            .await
    }

    /// Like `write_versioned`, but also pins the doc's team binding
    /// (Phase 3 platform Slice 2). The server enforces strict coupling
    /// — `team_id` must be `Some` iff `frontmatter.visibility == "team"`.
    pub async fn write_versioned_with_team(
        &self,
        id: &DocumentId,
        doc: &Document,
        base_version: Option<&str>,
        team_id: Option<uuid::Uuid>,
    ) -> Result<WriteResponse> {
        if doc.frontmatter.id != *id {
            return Err(SyncError::InvalidArgument(format!(
                "frontmatter id {:?} does not match write id {:?}",
                doc.frontmatter.id.as_str(),
                id.as_str()
            )));
        }
        let source = doc
            .serialize()
            .map_err(|e| SyncError::Document(e.to_string()))?;
        let url = self
            .client
            .config
            .tenant_url(&format!("vault/docs/{id}"))?;
        let body = WriteRequest {
            source,
            base_version: base_version.map(str::to_string),
            team_id,
        };
        self.client
            .request_json::<_, WriteResponse>(Method::PUT, url, Some(&body))
            .await
    }

    /// Read a doc and surface its server-side team binding alongside
    /// the parsed `Document`. Mirrors the trait `read` shape but adds
    /// the team_id needed by the desktop's editor to pre-fill the
    /// team picker on existing team docs.
    pub async fn read_with_team_id(
        &self,
        id: &DocumentId,
    ) -> Result<(Document, Option<uuid::Uuid>)> {
        let url = self
            .client
            .config
            .tenant_url(&format!("vault/docs/{id}"))?;
        let resp: DocResponse = self
            .client
            .request_json::<(), _>(Method::GET, url, None)
            .await?;
        let doc = Document::parse(&resp.source)
            .map_err(|e| SyncError::Document(e.to_string()))?;
        Ok((doc, resp.team_id))
    }

    /// Delete with an optional base-version precondition.
    pub async fn delete_versioned(
        &self,
        id: &DocumentId,
        base_version: Option<&str>,
    ) -> Result<()> {
        let mut url = self
            .client
            .config
            .tenant_url(&format!("vault/docs/{id}"))?;
        if let Some(ver) = base_version {
            url.query_pairs_mut().append_pair("base_version", ver);
        }
        self.client.request_empty(Method::DELETE, url).await
    }

    /// List with both `type` and `team_id` filters. The `VaultDriver`
    /// trait's `list` contract is type-only because team filtering is
    /// remote-specific (local vaults have no team concept); this is
    /// the escape hatch for the desktop's docs view, which surfaces a
    /// team filter when the active context is an org workspace.
    pub async fn list_with_filters(
        &self,
        type_filter: Option<&str>,
        team_id: Option<uuid::Uuid>,
    ) -> VaultResult<Vec<Entry>> {
        let mut url = vault_url(&self.client, "vault/docs")?;
        if let Some(t) = type_filter {
            url.query_pairs_mut().append_pair("type", t);
        }
        if let Some(tid) = team_id {
            url.query_pairs_mut().append_pair("team_id", &tid.to_string());
        }
        let resp: ListResponse = self
            .client
            .request_json::<(), _>(Method::GET, url, None)
            .await
            .map_err(into_vault_err)?;
        let mut out = Vec::with_capacity(resp.entries.len());
        for e in resp.entries {
            let id = DocumentId::new(e.doc_id.clone())
                .map_err(|_| VaultError::InvalidId(e.doc_id.clone()))?;
            out.push(Entry {
                id,
                type_: e.type_.clone(),
                path: PathBuf::from(format!("remote://{}/{}.md", e.type_, e.doc_id)),
            });
        }
        Ok(out)
    }
}

// ---------- wire types ----------

#[derive(Debug, Serialize)]
struct WriteRequest {
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_version: Option<String>,
    /// Team binding for `visibility = 'team'` docs. Required when the
    /// frontmatter's visibility is `team`; rejected otherwise. The DB
    /// CHECK constraint backs the same coupling on the server.
    #[serde(skip_serializing_if = "Option::is_none")]
    team_id: Option<uuid::Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct WriteResponse {
    pub doc_id: String,
    pub type_: String,
    pub visibility: String,
    pub version: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub team_id: Option<uuid::Uuid>,
}

#[derive(Debug, Deserialize)]
struct ListResponse {
    entries: Vec<ListEntryDto>,
}

/// Wire shape is broader (title/tags/updated/visibility) but the
/// `VaultDriver::list` contract only needs id + type_. Extra server
/// fields are ignored via serde's default behaviour.
#[derive(Debug, Deserialize)]
struct ListEntryDto {
    doc_id: String,
    type_: String,
}

/// Same shape principle as `ListEntryDto` — only `source` is consumed
/// by the trait `read` path. `team_id` is surfaced via
/// `read_with_team_id` for callers that need it (the desktop's
/// doc_read command, which propagates it to the team picker).
#[derive(Debug, Deserialize)]
struct DocResponse {
    source: String,
    #[serde(default)]
    team_id: Option<uuid::Uuid>,
}

// ---------- VaultDriver impl ----------

#[async_trait]
impl VaultDriver for RemoteVaultDriver {
    async fn list(&self, type_filter: Option<&str>) -> VaultResult<Vec<Entry>> {
        // Trait contract is type-only; team filtering goes through the
        // non-trait `list_with_filters` (called directly from desktop's
        // doc_list when an org team filter is active).
        self.list_with_filters(type_filter, None).await
    }

    async fn read(&self, id: &DocumentId) -> VaultResult<Document> {
        let url = vault_url(&self.client, &format!("vault/docs/{id}"))?;
        let resp: DocResponse = self
            .client
            .request_json::<(), _>(Method::GET, url, None)
            .await
            .map_err(into_vault_err)?;
        Document::parse(&resp.source)
    }

    async fn write(&self, id: &DocumentId, doc: &Document) -> VaultResult<()> {
        // Unconditional write — the base_version flavor is available
        // via `write_versioned` for callers that want the precondition.
        self.write_versioned(id, doc, None)
            .await
            .map(|_| ())
            .map_err(into_vault_err)
    }

    async fn delete(&self, id: &DocumentId) -> VaultResult<()> {
        self.delete_versioned(id, None)
            .await
            .map_err(into_vault_err)
    }
}

fn vault_url(client: &RemoteClient, suffix: &str) -> VaultResult<url::Url> {
    client
        .config
        .tenant_url(suffix)
        .map_err(|e| VaultError::NotFound(e.to_string()))
}

fn into_vault_err(e: SyncError) -> VaultError {
    // VaultError's variant set is narrow; the mapping below is
    // best-effort. Network / server errors collapse to NotFound with
    // the message preserved — the desktop's command layer translates
    // back out before surfacing to the user.
    match e {
        SyncError::NotFound => VaultError::NotFound(String::from("remote: not found")),
        SyncError::Unauthorized => {
            VaultError::NotFound(String::from("remote: unauthorized"))
        }
        SyncError::InvalidArgument(msg) => VaultError::InvalidId(msg),
        other => VaultError::NotFound(format!("remote: {other}")),
    }
}
