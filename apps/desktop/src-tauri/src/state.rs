//! Tauri-managed state: the workspace registry plus the currently-
//! active open vault.
//!
//! Phase 2a model: the registry tracks N workspaces; at any moment at
//! most one is *open* (its services loaded, watcher running). Switching
//! workspaces drops the previous `OpenVault` and opens a new one. This
//! is a deliberate simplification — keeping every workspace warm would
//! require N watchers, N indices in memory, and a coordination story
//! for the fs-watcher event channel, none of which is worth it at v1
//! vault sizes.
//!
//! Phase 2b.2 extends `OpenVault` to handle `kind = "remote"`. The
//! `vault` handle becomes a `RemoteVaultDriver` instead of the local
//! `PlainFileDriver`, and the local `Index` is used as a read cache
//! populated via `reindex_from`. The fs watcher and the local-only
//! `TokenService` / `AuditWriter` aren't meaningful for remote
//! workspaces, so those handles are `None` — commands that require
//! them surface a clear "not supported on remote workspaces" error.

use crate::watch::WatcherHandle;
use crate::workspaces::{self, Registry, WorkspaceEntry};
use mytex_audit::AuditWriter;
use mytex_auth::TokenService;
use mytex_index::Index;
use mytex_sync::{RemoteClient, RemoteConfig, RemoteVaultDriver};
use mytex_vault::{PlainFileDriver, VaultDriver};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct OpenVault {
    pub workspace_id: String,
    /// `"local"` | `"remote"`. Propagated from the registry entry.
    pub kind: String,
    pub root: PathBuf,
    pub vault: Arc<dyn VaultDriver>,
    pub index: Arc<Index>,
    /// Local token store. `None` for remote workspaces — those use
    /// the server's `/v1/t/:tid/tokens` endpoints, which live behind
    /// `token_*` commands when we wire them in a follow-up.
    pub auth: Option<Arc<TokenService>>,
    /// Local audit log. `None` for remote workspaces — the server
    /// owns the per-tenant chain and exposes `/v1/t/:tid/audit`.
    pub audit: Option<Arc<AuditWriter>>,
    /// Kept alive so the notify watcher thread doesn't exit. Replaced
    /// on each workspace switch (switching drops the old one). `None`
    /// for remote workspaces — there's no local filesystem to watch.
    pub _watcher: Option<WatcherHandle>,
    /// Remote-only: the control-plane HTTP client. `None` for local
    /// workspaces. Cloned into `workspace_unlock` so the command can
    /// talk to `/vault/crypto` and `/session-key` without downcasting
    /// the `Arc<dyn VaultDriver>`.
    pub remote_client: Option<Arc<RemoteClient>>,
    /// Remote-only: background task that re-publishes the content
    /// key before the server's TTL lapses. `None` when the workspace
    /// is locked (or when the tenant hasn't seeded crypto at all).
    /// Dropping the `OpenVault` aborts the task via `JoinHandle`.
    pub heartbeat: Option<HeartbeatHandle>,
}

/// Wraps the heartbeat task's join handle so `OpenVault` drop cleanly
/// cancels the loop.
pub struct HeartbeatHandle(tokio::task::JoinHandle<()>);

impl HeartbeatHandle {
    pub fn spawn(client: Arc<RemoteClient>, content_key_wire: String) -> Self {
        // Refresh at ~1/4 of the server's default 15-minute TTL so a
        // single missed publish doesn't lock the workspace.
        let interval = std::time::Duration::from_secs(4 * 60);
        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                match client.publish_session_key(&content_key_wire).await {
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(err = %e, "session-key heartbeat failed; stopping");
                        break;
                    }
                }
            }
        });
        Self(handle)
    }
}

impl Drop for HeartbeatHandle {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub struct AppState {
    registry_path: PathBuf,
    registry: RwLock<Registry>,
    open: RwLock<Option<OpenVault>>,
}

impl AppState {
    pub async fn new(registry_path: PathBuf) -> Result<Self, String> {
        let registry = Registry::load(&registry_path).await?;
        Ok(AppState {
            registry_path,
            registry: RwLock::new(registry),
            open: RwLock::new(None),
        })
    }

    pub async fn registry_snapshot(&self) -> Registry {
        self.registry.read().await.clone()
    }

    /// Apply a mutation to the registry, then persist atomically. The
    /// mutation runs under the write lock so concurrent callers can't
    /// race. Saves the registry to disk before releasing the lock so a
    /// subsequent `registry_snapshot` always reflects what's on disk.
    pub async fn mutate_registry<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&mut Registry) -> Result<T, String>,
    {
        let mut g = self.registry.write().await;
        let out = f(&mut g)?;
        g.save(&self.registry_path).await?;
        Ok(out)
    }

    pub async fn is_active_open(&self, id: &str) -> bool {
        self.open
            .read()
            .await
            .as_ref()
            .map(|v| v.workspace_id == id)
            .unwrap_or(false)
    }

    /// Swap in a fresh `OpenVault`, dropping any previous one. Dropping
    /// the old `OpenVault` tears down its watcher.
    pub async fn set_open(&self, v: OpenVault) {
        *self.open.write().await = Some(v);
    }

    pub async fn clear_open(&self) {
        *self.open.write().await = None;
    }

    pub async fn active_services(&self) -> Result<Services, String> {
        let guard = self.open.read().await;
        let v = guard
            .as_ref()
            .ok_or_else(|| "no workspace open".to_string())?;
        Ok(Services {
            workspace_id: v.workspace_id.clone(),
            kind: v.kind.clone(),
            root: v.root.clone(),
            vault: v.vault.clone(),
            index: v.index.clone(),
            auth: v.auth.clone(),
            audit: v.audit.clone(),
            remote_client: v.remote_client.clone(),
        })
    }

    /// Install or replace the heartbeat task for the currently-open
    /// remote workspace. Used by `workspace_unlock` after it
    /// successfully publishes a key. Dropping any previous handle
    /// aborts the prior heartbeat.
    pub async fn set_heartbeat(&self, h: Option<HeartbeatHandle>) {
        if let Some(v) = self.open.write().await.as_mut() {
            v.heartbeat = h;
        }
    }
}

/// A snapshot of the handles needed to serve a single command, cloned
/// out from under the state lock. Cloning `Arc`s is cheap; this lets
/// commands do long-running work without holding the state lock.
pub struct Services {
    #[allow(dead_code)]
    pub workspace_id: String,
    pub kind: String,
    pub root: PathBuf,
    pub vault: Arc<dyn VaultDriver>,
    pub index: Arc<Index>,
    pub auth: Option<Arc<TokenService>>,
    #[allow(dead_code)]
    pub audit: Option<Arc<AuditWriter>>,
    /// Remote-only: control-plane client for crypto + session-key
    /// endpoints. `None` for local workspaces.
    pub remote_client: Option<Arc<RemoteClient>>,
}

impl Services {
    pub fn is_remote(&self) -> bool {
        self.kind == "remote"
    }

    /// Shorthand for commands that only make sense on local
    /// workspaces. The error message points the user at what's missing
    /// so the UX isn't opaque.
    pub fn require_local(&self, feature: &str) -> Result<(), String> {
        if self.is_remote() {
            Err(format!(
                "{feature} is not yet wired through the server for remote workspaces (Phase 2b.2 follow-up)"
            ))
        } else {
            Ok(())
        }
    }
}

/// Build the full service stack for a workspace. Dispatches on kind.
pub async fn open_workspace(entry: &WorkspaceEntry) -> Result<OpenVault, String> {
    match entry.kind.as_str() {
        "local" => open_local(entry).await,
        "remote" => open_remote(entry).await,
        other => Err(format!("unsupported workspace kind: {other}")),
    }
}

async fn open_local(entry: &WorkspaceEntry) -> Result<OpenVault, String> {
    let root = entry.path.clone();
    // Canonicalize so fs-watch paths line up (matches mytex-mcp's
    // behavior on macOS where `/tmp` is a symlink).
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| format!("create vault dir: {e}"))?;
    let root = root
        .canonicalize()
        .map_err(|e| format!("canonicalize: {e}"))?;

    let mytex_dir = root.join(".mytex");
    tokio::fs::create_dir_all(&mytex_dir)
        .await
        .map_err(|e| format!("create .mytex: {e}"))?;

    // Seed type directories so an empty vault still has a navigable
    // shape for the UI. Matches `mytex-mcp init`.
    for t in SEED_TYPES {
        tokio::fs::create_dir_all(root.join(t))
            .await
            .map_err(|e| format!("create {t}: {e}"))?;
    }

    let vault: Arc<dyn VaultDriver> = Arc::new(PlainFileDriver::new(root.clone()));
    let index = Arc::new(
        Index::open(mytex_dir.join("index.sqlite"))
            .await
            .map_err(|e| format!("open index: {e}"))?,
    );
    index
        .reindex_from(&*vault)
        .await
        .map_err(|e| format!("reindex: {e}"))?;
    let auth = Arc::new(
        TokenService::open(mytex_dir.join("tokens.json"))
            .await
            .map_err(|e| format!("open tokens: {e}"))?,
    );
    let audit = Arc::new(
        AuditWriter::open(mytex_dir.join("audit.jsonl"))
            .await
            .map_err(|e| format!("open audit: {e}"))?,
    );

    Ok(OpenVault {
        workspace_id: entry.id.clone(),
        kind: entry.kind.clone(),
        root,
        vault,
        index,
        auth: Some(auth),
        audit: Some(audit),
        _watcher: None,
        remote_client: None,
        heartbeat: None,
    })
}

async fn open_remote(entry: &WorkspaceEntry) -> Result<OpenVault, String> {
    let server_url = entry
        .server_url
        .as_ref()
        .ok_or_else(|| "remote workspace missing server_url".to_string())?;
    let tenant_id = entry
        .tenant_id
        .ok_or_else(|| "remote workspace missing tenant_id".to_string())?;
    let session_token = entry
        .session_token
        .clone()
        .ok_or_else(|| "remote workspace has no session token; reconnect".to_string())?;
    let server_url: url::Url = server_url
        .parse()
        .map_err(|e| format!("invalid server_url {server_url:?}: {e}"))?;

    let cache_root = entry.path.clone();
    tokio::fs::create_dir_all(&cache_root)
        .await
        .map_err(|e| format!("create cache dir {}: {e}", cache_root.display()))?;

    let client = Arc::new(RemoteClient::new(RemoteConfig {
        server_url,
        tenant_id,
        session_token,
    }));
    let vault: Arc<dyn VaultDriver> =
        Arc::new(RemoteVaultDriver::new((*client).clone()));

    let index_path = cache_root.join("index.sqlite");
    let index = Arc::new(
        Index::open(&index_path)
            .await
            .map_err(|e| format!("open remote cache index: {e}"))?,
    );
    // Reindex on open, but tolerate `vault_locked` — a remote tenant
    // with seeded crypto has no readable documents until the user
    // unlocks. Logging the failure is enough; the index stays empty
    // until a successful unlock triggers a fresh reindex.
    if let Err(e) = index.reindex_from(&*vault).await {
        tracing::warn!(err = %e, "initial reindex failed; workspace may be locked");
    }

    Ok(OpenVault {
        workspace_id: entry.id.clone(),
        kind: entry.kind.clone(),
        root: cache_root,
        vault,
        index,
        auth: None,
        audit: None,
        _watcher: None,
        remote_client: Some(client),
        heartbeat: None,
    })
}

/// Convenience constructor that uses the registry's default path.
pub async fn default_state() -> Result<AppState, String> {
    AppState::new(workspaces::default_registry_path()).await
}

const SEED_TYPES: &[&str] = &[
    "identity",
    "roles",
    "goals",
    "relationships",
    "memories",
    "tools",
    "preferences",
    "domains",
    "decisions",
    "attachments",
];
