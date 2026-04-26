//! MCP HTTP transport end-to-end tests.
//!
//! Each `#[sqlx::test]` provisions an isolated Postgres DB, runs all
//! migrations, signs up an account, writes a few documents through the
//! existing vault endpoint, issues an `mcp_tokens` row through the
//! admin tokens endpoint, then exercises `/v1/mcp` with that bearer.
//!
//! Mirrors the bootstrap shape used by `oauth_flow.rs` and
//! `vault_flow.rs` so the user/tenant/auth setup stays one place to
//! change.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use orchext_server::{router, AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

const MAX_BODY: usize = 4 * 1024 * 1024;

async fn read_json(body: Body) -> Value {
    let bytes = to_bytes(body, MAX_BODY).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::Null)
}

fn signup_req(email: &str, password: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/auth/native/signup")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "email": email, "password": password }).to_string(),
        ))
        .unwrap()
}

async fn bootstrap(app: &axum::Router, email: &str) -> (String, String) {
    let signup = app
        .clone()
        .oneshot(signup_req(email, "correct horse battery staple"))
        .await
        .unwrap();
    assert_eq!(signup.status(), StatusCode::CREATED);
    let body = read_json(signup.into_body()).await;
    let secret = body["session"]["secret"].as_str().unwrap().to_string();

    let tenants = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/tenants")
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = read_json(tenants.into_body()).await;
    let tenant_id = body["memberships"][0]["tenant_id"]
        .as_str()
        .unwrap()
        .to_string();
    (secret, tenant_id)
}

/// Seed three docs at three visibilities. Returns nothing — tests
/// look documents up by id.
async fn seed_docs(app: &axum::Router, session: &str, tid: &str) {
    write_doc(
        app,
        session,
        tid,
        "rel-jane",
        DOC_WORK,
    )
    .await;
    write_doc(
        app,
        session,
        tid,
        "pref-comms",
        DOC_PUBLIC,
    )
    .await;
    write_doc(
        app,
        session,
        tid,
        "diary-2026-04-26",
        DOC_PRIVATE,
    )
    .await;
}

async fn write_doc(
    app: &axum::Router,
    session: &str,
    tid: &str,
    doc_id: &str,
    source: &str,
) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/v1/t/{tid}/vault/docs/{doc_id}"))
                .header("authorization", format!("Bearer {session}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "source": source }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "writing {doc_id}");
}

const DOC_WORK: &str = "---\n\
id: rel-jane\n\
type: relationships\n\
visibility: work\n\
tags:\n\
- manager\n\
- acme\n\
updated: 2026-04-19\n\
---\n\
# Jane Smith\n\
\n\
My manager at Acme. Prefers concise written updates.\n";

const DOC_PUBLIC: &str = "---\n\
id: pref-comms\n\
type: preferences\n\
visibility: public\n\
tags:\n\
- communication\n\
updated: 2026-04-20\n\
---\n\
# Communication style\n\
\n\
I prefer written over spoken updates whenever possible.\n";

const DOC_PRIVATE: &str = "---\n\
id: diary-2026-04-26\n\
type: memories\n\
visibility: private\n\
tags:\n\
- diary\n\
updated: 2026-04-26\n\
---\n\
# Diary entry\n\
\n\
Private content the agent must never reveal without explicit private scope.\n";

/// Issue an `mcp_tokens` row through the existing admin endpoint.
/// Returns the bearer secret. The `scope` argument is the visibility
/// labels the token may read; `mode` defaults to read.
async fn issue_token(
    app: &axum::Router,
    session: &str,
    tid: &str,
    label: &str,
    scope: &[&str],
) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/t/{tid}/tokens"))
                .header("authorization", format!("Bearer {session}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "label": label,
                        "scope": scope,
                        "mode": "read",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = read_json(resp.into_body()).await;
    body["secret"].as_str().unwrap().to_string()
}

fn rpc_call(token: &str, method: &str, params: Value, id: u64) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/mcp")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            })
            .to_string(),
        ))
        .unwrap()
}

// ---------- lifecycle ----------

#[sqlx::test(migrations = "./migrations")]
async fn mcp_initialize_returns_capabilities(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "init@example.com").await;
    let token = issue_token(&app, &sess, &tid, "init test", &["work"]).await;

    let resp = app
        .clone()
        .oneshot(rpc_call(&token, "initialize", json!({}), 1))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp.into_body()).await;
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 1);
    let result = &body["result"];
    assert_eq!(result["serverInfo"]["name"], "orchext");
    assert!(result["serverInfo"]["version"].is_string());
    assert_eq!(result["capabilities"]["tools"]["listChanged"], true);
    // SSE is deferred so subscribe is reported as false.
    assert_eq!(result["capabilities"]["resources"]["subscribe"], false);
}

#[sqlx::test(migrations = "./migrations")]
async fn mcp_tools_list_returns_three_tools(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "tools@example.com").await;
    let token = issue_token(&app, &sess, &tid, "tools test", &["work"]).await;

    let resp = app
        .clone()
        .oneshot(rpc_call(&token, "tools/list", json!({}), 1))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    let tools = body["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"context_search"));
    assert!(names.contains(&"context_get"));
    assert!(names.contains(&"context_list"));
}

// ---------- context_search ----------

#[sqlx::test(migrations = "./migrations")]
async fn mcp_search_finds_in_scope_doc(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "search@example.com").await;
    seed_docs(&app, &sess, &tid).await;
    let token = issue_token(&app, &sess, &tid, "search test", &["work", "public"]).await;

    let resp = app
        .clone()
        .oneshot(rpc_call(
            &token,
            "tools/call",
            json!({
                "name": "context_search",
                "arguments": { "query": "manager" }
            }),
            1,
        ))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    let structured = &body["result"]["structuredContent"];
    let results = structured["results"].as_array().unwrap();
    let ids: Vec<&str> = results.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"rel-jane"), "want rel-jane in {ids:?}");
}

#[sqlx::test(migrations = "./migrations")]
async fn mcp_search_excludes_private_when_not_in_scope(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "noprivate@example.com").await;
    seed_docs(&app, &sess, &tid).await;
    // Token does not include `private` scope, so the diary doc must
    // not appear regardless of query match.
    let token = issue_token(&app, &sess, &tid, "no-private", &["work", "public"]).await;

    let resp = app
        .clone()
        .oneshot(rpc_call(
            &token,
            "tools/call",
            json!({
                "name": "context_search",
                "arguments": { "query": "private" }
            }),
            1,
        ))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    let results = body["result"]["structuredContent"]["results"]
        .as_array()
        .unwrap();
    for r in results {
        assert_ne!(
            r["id"].as_str().unwrap(),
            "diary-2026-04-26",
            "private doc leaked"
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn mcp_search_widens_scope_returns_invalid_argument(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "widen@example.com").await;
    let token = issue_token(&app, &sess, &tid, "widen test", &["work"]).await;

    let resp = app
        .clone()
        .oneshot(rpc_call(
            &token,
            "tools/call",
            json!({
                "name": "context_search",
                "arguments": {
                    "query": "anything",
                    "scope": ["personal"]
                }
            }),
            1,
        ))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    assert_eq!(body["error"]["code"], -32004);
    assert_eq!(body["error"]["data"]["tag"], "invalid_argument");
}

// ---------- context_get ----------

#[sqlx::test(migrations = "./migrations")]
async fn mcp_get_returns_in_scope_doc(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "get@example.com").await;
    seed_docs(&app, &sess, &tid).await;
    let token = issue_token(&app, &sess, &tid, "get test", &["work"]).await;

    let resp = app
        .clone()
        .oneshot(rpc_call(
            &token,
            "tools/call",
            json!({ "name": "context_get", "arguments": { "id": "rel-jane" } }),
            1,
        ))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    let doc = &body["result"]["structuredContent"];
    assert_eq!(doc["id"], "rel-jane");
    assert_eq!(doc["type"], "relationships");
    assert!(doc["body"].as_str().unwrap().contains("Jane Smith"));
    assert!(doc["version"].as_str().unwrap().starts_with("sha256:"));
}

#[sqlx::test(migrations = "./migrations")]
async fn mcp_get_out_of_scope_returns_not_authorized(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "oof@example.com").await;
    seed_docs(&app, &sess, &tid).await;
    // Token has only "public" — must not see the work-scoped rel-jane.
    let token = issue_token(&app, &sess, &tid, "public-only", &["public"]).await;

    let resp = app
        .clone()
        .oneshot(rpc_call(
            &token,
            "tools/call",
            json!({ "name": "context_get", "arguments": { "id": "rel-jane" } }),
            1,
        ))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    assert_eq!(body["error"]["code"], -32002);
    assert_eq!(body["error"]["data"]["tag"], "not_authorized");
}

#[sqlx::test(migrations = "./migrations")]
async fn mcp_get_private_doc_blocked_without_private_scope(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "priv@example.com").await;
    seed_docs(&app, &sess, &tid).await;
    let token = issue_token(&app, &sess, &tid, "no-priv", &["work", "public"]).await;

    let resp = app
        .clone()
        .oneshot(rpc_call(
            &token,
            "tools/call",
            json!({ "name": "context_get", "arguments": { "id": "diary-2026-04-26" } }),
            1,
        ))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    assert_eq!(body["error"]["data"]["tag"], "not_authorized");
}

#[sqlx::test(migrations = "./migrations")]
async fn mcp_get_missing_doc_returns_not_authorized(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "miss@example.com").await;
    let token = issue_token(&app, &sess, &tid, "miss", &["work"]).await;

    let resp = app
        .clone()
        .oneshot(rpc_call(
            &token,
            "tools/call",
            json!({ "name": "context_get", "arguments": { "id": "nope" } }),
            1,
        ))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    // Indistinguishable from out-of-scope by design — see MCP.md §5.2.
    assert_eq!(body["error"]["data"]["tag"], "not_authorized");
}

// ---------- context_list ----------

#[sqlx::test(migrations = "./migrations")]
async fn mcp_list_filters_by_scope(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "list@example.com").await;
    seed_docs(&app, &sess, &tid).await;
    let token = issue_token(&app, &sess, &tid, "list-test", &["work"]).await;

    let resp = app
        .clone()
        .oneshot(rpc_call(
            &token,
            "tools/call",
            json!({ "name": "context_list", "arguments": {} }),
            1,
        ))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    let items = body["result"]["structuredContent"]["items"]
        .as_array()
        .unwrap();
    let ids: Vec<&str> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["rel-jane"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn mcp_list_filters_by_type(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "type@example.com").await;
    seed_docs(&app, &sess, &tid).await;
    let token =
        issue_token(&app, &sess, &tid, "type-test", &["work", "public", "private"]).await;

    let resp = app
        .clone()
        .oneshot(rpc_call(
            &token,
            "tools/call",
            json!({
                "name": "context_list",
                "arguments": { "type": "preferences" }
            }),
            1,
        ))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    let items = body["result"]["structuredContent"]["items"]
        .as_array()
        .unwrap();
    let ids: Vec<&str> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["pref-comms"]);
}

// ---------- resources ----------

#[sqlx::test(migrations = "./migrations")]
async fn mcp_resources_list_filters_by_scope(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "rl@example.com").await;
    seed_docs(&app, &sess, &tid).await;
    let token = issue_token(&app, &sess, &tid, "rl test", &["public"]).await;

    let resp = app
        .clone()
        .oneshot(rpc_call(&token, "resources/list", json!({}), 1))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    let resources = body["result"]["resources"].as_array().unwrap();
    let uris: Vec<&str> = resources.iter().map(|r| r["uri"].as_str().unwrap()).collect();
    // Only the public-visibility doc shows up.
    assert_eq!(uris, vec!["orchext://vault/preferences/pref-comms"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn mcp_resources_read_doc_returns_yaml_and_markdown(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "rr@example.com").await;
    seed_docs(&app, &sess, &tid).await;
    let token = issue_token(&app, &sess, &tid, "rr test", &["work"]).await;

    let resp = app
        .clone()
        .oneshot(rpc_call(
            &token,
            "resources/read",
            json!({ "uri": "orchext://vault/relationships/rel-jane" }),
            1,
        ))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    let contents = body["result"]["contents"].as_array().unwrap();
    assert_eq!(contents.len(), 2);
    let mimes: Vec<&str> = contents
        .iter()
        .map(|c| c["mimeType"].as_str().unwrap())
        .collect();
    assert!(mimes.contains(&"text/yaml"));
    assert!(mimes.contains(&"text/markdown"));
}

#[sqlx::test(migrations = "./migrations")]
async fn mcp_resources_read_root_returns_visible_types(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "root@example.com").await;
    seed_docs(&app, &sess, &tid).await;
    let token = issue_token(&app, &sess, &tid, "root test", &["work", "public"]).await;

    let resp = app
        .clone()
        .oneshot(rpc_call(
            &token,
            "resources/read",
            json!({ "uri": "orchext://vault/" }),
            1,
        ))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    let text = body["result"]["contents"][0]["text"].as_str().unwrap();
    let types: Vec<&str> = text.lines().collect();
    assert!(types.contains(&"relationships"));
    assert!(types.contains(&"preferences"));
    assert!(!types.contains(&"memories"), "private type leaked: {text}");
}

#[sqlx::test(migrations = "./migrations")]
async fn mcp_resources_read_out_of_scope_doc_returns_not_authorized(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "rdoof@example.com").await;
    seed_docs(&app, &sess, &tid).await;
    let token = issue_token(&app, &sess, &tid, "out-of-scope", &["public"]).await;

    let resp = app
        .clone()
        .oneshot(rpc_call(
            &token,
            "resources/read",
            json!({ "uri": "orchext://vault/relationships/rel-jane" }),
            1,
        ))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    assert_eq!(body["error"]["data"]["tag"], "not_authorized");
}

// ---------- auth failures ----------

#[sqlx::test(migrations = "./migrations")]
async fn mcp_no_bearer_returns_401(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/mcp")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn mcp_invalid_bearer_returns_401(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let resp = app
        .clone()
        .oneshot(rpc_call("ocx_completelywrongtoken_xx", "ping", json!({}), 1))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn mcp_revoked_token_returns_401(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "rev@example.com").await;
    let token = issue_token(&app, &sess, &tid, "rev test", &["work"]).await;

    // Revoke through the admin endpoint. We need the token id, which
    // is the first token returned from the list endpoint.
    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/t/{tid}/tokens"))
                .header("authorization", format!("Bearer {sess}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = read_json(list.into_body()).await;
    let token_id = body["tokens"][0]["id"].as_str().unwrap().to_string();
    let revoke = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/v1/t/{tid}/tokens/{token_id}"))
                .header("authorization", format!("Bearer {sess}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoke.status(), StatusCode::NO_CONTENT);

    // The revoked token now returns 401 on /v1/mcp.
    let resp = app
        .clone()
        .oneshot(rpc_call(&token, "ping", json!({}), 1))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ---------- protocol-level errors ----------

#[sqlx::test(migrations = "./migrations")]
async fn mcp_unknown_method_returns_jsonrpc_error(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "unk@example.com").await;
    let token = issue_token(&app, &sess, &tid, "unk", &["work"]).await;

    let resp = app
        .clone()
        .oneshot(rpc_call(&token, "totally/unknown", json!({}), 1))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp.into_body()).await;
    assert_eq!(body["error"]["code"], -32601);
}
