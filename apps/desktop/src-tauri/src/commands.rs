//! Tauri commands invoked by the frontend. Each is a thin wrapper
//! around the mytex-* crates, returning serde-serializable DTOs so
//! the UI doesn't need to know about internal types.

use crate::onboarding::{self, ChatMessage, SeedDocDraft};
use crate::settings;
use crate::state::{self, AppState, HeartbeatHandle};
use crate::watch;
use crate::workspaces::WorkspaceEntry;
use chrono::{DateTime, Duration, Utc};
use mytex_audit::{verify, AuditEntry, Iter as AuditIter};
use mytex_auth::{IssueRequest, Mode, Scope};
use mytex_crypto::{
    derive_master_key, unwrap_content_key, wrap_content_key, ContentKey, Salt, SealedBlob,
};
use mytex_sync::{list_tenants, login, LoginInput};
use mytex_vault::{Document, DocumentId, Frontmatter, Visibility};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tauri::{AppHandle, State};

// ---------------- workspaces ----------------

#[derive(Debug, Serialize)]
pub struct WorkspaceInfo {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub path: String,
    pub active: bool,
}

#[derive(Debug, Serialize)]
pub struct VaultInfo {
    pub workspace_id: String,
    pub name: String,
    pub root: String,
    pub document_count: u64,
}

fn entry_to_info(entry: &WorkspaceEntry, active: bool) -> WorkspaceInfo {
    WorkspaceInfo {
        id: entry.id.clone(),
        name: entry.name.clone(),
        kind: entry.kind.clone(),
        path: entry.path.to_string_lossy().to_string(),
        active,
    }
}

/// List every registered workspace, with `active` flagged on exactly
/// one (if any). Cheap — does not open any vault.
#[tauri::command]
pub async fn workspace_list(state: State<'_, AppState>) -> Result<Vec<WorkspaceInfo>, String> {
    let reg = state.registry_snapshot().await;
    Ok(reg
        .workspaces
        .iter()
        .map(|w| entry_to_info(w, reg.is_active(&w.id)))
        .collect())
}

/// Register a new local workspace at `path` and activate it. Opens the
/// vault (reindex, watcher) and returns the active `VaultInfo`. If the
/// path is already registered, activates the existing entry instead of
/// creating a duplicate.
#[tauri::command]
pub async fn workspace_add(
    state: State<'_, AppState>,
    app: AppHandle,
    path: String,
    name: Option<String>,
) -> Result<VaultInfo, String> {
    let raw = PathBuf::from(&path);
    tokio::fs::create_dir_all(&raw)
        .await
        .map_err(|e| format!("create {}: {e}", raw.display()))?;
    let canon = raw
        .canonicalize()
        .map_err(|e| format!("canonicalize {}: {e}", raw.display()))?;

    let display_name = name
        .as_ref()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| default_name_from_path(&canon));

    let id = state
        .mutate_registry(|reg| {
            let entry = reg.add_local(display_name, canon.clone());
            let id = entry.id.clone();
            reg.set_active(&id)?;
            Ok(id)
        })
        .await?;

    activate_inner(&state, &app, &id).await
}

/// Activate an existing workspace. Opens it if not already open, drops
/// any previously-open vault.
#[tauri::command]
pub async fn workspace_activate(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
) -> Result<VaultInfo, String> {
    state
        .mutate_registry(|reg| reg.set_active(&id))
        .await?;
    activate_inner(&state, &app, &id).await
}

// ---------------- remote workspace connect ----------------

#[derive(Debug, Deserialize)]
pub struct ConnectRemoteInput {
    pub server_url: String,
    pub email: String,
    pub password: String,
    /// Optional workspace display name. Defaults to the server host +
    /// the tenant name.
    pub name: Option<String>,
    /// If the user belongs to multiple tenants (2c), this selects one.
    /// Null picks the first personal tenant (the common case today).
    pub tenant_id: Option<uuid::Uuid>,
}

/// Log into a remote `mytex-server`, pick a tenant, register the
/// workspace, and activate it. Persists the returned session token in
/// the local registry so subsequent activations don't prompt.
#[tauri::command]
pub async fn workspace_connect_remote(
    state: State<'_, AppState>,
    app: AppHandle,
    input: ConnectRemoteInput,
) -> Result<VaultInfo, String> {
    let server_url: url::Url = input
        .server_url
        .parse()
        .map_err(|e| format!("invalid server url: {e}"))?;

    // 1. login -> session token
    let outcome = login(
        &server_url,
        &LoginInput {
            email: input.email.trim().to_lowercase(),
            password: input.password.clone(),
            label: Some("mytex-desktop".into()),
        },
    )
    .await
    .map_err(|e| format!("login: {e}"))?;

    // 2. list memberships and pick a tenant
    let tenants = list_tenants(&server_url, &outcome.session.secret)
        .await
        .map_err(|e| format!("list tenants: {e}"))?;
    let chosen = match input.tenant_id {
        Some(id) => tenants
            .into_iter()
            .find(|t| t.tenant_id == id)
            .ok_or_else(|| "selected tenant not found in memberships".to_string())?,
        None => tenants
            .into_iter()
            .find(|t| t.kind == "personal")
            .ok_or_else(|| {
                format!(
                    "no personal tenant for account on {}; pass tenant_id explicitly",
                    server_url.host_str().unwrap_or("server")
                )
            })?,
    };

    // 3. pick cache root + register. Cache root is
    //    `<home>/.mytex/remote/<workspace_id>/` — chosen below after
    //    `add_remote` generates the id.
    let name = input.name.unwrap_or_else(|| {
        format!(
            "{}@{}",
            chosen.name,
            server_url.host_str().unwrap_or("server")
        )
    });

    let home = dirs_home();
    let remote_base = home.join(".mytex").join("remote");
    tokio::fs::create_dir_all(&remote_base)
        .await
        .map_err(|e| format!("create remote cache root: {e}"))?;

    let id = state
        .mutate_registry(|reg| {
            // Use a placeholder cache path; we know the workspace_id only
            // after registration, so we update the path in a second pass
            // below. For `add_remote`'s dedupe-on-(url, tenant_id) check,
            // the placeholder path is irrelevant.
            let entry = reg.add_remote(
                name.clone(),
                remote_base.clone(),
                server_url.to_string(),
                chosen.tenant_id,
                outcome.account.email.clone(),
                outcome.session.secret.clone(),
                outcome.session.expires_at,
            );
            let id = entry.id.clone();
            Ok(id)
        })
        .await?;

    // Rewrite the cache path to `~/.mytex/remote/<id>/`. Persist.
    state
        .mutate_registry(|reg| {
            if let Some(w) = reg.workspaces.iter_mut().find(|w| w.id == id) {
                w.path = remote_base.join(&id);
            }
            reg.set_active(&id)?;
            Ok(())
        })
        .await?;

    activate_inner(&state, &app, &id).await
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

// ---------------- unlock / lock (2b.3 encryption) ----------------

#[derive(Debug, Deserialize)]
pub struct UnlockInput {
    pub passphrase: String,
}

#[derive(Debug, Serialize)]
pub struct UnlockOutcome {
    /// Seconds until the published key expires on the server, absent
    /// further heartbeats. The heartbeat task refreshes well before
    /// this, so the user-visible lock-out is much longer than this
    /// value.
    pub ttl_seconds: i64,
    /// Document count after reindexing from the now-unlocked server.
    pub document_count: u64,
}

/// Unlock the active remote workspace: derive the master key from
/// the passphrase, fetch or seed the wrapped content key, publish the
/// raw content key to the server, start the heartbeat, and reindex.
///
/// For a brand-new tenant the server has no crypto material yet; this
/// command seeds it atomically (generate salt + content key, wrap,
/// `POST /vault/init-crypto`). Subsequent unlocks skip seeding and
/// just unwrap.
#[tauri::command]
pub async fn workspace_unlock(
    state: State<'_, AppState>,
    input: UnlockInput,
) -> Result<UnlockOutcome, String> {
    let svcs = state.active_services().await?;
    if !svcs.is_remote() {
        return Err("unlock is only meaningful for remote workspaces".into());
    }
    let client = svcs
        .remote_client
        .clone()
        .ok_or_else(|| "remote workspace has no client handle".to_string())?;

    let crypto = client
        .get_crypto_state()
        .await
        .map_err(|e| format!("fetch crypto state: {e}"))?;

    let (content_key, salt) = if crypto.seeded {
        let salt_wire = crypto
            .kdf_salt
            .ok_or_else(|| "server reported seeded but returned no salt".to_string())?;
        let wrapped_wire = crypto
            .wrapped_content_key
            .ok_or_else(|| "server reported seeded but returned no wrapped key".to_string())?;
        let salt =
            Salt::from_wire(&salt_wire).map_err(|e| format!("decode salt: {e}"))?;
        let master = derive_master_key(&input.passphrase, &salt)
            .map_err(|e| format!("derive master key: {e}"))?;
        let wrapped = SealedBlob::from_wire(&wrapped_wire)
            .map_err(|e| format!("decode wrapped key: {e}"))?;
        let content = unwrap_content_key(&wrapped, &master)
            .map_err(|_| "unwrap failed — wrong passphrase?".to_string())?;
        (content, salt)
    } else {
        // First-time seed. Generate a fresh salt + random content key,
        // wrap the content key under the passphrase-derived master,
        // and post both to the server. Atomic failure modes:
        //   - init-crypto 409 → someone beat us to seeding; re-fetch
        //     and unwrap normally on the next attempt.
        let salt = Salt::generate();
        let master = derive_master_key(&input.passphrase, &salt)
            .map_err(|e| format!("derive master key: {e}"))?;
        let content = ContentKey::generate();
        let wrapped = wrap_content_key(&content, &master)
            .map_err(|e| format!("wrap content key: {e}"))?;
        client
            .init_crypto(&salt.to_wire(), &wrapped.to_wire())
            .await
            .map_err(|e| format!("init-crypto: {e}"))?;
        (content, salt)
    };
    let _ = salt;

    let publish = client
        .publish_session_key(&content_key.to_wire())
        .await
        .map_err(|e| format!("publish session key: {e}"))?;

    // Kick off the heartbeat. Any prior heartbeat on the current
    // OpenVault is dropped + aborted by replacement.
    let heartbeat = HeartbeatHandle::spawn(client.clone(), content_key.to_wire());
    state.set_heartbeat(Some(heartbeat)).await;

    // Now that the server can decrypt, do a reindex so the local
    // cache reflects real content (pre-unlock reindex would have
    // hit vault_locked and left the index empty).
    let reindex_count = svcs
        .index
        .reindex_from(&*svcs.vault)
        .await
        .map_err(|e| format!("reindex: {e}"))?;

    Ok(UnlockOutcome {
        ttl_seconds: publish.ttl_seconds,
        document_count: reindex_count.documents,
    })
}

/// Lock the active remote workspace: stop the heartbeat and tell the
/// server to drop the cached content key. The local index cache is
/// left intact (it only holds metadata the user has already seen);
/// future reads hit `vault_locked` until the next unlock.
#[tauri::command]
pub async fn workspace_lock(state: State<'_, AppState>) -> Result<(), String> {
    let svcs = state.active_services().await?;
    if !svcs.is_remote() {
        return Err("lock is only meaningful for remote workspaces".into());
    }
    state.set_heartbeat(None).await; // drop => abort
    if let Some(client) = svcs.remote_client {
        if let Err(e) = client.revoke_session_key().await {
            tracing::warn!(err = %e, "revoke_session_key failed");
        }
    }
    Ok(())
}

/// Snapshot of the current crypto state for the active workspace —
/// used by the UI to decide between "Connect" / "Unlock" / "Lock"
/// affordances without exposing any secret material.
#[derive(Debug, Serialize)]
pub struct CryptoStateDto {
    pub kind: String,
    pub seeded: bool,
    pub unlocked: bool,
}

#[tauri::command]
pub async fn workspace_crypto_state(
    state: State<'_, AppState>,
) -> Result<CryptoStateDto, String> {
    let svcs = state.active_services().await?;
    if !svcs.is_remote() {
        return Ok(CryptoStateDto {
            kind: "local".into(),
            seeded: false,
            unlocked: true,
        });
    }
    let client = svcs
        .remote_client
        .ok_or_else(|| "remote workspace has no client handle".to_string())?;
    let c = client
        .get_crypto_state()
        .await
        .map_err(|e| format!("fetch crypto state: {e}"))?;
    Ok(CryptoStateDto {
        kind: "remote".into(),
        seeded: c.seeded,
        unlocked: c.unlocked,
    })
}

#[tauri::command]
pub async fn workspace_remove(state: State<'_, AppState>, id: String) -> Result<(), String> {
    if state.is_active_open(&id).await {
        state.clear_open().await;
    }
    state
        .mutate_registry(|reg| {
            reg.remove(&id)
                .ok_or_else(|| format!("unknown workspace: {id}"))?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn workspace_rename(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> Result<(), String> {
    state
        .mutate_registry(|reg| reg.rename(&id, name.trim().to_string()))
        .await
}

/// Info for the currently-active workspace. If the registry has an
/// active entry but it's not yet open, opens it first. Returns `None`
/// only when no workspace is registered at all (first run).
#[tauri::command]
pub async fn vault_info(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Option<VaultInfo>, String> {
    // Open the active workspace lazily on first request.
    let needs_open = {
        let reg = state.registry_snapshot().await;
        let Some(active) = reg.active_entry().cloned() else {
            return Ok(None);
        };
        if state.is_active_open(&active.id).await {
            None
        } else {
            Some(active)
        }
    };
    if let Some(entry) = needs_open {
        return Ok(Some(activate_inner(&state, &app, &entry.id).await?));
    }

    let svcs = state.active_services().await?;
    let reg = state.registry_snapshot().await;
    let entry = reg
        .active_entry()
        .ok_or_else(|| "active workspace missing from registry".to_string())?;
    let list = svcs
        .vault
        .list(None)
        .await
        .map_err(|e| format!("list: {e}"))?;
    Ok(Some(VaultInfo {
        workspace_id: entry.id.clone(),
        name: entry.name.clone(),
        root: entry.path.to_string_lossy().to_string(),
        document_count: list.len() as u64,
    }))
}

async fn activate_inner(
    state: &State<'_, AppState>,
    app: &AppHandle,
    id: &str,
) -> Result<VaultInfo, String> {
    let entry = {
        let reg = state.registry_snapshot().await;
        reg.find(id)
            .cloned()
            .ok_or_else(|| format!("unknown workspace: {id}"))?
    };

    // Drop any previously-open vault before opening the new one so the
    // old watcher is fully torn down.
    state.clear_open().await;

    let mut opened = state::open_workspace(&entry).await?;
    let list = opened
        .vault
        .list(None)
        .await
        .map_err(|e| format!("list: {e}"))?;
    let count = list.len() as u64;

    // The fs watcher is local-only. Remote workspaces pick up server
    // changes via periodic re-sync / SSE (deferred), not from a notify
    // watcher on the cache directory.
    if opened.kind == "local" {
        match watch::spawn(
            opened.root.clone(),
            opened.vault.clone(),
            opened.index.clone(),
            app.clone(),
        ) {
            Ok(handle) => opened._watcher = Some(handle),
            Err(e) => {
                tracing::warn!(err = %e, "fs watcher failed to start; live refresh disabled")
            }
        }
    }

    let root = opened.root.to_string_lossy().to_string();
    state.set_open(opened).await;

    Ok(VaultInfo {
        workspace_id: entry.id,
        name: entry.name,
        root,
        document_count: count,
    })
}

fn default_name_from_path(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Workspace".to_string())
}

// ---------------- documents ----------------

#[derive(Debug, Serialize)]
pub struct DocListItem {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub title: String,
    pub visibility: String,
    pub tags: Vec<String>,
    pub updated: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DocDetail {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub visibility: String,
    pub tags: Vec<String>,
    pub links: Vec<String>,
    pub aliases: Vec<String>,
    pub source: Option<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub body: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
pub struct DocInput {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub visibility: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    pub body: String,
}

#[tauri::command]
pub async fn doc_list(state: State<'_, AppState>) -> Result<Vec<DocListItem>, String> {
    let svcs = state.active_services().await?;
    let entries = svcs
        .vault
        .list(None)
        .await
        .map_err(|e| format!("list: {e}"))?;

    // Pull each doc's frontmatter so the list reflects visibility, tags,
    // updated, and a sensible title. For v1 vault sizes (hundreds of
    // docs) the O(n) reads are fine; swap to an index-only path if this
    // ever gets heavy.
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        match svcs.vault.read(&entry.id).await {
            Ok(doc) => {
                let title = title_from_body(&doc.body, entry.id.as_str());
                out.push(DocListItem {
                    id: entry.id.to_string(),
                    type_: entry.type_,
                    title,
                    visibility: doc.frontmatter.visibility.as_label().to_string(),
                    tags: doc.frontmatter.tags,
                    updated: doc.frontmatter.updated.map(|d| d.to_string()),
                });
            }
            Err(e) => {
                tracing::warn!(id = %entry.id, err = %e, "skipping unreadable doc");
            }
        }
    }
    out.sort_by(|a, b| {
        b.updated
            .cmp(&a.updated)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(out)
}

#[tauri::command]
pub async fn doc_read(
    state: State<'_, AppState>,
    id: String,
) -> Result<DocDetail, String> {
    let svcs = state.active_services().await?;
    let doc_id = DocumentId::new(id).map_err(|e| e.to_string())?;
    let doc = svcs
        .vault
        .read(&doc_id)
        .await
        .map_err(|e| format!("read: {e}"))?;
    let version = doc.version().map_err(|e| e.to_string())?;
    Ok(DocDetail {
        id: doc.frontmatter.id.to_string(),
        type_: doc.frontmatter.type_.clone(),
        visibility: doc.frontmatter.visibility.as_label().to_string(),
        tags: doc.frontmatter.tags,
        links: doc.frontmatter.links,
        aliases: doc.frontmatter.aliases,
        source: doc.frontmatter.source,
        created: doc.frontmatter.created.map(|d| d.to_string()),
        updated: doc.frontmatter.updated.map(|d| d.to_string()),
        body: doc.body,
        version,
    })
}

#[tauri::command]
pub async fn doc_write(
    state: State<'_, AppState>,
    input: DocInput,
) -> Result<DocDetail, String> {
    let svcs = state.active_services().await?;
    let id = DocumentId::new(input.id.clone()).map_err(|e| format!("id: {e}"))?;
    let visibility = Visibility::from_label(&input.visibility)
        .map_err(|e| format!("visibility: {e}"))?;
    let today = Utc::now().date_naive();

    // Preserve `created` from disk if the doc already exists; stamp
    // `updated` to today on every write.
    let existing = svcs.vault.read(&id).await.ok();
    let created = existing
        .as_ref()
        .and_then(|d| d.frontmatter.created)
        .or(Some(today));

    let fm = Frontmatter {
        id: id.clone(),
        type_: input.type_.clone(),
        visibility,
        tags: input.tags,
        links: input.links,
        aliases: input.aliases,
        created,
        updated: Some(today),
        source: input.source,
        principal: None,
        schema: None,
        extras: BTreeMap::new(),
    };
    let doc = Document {
        frontmatter: fm,
        body: input.body,
    };
    svcs.vault
        .write(&id, &doc)
        .await
        .map_err(|e| format!("write: {e}"))?;
    svcs.index
        .upsert(&input.type_, &doc)
        .await
        .map_err(|e| format!("index upsert: {e}"))?;

    let version = doc.version().map_err(|e| e.to_string())?;
    Ok(DocDetail {
        id: id.to_string(),
        type_: doc.frontmatter.type_.clone(),
        visibility: doc.frontmatter.visibility.as_label().to_string(),
        tags: doc.frontmatter.tags,
        links: doc.frontmatter.links,
        aliases: doc.frontmatter.aliases,
        source: doc.frontmatter.source,
        created: doc.frontmatter.created.map(|d| d.to_string()),
        updated: doc.frontmatter.updated.map(|d| d.to_string()),
        body: doc.body,
        version,
    })
}

#[tauri::command]
pub async fn doc_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let svcs = state.active_services().await?;
    let doc_id = DocumentId::new(id).map_err(|e| e.to_string())?;
    svcs.vault
        .delete(&doc_id)
        .await
        .map_err(|e| format!("delete: {e}"))?;
    svcs.index
        .remove(&doc_id)
        .await
        .map_err(|e| format!("index remove: {e}"))?;
    Ok(())
}

// ---------------- graph ----------------

#[derive(Debug, Serialize)]
pub struct GraphNode {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub title: String,
    pub visibility: String,
}

#[derive(Debug, Serialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Serialize)]
pub struct GraphSnapshot {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[tauri::command]
pub async fn graph_snapshot(state: State<'_, AppState>) -> Result<GraphSnapshot, String> {
    let svcs = state.active_services().await?;
    let items = svcs
        .index
        .list(Default::default())
        .await
        .map_err(|e| format!("list: {e}"))?;

    let mut nodes = Vec::with_capacity(items.len());
    let mut known = std::collections::HashSet::with_capacity(items.len());
    for it in items {
        known.insert(it.id.clone());
        nodes.push(GraphNode {
            id: it.id,
            type_: it.type_,
            title: it.title,
            visibility: it.visibility,
        });
    }

    let raw_edges = svcs
        .index
        .all_edges()
        .await
        .map_err(|e| format!("edges: {e}"))?;

    // Keep only edges where both endpoints are in the current node set.
    // An unresolved target (a link to something not in the vault) would
    // render as a dangling node; skip for v1 clarity.
    let edges = raw_edges
        .into_iter()
        .filter(|(s, t)| known.contains(s) && known.contains(t))
        .map(|(source, target)| GraphEdge { source, target })
        .collect();

    Ok(GraphSnapshot { nodes, edges })
}

// ---------------- tokens ----------------

#[derive(Debug, Serialize)]
pub struct TokenInfo {
    pub id: String,
    pub label: String,
    pub scope: Vec<String>,
    pub mode: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
    pub revoked: bool,
}

#[derive(Debug, Serialize)]
pub struct IssuedTokenDto {
    pub info: TokenInfo,
    /// Shown to the user exactly once.
    pub secret: String,
}

#[derive(Debug, Deserialize)]
pub struct TokenIssueInput {
    pub label: String,
    pub scope: Vec<String>,
    pub mode: String, // "read" | "read_propose"
    pub ttl_days: Option<i64>,
}

#[tauri::command]
pub async fn token_list(state: State<'_, AppState>) -> Result<Vec<TokenInfo>, String> {
    let svcs = state.active_services().await?;
    svcs.require_local("token listing")?;
    let auth = svcs.auth.as_ref().expect("local has auth");
    let tokens = auth.list().await;
    Ok(tokens.iter().map(public_to_info).collect())
}

#[tauri::command]
pub async fn token_issue(
    state: State<'_, AppState>,
    input: TokenIssueInput,
) -> Result<IssuedTokenDto, String> {
    let svcs = state.active_services().await?;
    svcs.require_local("token issuance")?;
    let auth = svcs.auth.as_ref().expect("local has auth");
    let scope = Scope::new(input.scope).map_err(|e| format!("scope: {e}"))?;
    let mode = match input.mode.as_str() {
        "read" => Mode::Read,
        "read_propose" => Mode::ReadPropose,
        other => return Err(format!("unknown mode: {other}")),
    };
    let ttl = input.ttl_days.map(Duration::days);
    let issued = auth
        .issue(IssueRequest {
            label: input.label,
            scope,
            mode,
            limits: Default::default(),
            ttl,
        })
        .await
        .map_err(|e| format!("issue: {e}"))?;
    Ok(IssuedTokenDto {
        info: public_to_info(&issued.info),
        secret: issued.secret.expose().to_string(),
    })
}

#[tauri::command]
pub async fn token_revoke(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let svcs = state.active_services().await?;
    svcs.require_local("token revocation")?;
    let auth = svcs.auth.as_ref().expect("local has auth");
    auth.revoke(&id).await.map_err(|e| format!("revoke: {e}"))
}

// ---------------- audit ----------------

#[derive(Debug, Serialize)]
pub struct AuditRow {
    pub seq: u64,
    pub ts: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub document_id: Option<String>,
    pub scope_used: Vec<String>,
    pub outcome: String,
}

#[derive(Debug, Serialize)]
pub struct AuditPage {
    pub entries: Vec<AuditRow>,
    pub total: u64,
    pub chain_valid: bool,
}

#[tauri::command]
pub async fn audit_list(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<AuditPage, String> {
    let svcs = state.active_services().await?;
    svcs.require_local("audit log")?;
    let path = svcs.root.join(".mytex/audit.jsonl");
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return Ok(AuditPage {
            entries: vec![],
            total: 0,
            chain_valid: true,
        });
    }

    let report = verify(&path).await.ok();
    let chain_valid = report.is_some();
    let total = report.as_ref().map(|r| r.total_entries).unwrap_or(0);

    let mut iter = AuditIter::open(&path).await.map_err(|e| e.to_string())?;
    let mut all = Vec::new();
    while let Some(entry) = iter.next().await.map_err(|e| e.to_string())? {
        all.push(entry);
    }
    // Newest first.
    all.reverse();
    if let Some(n) = limit {
        all.truncate(n);
    }
    let entries = all.into_iter().map(entry_to_row).collect();
    Ok(AuditPage {
        entries,
        total,
        chain_valid,
    })
}

// ---------------- helpers ----------------

fn title_from_body(body: &str, fallback_id: &str) -> String {
    for line in body.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("# ") {
            let s = rest.trim();
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    fallback_id.to_string()
}

fn public_to_info(t: &mytex_auth::PublicTokenInfo) -> TokenInfo {
    TokenInfo {
        id: t.id.clone(),
        label: t.label.clone(),
        scope: t.scope.clone(),
        mode: match t.mode {
            Mode::Read => "read".into(),
            Mode::ReadPropose => "read_propose".into(),
        },
        created_at: t.created_at,
        expires_at: t.expires_at,
        last_used: t.last_used,
        revoked: t.revoked_at.is_some(),
    }
}

// ---------------- settings ----------------

#[derive(Debug, Serialize)]
pub struct SettingsInfo {
    pub has_api_key: bool,
}

#[tauri::command]
pub async fn settings_status(state: State<'_, AppState>) -> Result<SettingsInfo, String> {
    let svcs = state.active_services().await?;
    let s = settings::load(&svcs.root).await?;
    Ok(SettingsInfo {
        has_api_key: s.anthropic_api_key.is_some(),
    })
}

#[tauri::command]
pub async fn settings_set_api_key(
    state: State<'_, AppState>,
    api_key: String,
) -> Result<(), String> {
    let svcs = state.active_services().await?;
    let trimmed = api_key.trim().to_string();
    let mut s = settings::load(&svcs.root).await?;
    s.anthropic_api_key = if trimmed.is_empty() { None } else { Some(trimmed) };
    settings::save(&svcs.root, &s).await
}

// ---------------- onboarding ----------------

#[derive(Debug, Deserialize)]
pub struct OnboardingChatInput {
    pub history: Vec<ChatMessage>,
}

#[derive(Debug, Serialize)]
pub struct OnboardingChatOutput {
    pub reply: String,
}

#[tauri::command]
pub async fn onboarding_chat(
    state: State<'_, AppState>,
    input: OnboardingChatInput,
) -> Result<OnboardingChatOutput, String> {
    let svcs = state.active_services().await?;
    let s = settings::load(&svcs.root).await?;
    let key = s
        .anthropic_api_key
        .ok_or_else(|| "anthropic api key not set".to_string())?;
    let reply = onboarding::chat(&key, onboarding::SYSTEM_PROMPT_CHAT, &input.history).await?;
    Ok(OnboardingChatOutput { reply })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OnboardingSeedDoc {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub visibility: String,
    pub body: String,
}

#[tauri::command]
pub async fn onboarding_finalize(
    state: State<'_, AppState>,
    input: OnboardingChatInput,
) -> Result<Vec<OnboardingSeedDoc>, String> {
    let svcs = state.active_services().await?;
    let s = settings::load(&svcs.root).await?;
    let key = s
        .anthropic_api_key
        .ok_or_else(|| "anthropic api key not set".to_string())?;

    let mut history = input.history;
    history.push(ChatMessage {
        role: "user".into(),
        content:
            "Based on our conversation, return the seed documents now. JSON array only, no prose."
                .into(),
    });

    let raw = onboarding::chat(&key, onboarding::SYSTEM_PROMPT_FINALIZE, &history).await?;
    let json = onboarding::extract_json_array(&raw)
        .ok_or_else(|| format!("could not find JSON array in agent output: {raw}"))?;
    let drafts: Vec<SeedDocDraft> =
        serde_json::from_str(json).map_err(|e| format!("parse seed docs: {e}; raw: {raw}"))?;

    Ok(drafts
        .into_iter()
        .map(|d| OnboardingSeedDoc {
            id: d.id,
            type_: d.type_,
            visibility: d.visibility,
            body: d.body,
        })
        .collect())
}

#[derive(Debug, Deserialize)]
pub struct OnboardingSaveInput {
    pub docs: Vec<OnboardingSeedDoc>,
}

#[tauri::command]
pub async fn onboarding_save(
    state: State<'_, AppState>,
    input: OnboardingSaveInput,
) -> Result<u32, String> {
    let svcs = state.active_services().await?;
    let today = Utc::now().date_naive();
    let mut saved = 0u32;
    for d in input.docs {
        let id = DocumentId::new(d.id.clone()).map_err(|e| format!("{}: {e}", d.id))?;
        let visibility = Visibility::from_label(&d.visibility)
            .map_err(|e| format!("{} visibility: {e}", d.id))?;
        let fm = Frontmatter {
            id: id.clone(),
            type_: d.type_.clone(),
            visibility,
            tags: vec![],
            links: vec![],
            aliases: vec![],
            created: Some(today),
            updated: Some(today),
            source: Some("onboarding".into()),
            principal: None,
            schema: None,
            extras: BTreeMap::new(),
        };
        let doc = Document {
            frontmatter: fm,
            body: d.body,
        };
        svcs.vault
            .write(&id, &doc)
            .await
            .map_err(|e| format!("write {id}: {e}"))?;
        svcs.index
            .upsert(&d.type_, &doc)
            .await
            .map_err(|e| format!("index upsert {id}: {e}"))?;
        saved += 1;
    }
    Ok(saved)
}

// ---------------- helpers ----------------

fn entry_to_row(e: AuditEntry) -> AuditRow {
    AuditRow {
        seq: e.seq,
        ts: e.ts,
        actor: e.actor.as_encoded(),
        action: e.action,
        document_id: e.document_id,
        scope_used: e.scope_used,
        outcome: format!("{:?}", e.outcome).to_lowercase(),
    }
}

