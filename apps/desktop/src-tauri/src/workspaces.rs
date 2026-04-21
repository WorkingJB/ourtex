//! Workspace registry stored at `~/.mytex/workspaces.json`.
//!
//! The registry is per-install client state, not part of the vault
//! format (see FORMAT.md §11.1). It tracks every vault the user has
//! registered with this desktop install and which one is currently
//! active.
//!
//! Phase 2a ships only `kind = "local"` entries. Phase 2b adds
//! `kind = "remote"` for vaults backed by `mytex-server`.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const REGISTRY_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registry {
    pub version: u32,
    #[serde(default)]
    pub active: Option<String>,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceEntry>,
}

impl Default for Registry {
    fn default() -> Self {
        Registry {
            version: REGISTRY_VERSION,
            active: None,
            workspaces: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub id: String,
    pub name: String,
    /// `"local"` | `"remote"`.
    pub kind: String,
    /// Filesystem root for local workspaces. For remote entries this is
    /// the *cache* root under `~/.mytex/remote/<workspace_id>/` — the
    /// frontend displays the server URL instead.
    pub path: PathBuf,
    pub added_at: DateTime<Utc>,

    // --- remote-only fields (Phase 2b.2) ---
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_email: Option<String>,
    /// Session bearer token. Stored in plaintext inside the registry
    /// file for now (same threat model as `.mytex/settings.json`).
    /// Known gap: should move to the OS keychain in 2b.3 with the
    /// crypto + unlock flow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_expires_at: Option<DateTime<Utc>>,
}

impl Registry {
    /// Load from disk, or return a fresh empty registry if the file
    /// does not exist yet. Any error reading a file that *does* exist
    /// propagates up — we do not silently clobber a corrupted registry.
    pub async fn load(path: &Path) -> Result<Self, String> {
        match tokio::fs::read(path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("parse {}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
            Err(e) => Err(format!("read {}: {e}", path.display())),
        }
    }

    /// Atomic write via temp + rename, so a crash mid-write cannot leave
    /// a torn JSON file.
    pub async fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| format!("serialize: {e}"))?;
        tokio::fs::write(&tmp, &bytes)
            .await
            .map_err(|e| format!("write {}: {e}", tmp.display()))?;
        tokio::fs::rename(&tmp, path)
            .await
            .map_err(|e| format!("rename {}: {e}", path.display()))?;
        Ok(())
    }

    pub fn find(&self, id: &str) -> Option<&WorkspaceEntry> {
        self.workspaces.iter().find(|w| w.id == id)
    }

    pub fn is_active(&self, id: &str) -> bool {
        self.active.as_deref() == Some(id)
    }

    /// Add a new workspace. If another workspace already points at the
    /// same canonical path, returns that existing entry instead of
    /// creating a duplicate.
    pub fn add_local(&mut self, name: String, path: PathBuf) -> &WorkspaceEntry {
        if let Some(pos) = self.workspaces.iter().position(|w| w.path == path) {
            return &self.workspaces[pos];
        }
        let entry = WorkspaceEntry {
            id: generate_workspace_id(),
            name,
            kind: "local".into(),
            path,
            added_at: Utc::now(),
            server_url: None,
            tenant_id: None,
            account_email: None,
            session_token: None,
            session_expires_at: None,
        };
        self.workspaces.push(entry);
        self.workspaces.last().unwrap()
    }

    /// Register a remote workspace. Dedupes on (server_url, tenant_id).
    #[allow(clippy::too_many_arguments)]
    pub fn add_remote(
        &mut self,
        name: String,
        cache_root: PathBuf,
        server_url: String,
        tenant_id: Uuid,
        account_email: String,
        session_token: String,
        session_expires_at: DateTime<Utc>,
    ) -> &WorkspaceEntry {
        if let Some(pos) = self.workspaces.iter().position(|w| {
            w.kind == "remote"
                && w.server_url.as_deref() == Some(server_url.as_str())
                && w.tenant_id == Some(tenant_id)
        }) {
            // Refresh the session token on re-registration.
            self.workspaces[pos].session_token = Some(session_token);
            self.workspaces[pos].session_expires_at = Some(session_expires_at);
            self.workspaces[pos].account_email = Some(account_email);
            return &self.workspaces[pos];
        }
        let entry = WorkspaceEntry {
            id: generate_workspace_id(),
            name,
            kind: "remote".into(),
            path: cache_root,
            added_at: Utc::now(),
            server_url: Some(server_url),
            tenant_id: Some(tenant_id),
            account_email: Some(account_email),
            session_token: Some(session_token),
            session_expires_at: Some(session_expires_at),
        };
        self.workspaces.push(entry);
        self.workspaces.last().unwrap()
    }

    pub fn remove(&mut self, id: &str) -> Option<WorkspaceEntry> {
        let pos = self.workspaces.iter().position(|w| w.id == id)?;
        let removed = self.workspaces.remove(pos);
        if self.active.as_deref() == Some(id) {
            // Promote the first remaining workspace (most recently added
            // is last; first is the original). Callers can reactivate
            // anything they prefer.
            self.active = self.workspaces.first().map(|w| w.id.clone());
        }
        Some(removed)
    }

    pub fn rename(&mut self, id: &str, name: String) -> Result<(), String> {
        let entry = self
            .workspaces
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or_else(|| format!("unknown workspace: {id}"))?;
        entry.name = name;
        Ok(())
    }

    pub fn set_active(&mut self, id: &str) -> Result<(), String> {
        if !self.workspaces.iter().any(|w| w.id == id) {
            return Err(format!("unknown workspace: {id}"));
        }
        self.active = Some(id.to_string());
        Ok(())
    }

    pub fn active_entry(&self) -> Option<&WorkspaceEntry> {
        let id = self.active.as_deref()?;
        self.find(id)
    }
}

fn generate_workspace_id() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("ws_{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// Default registry location: `~/.mytex/workspaces.json`. If `$HOME` is
/// unset (which should never happen on macOS/Linux/Windows Tauri), falls
/// back to `./.mytex/workspaces.json` in the current working directory
/// so the app is still usable — the user just loses cross-run state.
pub fn default_registry_path() -> PathBuf {
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(".mytex").join("workspaces.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn load_missing_returns_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("workspaces.json");
        let r = Registry::load(&path).await.unwrap();
        assert_eq!(r.version, REGISTRY_VERSION);
        assert!(r.active.is_none());
        assert!(r.workspaces.is_empty());
    }

    #[tokio::test]
    async fn roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("workspaces.json");
        let mut r = Registry::default();
        let id = r
            .add_local("Personal".into(), PathBuf::from("/tmp/mytex-personal"))
            .id
            .clone();
        r.set_active(&id).unwrap();
        r.save(&path).await.unwrap();

        let r2 = Registry::load(&path).await.unwrap();
        assert_eq!(r2.workspaces.len(), 1);
        assert_eq!(r2.active.as_deref(), Some(id.as_str()));
        assert_eq!(r2.workspaces[0].kind, "local");
    }

    #[test]
    fn add_local_dedupes_on_path() {
        let mut r = Registry::default();
        let a = r.add_local("A".into(), PathBuf::from("/x")).id.clone();
        let b = r.add_local("B".into(), PathBuf::from("/x")).id.clone();
        assert_eq!(a, b, "second add for same path must return existing id");
        assert_eq!(r.workspaces.len(), 1);
    }

    #[test]
    fn remove_active_promotes_first_remaining() {
        let mut r = Registry::default();
        let a = r.add_local("A".into(), PathBuf::from("/a")).id.clone();
        let b = r.add_local("B".into(), PathBuf::from("/b")).id.clone();
        r.set_active(&b).unwrap();
        r.remove(&b);
        assert_eq!(r.active.as_deref(), Some(a.as_str()));
    }
}
