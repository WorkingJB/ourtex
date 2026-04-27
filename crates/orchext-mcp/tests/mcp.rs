use chrono::NaiveDate;
use orchext_audit::AuditWriter;
use orchext_auth::{IssueRequest, Mode, Scope, TokenService};
use orchext_index::Index;
use orchext_mcp::rpc::{Notification, Request};
use orchext_mcp::Server;
use orchext_vault::{Document, DocumentId, Frontmatter, PlainFileDriver, VaultDriver, Visibility};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::mpsc;

// ---------- test fixtures ----------

fn doc(id: &str, type_: &str, vis: Visibility, title: &str, body_rest: &str, tags: &[&str], source: Option<&str>) -> Document {
    let fm = Frontmatter {
        id: DocumentId::new(id).unwrap(),
        type_: type_.to_string(),
        visibility: vis,
        tags: tags.iter().map(|s| s.to_string()).collect(),
        links: vec![],
        aliases: vec![],
        created: None,
        updated: Some(NaiveDate::from_ymd_opt(2026, 4, 18).unwrap()),
        source: source.map(|s| s.to_string()),
        principal: None,
        schema: None,
        extras: BTreeMap::new(),
    };
    Document {
        frontmatter: fm,
        body: format!("# {title}\n\n{body_rest}\n"),
    }
}

struct Fixture {
    _tmp: TempDir,
    server: Server,
    audit_path: std::path::PathBuf,
}

async fn fixture(scope_labels: &[&str]) -> Fixture {
    fixture_with_notifier(scope_labels, None).await
}

async fn fixture_with_notifier(
    scope_labels: &[&str],
    notifier: Option<mpsc::UnboundedSender<Notification>>,
) -> Fixture {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    let vault = Arc::new(PlainFileDriver::new(root.clone()));

    vault
        .write(
            &DocumentId::new("rel-jane").unwrap(),
            &doc(
                "rel-jane",
                "relationships",
                Visibility::Work,
                "Jane Smith",
                "My manager at Acme. Prefers written updates.",
                &["manager", "acme"],
                Some("onboarding-interview-2026-01"),
            ),
        )
        .await
        .unwrap();
    vault
        .write(
            &DocumentId::new("pref-comms").unwrap(),
            &doc(
                "pref-comms",
                "preferences",
                Visibility::Work,
                "Communication style",
                "Prefer written async updates.",
                &["style"],
                None,
            ),
        )
        .await
        .unwrap();
    vault
        .write(
            &DocumentId::new("diary-0001").unwrap(),
            &doc(
                "diary-0001",
                "memories",
                Visibility::Private,
                "Diary",
                "Private thoughts about manager and acme.",
                &["journal"],
                None,
            ),
        )
        .await
        .unwrap();
    vault
        .write(
            &DocumentId::new("me-identity").unwrap(),
            &doc(
                "me-identity",
                "identity",
                Visibility::Personal,
                "About me",
                "Personal (not private) background.",
                &[],
                None,
            ),
        )
        .await
        .unwrap();

    let orchext_dir = root.join(".orchext");
    let index = Arc::new(Index::open(orchext_dir.join("index.sqlite")).await.unwrap());
    index.reindex_from(&*vault).await.unwrap();

    let auth = Arc::new(TokenService::open(orchext_dir.join("tokens.json")).await.unwrap());
    let audit_path = orchext_dir.join("audit.jsonl");
    let audit = Arc::new(AuditWriter::open(&audit_path).await.unwrap());

    let scope = Scope::new(scope_labels.iter().map(|s| s.to_string())).unwrap();
    let issued = auth
        .issue(IssueRequest {
            label: "test".into(),
            scope,
            mode: Mode::Read,
            limits: Default::default(),
            ttl: None,
        })
        .await
        .unwrap();
    let token = auth.authenticate(issued.secret.expose()).await.unwrap();

    let vault_arc: Arc<dyn VaultDriver> = vault;
    let mut server = Server::new(vault_arc, index, auth, audit, token);
    if let Some(tx) = notifier {
        server = server.with_notifier(tx);
    }
    Fixture {
        _tmp: tmp,
        server,
        audit_path,
    }
}

/// Spin up a server wired with `Mode::ReadPropose` and a proposals
/// spool dir under the temp vault root. Returns the spool path so tests
/// can assert on the dropped JSON files.
async fn fixture_propose(scope_labels: &[&str]) -> (Fixture, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    let vault = Arc::new(PlainFileDriver::new(root.clone()));
    vault
        .write(
            &DocumentId::new("rel-jane").unwrap(),
            &doc(
                "rel-jane",
                "relationships",
                Visibility::Work,
                "Jane Smith",
                "My manager at Acme.",
                &["manager"],
                None,
            ),
        )
        .await
        .unwrap();

    let orchext_dir = root.join(".orchext");
    let proposals_dir = orchext_dir.join("proposals");
    tokio::fs::create_dir_all(&proposals_dir).await.unwrap();
    let index = Arc::new(Index::open(orchext_dir.join("index.sqlite")).await.unwrap());
    index.reindex_from(&*vault).await.unwrap();
    let auth = Arc::new(TokenService::open(orchext_dir.join("tokens.json")).await.unwrap());
    let audit_path = orchext_dir.join("audit.jsonl");
    let audit = Arc::new(AuditWriter::open(&audit_path).await.unwrap());
    let scope = Scope::new(scope_labels.iter().map(|s| s.to_string())).unwrap();
    let issued = auth
        .issue(IssueRequest {
            label: "agent".into(),
            scope,
            mode: Mode::ReadPropose,
            limits: Default::default(),
            ttl: None,
        })
        .await
        .unwrap();
    let token = auth.authenticate(issued.secret.expose()).await.unwrap();
    let vault_arc: Arc<dyn VaultDriver> = vault;
    let server = Server::new(vault_arc, index, auth, audit, token)
        .with_proposals_dir(proposals_dir.clone());
    (
        Fixture {
            _tmp: tmp,
            server,
            audit_path,
        },
        proposals_dir,
    )
}

fn req(id: i64, method: &str, params: Value) -> Request {
    serde_json::from_value(json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    }))
    .unwrap()
}

async fn call(server: &Server, method: &str, params: Value) -> Value {
    let resp = server.handle(req(1, method, params)).await.unwrap();
    let v = serde_json::to_value(&resp).unwrap();
    assert!(
        v.get("error").is_none(),
        "expected ok, got error: {}",
        v["error"]
    );
    v["result"].clone()
}

async fn call_err(server: &Server, method: &str, params: Value) -> Value {
    let resp = server.handle(req(1, method, params)).await.unwrap();
    let v = serde_json::to_value(&resp).unwrap();
    assert!(
        v.get("result").is_none(),
        "expected error, got result: {}",
        v["result"]
    );
    v["error"].clone()
}

fn tool_call(tool: &str, args: Value) -> Value {
    json!({ "name": tool, "arguments": args })
}

// ---------- tests ----------

#[tokio::test]
async fn initialize_advertises_capabilities() {
    let fx = fixture(&["work", "public"]).await;
    let result = call(&fx.server, "initialize", json!({})).await;
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert_eq!(result["serverInfo"]["name"], "orchext");
    assert_eq!(result["capabilities"]["tools"]["listChanged"], true);
    assert_eq!(result["capabilities"]["resources"]["subscribe"], true);
}

#[tokio::test]
async fn tools_list_returns_context_namespace() {
    let fx = fixture(&["work"]).await;
    let result = call(&fx.server, "tools/list", json!({})).await;
    let tools = result["tools"].as_array().unwrap();
    let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"context_search"));
    assert!(names.contains(&"context_get"));
    assert!(names.contains(&"context_list"));
}

#[tokio::test]
async fn notifications_do_not_respond() {
    let fx = fixture(&["work"]).await;
    // Notification has no `id`.
    let req: Request = serde_json::from_value(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }))
    .unwrap();
    assert!(fx.server.handle(req).await.is_none());
}

#[tokio::test]
async fn search_returns_in_scope_and_not_private() {
    let fx = fixture(&["work", "public"]).await;
    let result = call(
        &fx.server,
        "tools/call",
        tool_call("context_search", json!({ "query": "manager" })),
    )
    .await;
    let structured = &result["structuredContent"];
    let results = structured["results"].as_array().unwrap();
    assert!(!results.is_empty());
    for hit in results {
        assert_ne!(hit["visibility"], "private");
        assert_ne!(hit["visibility"], "personal");
    }
    // Provenance for rel-jane includes source.
    let jane = results
        .iter()
        .find(|h| h["id"] == "rel-jane")
        .expect("rel-jane should be in results");
    assert_eq!(jane["source"], "onboarding-interview-2026-01");
}

#[tokio::test]
async fn search_private_floor_requires_explicit_private() {
    // Without `private` in scope, a private doc that matches the body MUST
    // NOT surface. This is MCP.md §3.2's hard floor.
    let fx = fixture(&["work", "public", "personal"]).await;
    let result = call(
        &fx.server,
        "tools/call",
        tool_call("context_search", json!({ "query": "Private thoughts" })),
    )
    .await;
    let results = result["structuredContent"]["results"].as_array().unwrap();
    assert!(
        results.iter().all(|h| h["id"] != "diary-0001"),
        "private doc should not appear without `private` scope"
    );

    // Same query with `private` in scope surfaces it.
    let fx2 = fixture(&["work", "private"]).await;
    let result = call(
        &fx2.server,
        "tools/call",
        tool_call("context_search", json!({ "query": "Private thoughts" })),
    )
    .await;
    let results = result["structuredContent"]["results"].as_array().unwrap();
    assert!(results.iter().any(|h| h["id"] == "diary-0001"));
}

#[tokio::test]
async fn get_returns_document_in_scope() {
    let fx = fixture(&["work"]).await;
    let result = call(
        &fx.server,
        "tools/call",
        tool_call("context_get", json!({ "id": "rel-jane" })),
    )
    .await;
    let structured = &result["structuredContent"];
    assert_eq!(structured["id"], "rel-jane");
    assert_eq!(structured["type"], "relationships");
    assert!(structured["body"].as_str().unwrap().contains("Jane Smith"));
    assert!(structured["version"].as_str().unwrap().starts_with("sha256:"));
}

#[tokio::test]
async fn get_out_of_scope_is_not_authorized() {
    let fx = fixture(&["work"]).await;
    // diary-0001 is private; token has no `private`.
    let err = call_err(
        &fx.server,
        "tools/call",
        tool_call("context_get", json!({ "id": "diary-0001" })),
    )
    .await;
    assert_eq!(err["code"], -32002);
    assert_eq!(err["data"]["tag"], "not_authorized");
}

#[tokio::test]
async fn get_nonexistent_is_indistinguishable_from_out_of_scope() {
    let fx = fixture(&["work", "private"]).await;
    let err = call_err(
        &fx.server,
        "tools/call",
        tool_call("context_get", json!({ "id": "does-not-exist" })),
    )
    .await;
    assert_eq!(err["code"], -32002);
    assert_eq!(err["data"]["tag"], "not_authorized");
}

#[tokio::test]
async fn list_excludes_out_of_scope_and_private() {
    let fx = fixture(&["work", "personal"]).await;
    let result = call(
        &fx.server,
        "tools/call",
        tool_call("context_list", json!({})),
    )
    .await;
    let items = result["structuredContent"]["items"].as_array().unwrap();
    let ids: Vec<_> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"rel-jane"));
    assert!(ids.contains(&"pref-comms"));
    assert!(ids.contains(&"me-identity"));
    assert!(!ids.contains(&"diary-0001"), "private doc must not be listed");
}

#[tokio::test]
async fn search_rejects_widening_scope_argument() {
    // Token has only `work`; passing ["private"] tries to widen and must fail.
    let fx = fixture(&["work"]).await;
    let err = call_err(
        &fx.server,
        "tools/call",
        tool_call(
            "context_search",
            json!({ "query": "anything", "scope": ["private"] }),
        ),
    )
    .await;
    assert_eq!(err["code"], -32004);
    assert_eq!(err["data"]["tag"], "invalid_argument");
}

#[tokio::test]
async fn search_validates_query_len() {
    let fx = fixture(&["work"]).await;
    let err = call_err(
        &fx.server,
        "tools/call",
        tool_call("context_search", json!({ "query": "" })),
    )
    .await;
    assert_eq!(err["code"], -32004);
}

#[tokio::test]
async fn resources_list_filters_by_scope() {
    let fx = fixture(&["work"]).await;
    let result = call(&fx.server, "resources/list", json!({})).await;
    let resources = result["resources"].as_array().unwrap();
    let uris: Vec<_> = resources
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    assert!(uris.iter().any(|u| u.ends_with("/rel-jane")));
    assert!(uris.iter().any(|u| u.ends_with("/pref-comms")));
    // personal + private are out of scope for a work-only token.
    assert!(!uris.iter().any(|u| u.ends_with("/diary-0001")));
    assert!(!uris.iter().any(|u| u.ends_with("/me-identity")));
}

#[tokio::test]
async fn resources_read_returns_two_content_items() {
    let fx = fixture(&["work"]).await;
    let result = call(
        &fx.server,
        "resources/read",
        json!({ "uri": "orchext://vault/relationships/rel-jane" }),
    )
    .await;
    let contents = result["contents"].as_array().unwrap();
    assert_eq!(contents.len(), 2);
    let mimes: Vec<_> = contents
        .iter()
        .map(|c| c["mimeType"].as_str().unwrap())
        .collect();
    assert!(mimes.contains(&"text/yaml"));
    assert!(mimes.contains(&"text/markdown"));
}

#[tokio::test]
async fn resources_read_out_of_scope_denies() {
    let fx = fixture(&["work"]).await;
    let err = call_err(
        &fx.server,
        "resources/read",
        json!({ "uri": "orchext://vault/memories/diary-0001" }),
    )
    .await;
    assert_eq!(err["code"], -32002);
}

#[tokio::test]
async fn audit_log_grows_per_call() {
    let fx = fixture(&["work"]).await;

    let _ = call(&fx.server, "tools/call", tool_call("context_list", json!({}))).await;
    let _ = call_err(
        &fx.server,
        "tools/call",
        tool_call("context_get", json!({ "id": "diary-0001" })),
    )
    .await;

    let report = orchext_audit::verify(&fx.audit_path).await.unwrap();
    // At least one ok (list) and one denied (get diary-0001).
    assert!(report.total_entries >= 2);
}

#[tokio::test]
async fn method_not_found_returns_jsonrpc_error() {
    let fx = fixture(&["work"]).await;
    let err = call_err(&fx.server, "does/not/exist", json!({})).await;
    assert_eq!(err["code"], -32601);
}

#[tokio::test]
async fn subscribe_then_write_emits_notification() {
    let (tx, mut rx) = mpsc::unbounded_channel::<Notification>();
    let fx = fixture_with_notifier(&["work", "public"], Some(tx)).await;

    // Subscribe to a specific document.
    let _ = call(
        &fx.server,
        "resources/subscribe",
        json!({ "uri": "orchext://vault/relationships/rel-jane" }),
    )
    .await;

    // Simulate the fs watcher firing for that document by driving the
    // server's emitter directly. This is what `watch::apply_and_notify`
    // would do after reindexing on a real fs change.
    fx.server.emit_resource_updated("orchext://vault/relationships/rel-jane");

    let note = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
        .await
        .expect("notification should arrive")
        .expect("channel still open");
    assert_eq!(note.method, "notifications/resources/updated");
    assert_eq!(
        note.params.as_ref().and_then(|p| p.get("uri")).and_then(Value::as_str),
        Some("orchext://vault/relationships/rel-jane")
    );
}

#[tokio::test]
async fn unsubscribed_uri_does_not_fire() {
    let (tx, mut rx) = mpsc::unbounded_channel::<Notification>();
    let fx = fixture_with_notifier(&["work", "public"], Some(tx)).await;
    // No subscribe. Emit should be a silent no-op.
    fx.server.emit_resource_updated("orchext://vault/relationships/rel-jane");
    let got = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await;
    assert!(got.is_err(), "expected timeout, got {got:?}");
}

#[tokio::test]
async fn type_level_subscription_matches_any_doc_in_type() {
    let (tx, mut rx) = mpsc::unbounded_channel::<Notification>();
    let fx = fixture_with_notifier(&["work", "public"], Some(tx)).await;
    let _ = call(
        &fx.server,
        "resources/subscribe",
        json!({ "uri": "orchext://vault/relationships/" }),
    )
    .await;
    fx.server.emit_resource_updated("orchext://vault/relationships/rel-jane");
    let note = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
        .await
        .expect("should arrive")
        .expect("open");
    assert_eq!(note.method, "notifications/resources/updated");
}

#[tokio::test]
async fn unsubscribe_stops_notifications() {
    let (tx, mut rx) = mpsc::unbounded_channel::<Notification>();
    let fx = fixture_with_notifier(&["work", "public"], Some(tx)).await;
    let uri = "orchext://vault/relationships/rel-jane";
    let _ = call(&fx.server, "resources/subscribe", json!({ "uri": uri })).await;
    let _ = call(&fx.server, "resources/unsubscribe", json!({ "uri": uri })).await;
    fx.server.emit_resource_updated(uri);
    let got = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await;
    assert!(got.is_err(), "expected no notification after unsubscribe");
}

#[tokio::test]
async fn rate_limit_kicks_in_above_threshold() {
    // Tight-loop tools/list. Default budget is 60/10s; the 61st call in the
    // same window should return -32005.
    let fx = fixture(&["work"]).await;
    for _ in 0..60 {
        let _ = call(&fx.server, "tools/list", json!({})).await;
    }
    let err = call_err(&fx.server, "tools/list", json!({})).await;
    assert_eq!(err["code"], -32005);
    assert_eq!(err["data"]["tag"], "rate_limited");
    assert!(err["data"]["retry_after_ms"].as_u64().is_some());
}

#[tokio::test]
async fn fs_watcher_reindexes_and_notifies() {
    // End-to-end: spawn the real watcher, write a new doc, confirm the
    // index picks it up and a subscribed URI receives a notification.
    // Built directly (not via `fixture`) so we can own the `Arc<Server>`
    // that `watch::spawn` needs.
    // Canonicalize the temp path: on macOS `/var/folders/...` resolves via
    // a symlink that fsevent sometimes can't follow, silently swallowing
    // the watch events this test depends on.
    let tmp = TempDir::new().unwrap();
    // Pre-create the type directory so we can canonicalize under the root.
    tokio::fs::create_dir_all(tmp.path().join("relationships"))
        .await
        .unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let vault: Arc<dyn VaultDriver> = Arc::new(PlainFileDriver::new(root.clone()));
    vault
        .write(
            &DocumentId::new("rel-existing").unwrap(),
            &doc(
                "rel-existing",
                "relationships",
                Visibility::Work,
                "Existing",
                "starter body",
                &[],
                None,
            ),
        )
        .await
        .unwrap();
    let orchext_dir = root.join(".orchext");
    let index = Arc::new(Index::open(orchext_dir.join("index.sqlite")).await.unwrap());
    index.reindex_from(&*vault).await.unwrap();
    let auth = Arc::new(TokenService::open(orchext_dir.join("tokens.json")).await.unwrap());
    let audit = Arc::new(
        AuditWriter::open(orchext_dir.join("audit.jsonl")).await.unwrap(),
    );
    let scope = Scope::new(["work".to_string()]).unwrap();
    let issued = auth
        .issue(IssueRequest {
            label: "t".into(),
            scope,
            mode: Mode::Read,
            limits: Default::default(),
            ttl: None,
        })
        .await
        .unwrap();
    let token = auth.authenticate(issued.secret.expose()).await.unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel::<Notification>();
    let server = Arc::new(
        Server::new(vault.clone(), index.clone(), auth, audit, token).with_notifier(tx),
    );

    let _watch = orchext_mcp::watch::spawn(root.clone(), server.clone()).unwrap();

    // Subscribe to type-level so any new relationship doc fires.
    let _ = server
        .handle(serde_json::from_value::<Request>(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "resources/subscribe",
            "params": { "uri": "orchext://vault/relationships/" }
        }))
        .unwrap())
        .await;

    // Write a new doc to disk.
    vault
        .write(
            &DocumentId::new("rel-new").unwrap(),
            &doc(
                "rel-new",
                "relationships",
                Visibility::Work,
                "New colleague",
                "matches the search query: colleague",
                &[],
                None,
            ),
        )
        .await
        .unwrap();

    // Wait for the notification (watcher debounce + index upsert).
    let note = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match rx.recv().await {
                Some(n) if n.method == "notifications/resources/updated" => return Some(n),
                Some(_) => continue,
                None => return None,
            }
        }
    })
    .await
    .expect("watcher should fire within 5s")
    .expect("channel open");
    let uri = note.params.as_ref().and_then(|p| p.get("uri")).and_then(Value::as_str);
    assert_eq!(uri, Some("orchext://vault/relationships/rel-new"));

    // And the index has it.
    let items = index
        .list(orchext_index::ListFilter {
            allowed_visibility: vec!["work".to_string()],
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(items.iter().any(|i| i.id == "rel-new"));
}


// ---------- context_propose (stdio) ----------

#[tokio::test]
async fn propose_writes_spool_file_and_leaves_doc_unchanged() {
    let (fx, spool) = fixture_propose(&["work"]).await;

    // Get current version via context_get so the propose carries a
    // fresh base_version (same dance an agent would do).
    let got = call(
        &fx.server,
        "tools/call",
        tool_call("context_get", json!({ "id": "rel-jane" })),
    )
    .await;
    let version = got["structuredContent"]["version"].as_str().unwrap().to_string();

    let result = call(
        &fx.server,
        "tools/call",
        tool_call(
            "context_propose",
            json!({
                "id": "rel-jane",
                "base_version": version,
                "patch": {
                    "frontmatter": { "tags": ["manager", "mentor"] },
                    "body_append": "\n\nMentioned mentoring 2026-04-27."
                },
                "reason": "weekly 1:1"
            }),
        ),
    )
    .await;
    let proposal_id = result["structuredContent"]["proposal_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(result["structuredContent"]["status"], "pending");

    // Spool now has exactly one proposal file with the expected shape.
    let path = spool.join(format!("{proposal_id}.json"));
    let bytes = tokio::fs::read(&path).await.unwrap();
    let payload: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(payload["proposal_id"], proposal_id);
    assert_eq!(payload["doc_id"], "rel-jane");
    assert_eq!(payload["status"], "pending");
    assert_eq!(payload["base_version"], version);
    assert!(payload["patch"]["body_append"].is_string());

    // Doc on disk is untouched: reading it again returns the same
    // version we proposed against.
    let after = call(
        &fx.server,
        "tools/call",
        tool_call("context_get", json!({ "id": "rel-jane" })),
    )
    .await;
    assert_eq!(
        after["structuredContent"]["version"].as_str().unwrap(),
        version
    );
}

#[tokio::test]
async fn propose_rejects_stale_base_version() {
    let (fx, _spool) = fixture_propose(&["work"]).await;
    let err = call_err(
        &fx.server,
        "tools/call",
        tool_call(
            "context_propose",
            json!({
                "id": "rel-jane",
                "base_version": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                "patch": { "body_append": "x" },
                "reason": "stale"
            }),
        ),
    )
    .await;
    assert_eq!(err["code"], -32003);
    assert_eq!(err["data"]["tag"], "version_conflict");
}

#[tokio::test]
async fn propose_without_proposals_dir_is_disabled() {
    // Same setup as fixture_propose but skip with_proposals_dir — the
    // server should refuse the call uniformly with `proposals_disabled`,
    // matching the read-only-token path.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    let vault = Arc::new(PlainFileDriver::new(root.clone()));
    vault
        .write(
            &DocumentId::new("rel-jane").unwrap(),
            &doc(
                "rel-jane",
                "relationships",
                Visibility::Work,
                "Jane",
                ".",
                &[],
                None,
            ),
        )
        .await
        .unwrap();
    let orchext_dir = root.join(".orchext");
    let index = Arc::new(Index::open(orchext_dir.join("index.sqlite")).await.unwrap());
    index.reindex_from(&*vault).await.unwrap();
    let auth = Arc::new(TokenService::open(orchext_dir.join("tokens.json")).await.unwrap());
    let audit = Arc::new(AuditWriter::open(orchext_dir.join("audit.jsonl")).await.unwrap());
    let scope = Scope::new(["work".to_string()]).unwrap();
    let issued = auth
        .issue(IssueRequest {
            label: "agent".into(),
            scope,
            mode: Mode::ReadPropose,
            limits: Default::default(),
            ttl: None,
        })
        .await
        .unwrap();
    let token = auth.authenticate(issued.secret.expose()).await.unwrap();
    let vault_arc: Arc<dyn VaultDriver> = vault;
    let server = Server::new(vault_arc, index, auth, audit, token);

    let err = call_err(
        &server,
        "tools/call",
        tool_call(
            "context_propose",
            json!({
                "id": "rel-jane",
                "base_version": "sha256:0",
                "patch": { "body_append": "x" },
                "reason": "no spool"
            }),
        ),
    )
    .await;
    assert_eq!(err["code"], -32007);
    assert_eq!(err["data"]["tag"], "proposals_disabled");
}

#[tokio::test]
async fn read_only_mode_rejects_propose() {
    // The default fixture issues `Mode::Read`; even with a hypothetical
    // proposals dir, the mode gate must fire first.
    let fx = fixture(&["work"]).await;
    let err = call_err(
        &fx.server,
        "tools/call",
        tool_call(
            "context_propose",
            json!({
                "id": "rel-jane",
                "base_version": "sha256:0",
                "patch": { "body_append": "x" },
                "reason": "read mode"
            }),
        ),
    )
    .await;
    assert_eq!(err["code"], -32007);
    assert_eq!(err["data"]["tag"], "proposals_disabled");
}
