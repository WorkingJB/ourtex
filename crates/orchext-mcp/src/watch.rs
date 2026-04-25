//! Filesystem watcher → live re-index + `notifications/resources/updated`.
//!
//! Runs a std::thread holding the sync `notify::RecommendedWatcher`
//! receiver. Each relevant event is turned into (type, id) and routed
//! back into tokio via the runtime handle: we reindex the document in
//! place (read on create/modify, remove on delete) and, after the index
//! is consistent, ask the server to emit the MCP notification so
//! subscribed clients re-read fresh data.

use crate::server::Server;
use ourtex_vault::DocumentId;
use notify::event::EventKind;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::runtime::Handle;

pub struct WatcherHandle {
    _watcher: RecommendedWatcher,
}

pub fn spawn(vault_root: PathBuf, server: Arc<Server>) -> Result<WatcherHandle, notify::Error> {
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;
    watcher.watch(&vault_root, RecursiveMode::Recursive)?;

    let runtime = Handle::current();
    let root = vault_root;
    std::thread::Builder::new()
        .name("ourtex-mcp-watch".into())
        .spawn(move || {
            while let Ok(res) = rx.recv() {
                match res {
                    Ok(event) => {
                        if !is_relevant_kind(&event.kind) {
                            continue;
                        }
                        for path in event.paths {
                            if let Some((type_, id)) = classify(&root, &path) {
                                let server = server.clone();
                                runtime.spawn(async move {
                                    apply_and_notify(&server, &type_, &id).await;
                                });
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(err = %e, "fs watcher error");
                    }
                }
            }
        })
        .expect("spawn watcher thread");

    Ok(WatcherHandle { _watcher: watcher })
}

async fn apply_and_notify(server: &Server, type_: &str, id: &str) {
    let Ok(doc_id) = DocumentId::new(id) else {
        tracing::debug!(id, "skipping watcher event with invalid id");
        return;
    };

    let vault = server.vault();
    let index = server.index();

    match vault.read(&doc_id).await {
        Ok(doc) => {
            if let Err(e) = index.upsert(type_, &doc).await {
                tracing::warn!(err = %e, id, "watcher upsert failed");
            }
        }
        Err(_) => {
            // Missing file → remove from index. `remove` is idempotent.
            if let Err(e) = index.remove(&doc_id).await {
                tracing::warn!(err = %e, id, "watcher remove failed");
            }
        }
    }

    let uri = format!("ourtex://vault/{}/{}", type_, id);
    server.emit_resource_updated(&uri);
}

fn is_relevant_kind(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

/// Map a filesystem path under the vault root to a `(type, id)` pair,
/// or `None` if the path isn't a document we care about.
///
/// Valid shape: `<root>/<type>/<id>.md`. Anything under `.ourtex/`, a
/// dotfile, or deeper nesting is ignored — this matches `PlainFileDriver`
/// rules so the index stays in sync with what `list()` returns.
fn classify(root: &Path, path: &Path) -> Option<(String, String)> {
    let rel = path.strip_prefix(root).ok()?;
    let components: Vec<_> = rel.components().collect();
    if components.len() != 2 {
        return None;
    }
    let type_name = components[0].as_os_str().to_str()?;
    if type_name.starts_with('.') {
        return None;
    }
    let file = components[1].as_os_str().to_str()?;
    let id = file.strip_suffix(".md")?;
    if id.is_empty() {
        return None;
    }
    Some((type_name.to_string(), id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_doc_path() {
        let root = Path::new("/vault");
        assert_eq!(
            classify(root, Path::new("/vault/relationships/rel-jane.md")),
            Some(("relationships".into(), "rel-jane".into()))
        );
    }

    #[test]
    fn skips_dot_ourtex() {
        let root = Path::new("/vault");
        assert!(classify(root, Path::new("/vault/.ourtex/audit.jsonl")).is_none());
    }

    #[test]
    fn skips_deep_nesting() {
        let root = Path::new("/vault");
        assert!(classify(root, Path::new("/vault/x/y/z.md")).is_none());
    }

    #[test]
    fn skips_non_md() {
        let root = Path::new("/vault");
        assert!(classify(root, Path::new("/vault/relationships/rel-jane.txt")).is_none());
    }
}
