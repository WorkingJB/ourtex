//! `context.propose` write-back flow end-to-end.
//!
//! Pins the four invariants that 2b.5 slice 4 promises:
//!   1. A `read+propose` token can submit a proposal that lands in the
//!      review queue without mutating the document.
//!   2. A `read` token gets `proposals_disabled`; the document is
//!      untouched.
//!   3. Approve applies the patch, bumps the document's version, and
//!      flips the proposal to `approved` carrying `applied_version`.
//!   4. A second propose against the post-approval document fails with
//!      `version_conflict` if it carries the stale base.

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

const DOC: &str = "---\n\
id: rel-jane\n\
type: relationships\n\
visibility: work\n\
tags:\n\
- manager\n\
updated: 2026-04-26\n\
---\n\
# Jane Smith\n\
\n\
My manager at Acme.\n";

async fn bootstrap(app: &axum::Router, email: &str) -> (String, String) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/native/signup")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "email": email, "password": "correct horse battery staple" })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = read_json(resp.into_body()).await;
    let secret = body["session"]["secret"].as_str().unwrap().to_string();

    let resp = app
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
    let body = read_json(resp.into_body()).await;
    let tid = body["memberships"][0]["tenant_id"]
        .as_str()
        .unwrap()
        .to_string();
    (secret, tid)
}

async fn write_doc(app: &axum::Router, sess: &str, tid: &str, source: &str) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/v1/t/{tid}/vault/docs/rel-jane"))
                .header("authorization", format!("Bearer {sess}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "source": source }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    read_json(resp.into_body()).await["version"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn issue_token(
    app: &axum::Router,
    sess: &str,
    tid: &str,
    label: &str,
    scope: &[&str],
    mode: &str,
) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/t/{tid}/tokens"))
                .header("authorization", format!("Bearer {sess}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "label": label,
                        "scope": scope,
                        "mode": mode,
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    read_json(resp.into_body()).await["secret"]
        .as_str()
        .unwrap()
        .to_string()
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

async fn current_doc_version(app: &axum::Router, sess: &str, tid: &str) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/t/{tid}/vault/docs/rel-jane"))
                .header("authorization", format!("Bearer {sess}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    read_json(resp.into_body()).await["version"]
        .as_str()
        .unwrap()
        .to_string()
}

#[sqlx::test(migrations = "./migrations")]
async fn read_only_token_cannot_propose(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "ro@example.com").await;
    let version = write_doc(&app, &sess, &tid, DOC).await;
    let token = issue_token(&app, &sess, &tid, "ro", &["work"], "read").await;

    let resp = app
        .clone()
        .oneshot(rpc_call(
            &token,
            "tools/call",
            json!({
                "name": "context_propose",
                "arguments": {
                    "id": "rel-jane",
                    "base_version": version,
                    "patch": { "body_append": "\n\nMet 2026-04-27." },
                    "reason": "test"
                }
            }),
            1,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp.into_body()).await;
    assert_eq!(body["error"]["code"], -32007);
    assert_eq!(body["error"]["data"]["tag"], "proposals_disabled");
}

#[sqlx::test(migrations = "./migrations")]
async fn propose_then_list_then_approve(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "rw@example.com").await;
    let version = write_doc(&app, &sess, &tid, DOC).await;
    let agent = issue_token(&app, &sess, &tid, "agent", &["work"], "read_propose").await;

    // 1. Agent proposes a body append + frontmatter merge.
    let resp = app
        .clone()
        .oneshot(rpc_call(
            &agent,
            "tools/call",
            json!({
                "name": "context_propose",
                "arguments": {
                    "id": "rel-jane",
                    "base_version": version,
                    "patch": {
                        "frontmatter": { "tags": ["manager", "acme", "mentor"] },
                        "body_append": "\n\nNote on 2026-04-27.\n"
                    },
                    "reason": "Observed during 1:1."
                }
            }),
            1,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp.into_body()).await;
    let proposal_id = body["result"]["structuredContent"]["proposal_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        body["result"]["structuredContent"]["status"]
            .as_str()
            .unwrap(),
        "pending"
    );

    // The vault is unchanged at this point — propose never mutates.
    let post_propose_version = current_doc_version(&app, &sess, &tid).await;
    assert_eq!(post_propose_version, version);

    // 2. Reviewer (the same logged-in account, who is owner) lists.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/t/{tid}/proposals"))
                .header("authorization", format!("Bearer {sess}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp.into_body()).await;
    let arr = body["proposals"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"].as_str().unwrap(), proposal_id);
    assert_eq!(arr[0]["status"].as_str().unwrap(), "pending");

    // 3. Approve.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/t/{tid}/proposals/{proposal_id}/approve"))
                .header("authorization", format!("Bearer {sess}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "note": null }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp.into_body()).await;
    assert_eq!(body["proposal"]["status"].as_str().unwrap(), "approved");
    let new_version = body["applied_version"].as_str().unwrap().to_string();
    assert_ne!(new_version, version, "version must bump on approve");

    // 4. Read the doc back: tags are merged, body has the appended note,
    //    version matches `applied_version`.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/t/{tid}/vault/docs/rel-jane"))
                .header("authorization", format!("Bearer {sess}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    let source = body["source"].as_str().unwrap();
    assert!(source.contains("mentor"), "tags must include `mentor`");
    assert!(source.contains("Note on 2026-04-27."));
    assert_eq!(body["version"].as_str().unwrap(), new_version);
}

#[sqlx::test(migrations = "./migrations")]
async fn approve_with_stale_base_version_conflicts(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "stale@example.com").await;
    let v1 = write_doc(&app, &sess, &tid, DOC).await;
    let agent = issue_token(&app, &sess, &tid, "agent", &["work"], "read_propose").await;

    // Agent proposes against v1.
    let resp = app
        .clone()
        .oneshot(rpc_call(
            &agent,
            "tools/call",
            json!({
                "name": "context_propose",
                "arguments": {
                    "id": "rel-jane",
                    "base_version": v1,
                    "patch": { "body_append": "\n\nA." },
                    "reason": "first"
                }
            }),
            1,
        ))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    let stale_id = body["result"]["structuredContent"]["proposal_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Owner directly writes a competing change to bump the version,
    // making the pending proposal's base stale.
    let updated = "---\n\
        id: rel-jane\n\
        type: relationships\n\
        visibility: work\n\
        tags:\n\
        - manager\n\
        updated: 2026-04-27\n\
        ---\n\
        # Jane Smith\n\
        \n\
        My manager at Acme. Updated by owner.\n";
    let v2 = write_doc(&app, &sess, &tid, updated).await;
    assert_ne!(v1, v2);

    // Approve must fail with version_conflict.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/t/{tid}/proposals/{stale_id}/approve"))
                .header("authorization", format!("Bearer {sess}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "note": null }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "./migrations")]
async fn reject_marks_proposal_without_changing_doc(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "reject@example.com").await;
    let version = write_doc(&app, &sess, &tid, DOC).await;
    let agent = issue_token(&app, &sess, &tid, "agent", &["work"], "read_propose").await;

    let resp = app
        .clone()
        .oneshot(rpc_call(
            &agent,
            "tools/call",
            json!({
                "name": "context_propose",
                "arguments": {
                    "id": "rel-jane",
                    "base_version": version,
                    "patch": { "body_replace": "totally different body" },
                    "reason": "rewrite"
                }
            }),
            1,
        ))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    let pid = body["result"]["structuredContent"]["proposal_id"]
        .as_str()
        .unwrap()
        .to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/t/{tid}/proposals/{pid}/reject"))
                .header("authorization", format!("Bearer {sess}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "note": "too aggressive" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp.into_body()).await;
    assert_eq!(body["status"].as_str().unwrap(), "rejected");
    assert_eq!(body["decision_note"].as_str().unwrap(), "too aggressive");

    // Doc version unchanged.
    assert_eq!(current_doc_version(&app, &sess, &tid).await, version);
}

#[sqlx::test(migrations = "./migrations")]
async fn cannot_approve_already_decided_proposal(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "twice@example.com").await;
    let version = write_doc(&app, &sess, &tid, DOC).await;
    let agent = issue_token(&app, &sess, &tid, "agent", &["work"], "read_propose").await;

    let resp = app
        .clone()
        .oneshot(rpc_call(
            &agent,
            "tools/call",
            json!({
                "name": "context_propose",
                "arguments": {
                    "id": "rel-jane",
                    "base_version": version,
                    "patch": { "body_append": "\n\nx" },
                    "reason": "first"
                }
            }),
            1,
        ))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    let pid = body["result"]["structuredContent"]["proposal_id"]
        .as_str()
        .unwrap()
        .to_string();

    let approve_req = || {
        Request::builder()
            .method("POST")
            .uri(format!("/v1/t/{tid}/proposals/{pid}/approve"))
            .header("authorization", format!("Bearer {sess}"))
            .header("content-type", "application/json")
            .body(Body::from(json!({ "note": null }).to_string()))
            .unwrap()
    };

    let first = app.clone().oneshot(approve_req()).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = app.clone().oneshot(approve_req()).await.unwrap();
    assert_eq!(second.status(), StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "./migrations")]
async fn propose_against_unknown_doc_returns_not_authorized(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (sess, tid) = bootstrap(&app, "ghost@example.com").await;
    let agent = issue_token(&app, &sess, &tid, "agent", &["work"], "read_propose").await;

    let resp = app
        .clone()
        .oneshot(rpc_call(
            &agent,
            "tools/call",
            json!({
                "name": "context_propose",
                "arguments": {
                    "id": "no-such-doc",
                    "base_version": "sha256:0",
                    "patch": { "body_append": "x" },
                    "reason": "test"
                }
            }),
            1,
        ))
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    assert_eq!(body["error"]["code"], -32002);
    assert_eq!(body["error"]["data"]["tag"], "not_authorized");
}
