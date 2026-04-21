//! Vault + index + tokens + audit end-to-end tests.
//!
//! Each `#[sqlx::test]` spins up an isolated Postgres database, runs
//! migrations, and stands the axum router up via `tower::ServiceExt`.
//! Requires `DATABASE_URL` to be reachable; without it the integration
//! suite skips at connect time.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use mytex_server::{router, AppState};
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
        .uri("/v1/auth/signup")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "email": email, "password": password }).to_string(),
        ))
        .unwrap()
}

/// Sign up a fresh account and return `(session_secret, tenant_id)`.
async fn bootstrap_user(app: &axum::Router, email: &str) -> (String, String) {
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
    assert_eq!(tenants.status(), StatusCode::OK);
    let body = read_json(tenants.into_body()).await;
    let tenant_id = body["memberships"][0]["tenant_id"]
        .as_str()
        .unwrap()
        .to_string();
    (secret, tenant_id)
}

const DOC_SOURCE: &str = "---\n\
id: rel-jane-smith\n\
type: relationships\n\
visibility: work\n\
tags:\n\
- manager\n\
- acme\n\
links:\n\
- goal-q2-launch\n\
updated: 2026-04-19\n\
---\n\
# Jane Smith\n\
\n\
My manager at Acme.\n";

#[sqlx::test(migrations = "./migrations")]
async fn vault_write_read_roundtrip(db: PgPool) {
    let app = router(AppState::new(db));
    let (secret, tid) = bootstrap_user(&app, "vault@example.com").await;

    // PUT writes the doc.
    let write = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/v1/t/{tid}/vault/docs/rel-jane-smith"))
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "source": DOC_SOURCE }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(write.status(), StatusCode::OK);
    let body = read_json(write.into_body()).await;
    assert!(body["version"].as_str().unwrap().starts_with("sha256:"));

    // GET reads it back. The returned source round-trips — re-parsing
    // gives us the same frontmatter and body.
    let read = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/t/{tid}/vault/docs/rel-jane-smith"))
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::OK);
    let body = read_json(read.into_body()).await;
    let source = body["source"].as_str().unwrap();
    let parsed = mytex_vault::Document::parse(source).unwrap();
    assert_eq!(parsed.frontmatter.id.as_str(), "rel-jane-smith");
    assert_eq!(parsed.frontmatter.tags, vec!["manager", "acme"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn vault_version_conflict(db: PgPool) {
    let app = router(AppState::new(db));
    let (secret, tid) = bootstrap_user(&app, "conflict@example.com").await;

    // Initial write.
    let write = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/v1/t/{tid}/vault/docs/rel-jane-smith"))
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "source": DOC_SOURCE }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(write.status(), StatusCode::OK);

    // Second write with a bogus base_version must 409.
    let conflict = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/v1/t/{tid}/vault/docs/rel-jane-smith"))
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "source": DOC_SOURCE,
                        "base_version": "sha256:deadbeef"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let body = read_json(conflict.into_body()).await;
    assert_eq!(body["error"]["message"], "version_conflict");
}

#[sqlx::test(migrations = "./migrations")]
async fn vault_cross_tenant_is_not_found(db: PgPool) {
    // A second user cannot read another user's tenant's docs — the
    // tenant middleware rejects them before any doc handler runs.
    let app = router(AppState::new(db));
    let (_alice_secret, alice_tid) = bootstrap_user(&app, "alice@example.com").await;
    let (bob_secret, _bob_tid) = bootstrap_user(&app, "bob@example.com").await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/t/{alice_tid}/vault/docs"))
                .header("authorization", format!("Bearer {bob_secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "./migrations")]
async fn index_search_finds_content(db: PgPool) {
    let app = router(AppState::new(db));
    let (secret, tid) = bootstrap_user(&app, "search@example.com").await;

    // Write two docs.
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/v1/t/{tid}/vault/docs/rel-jane-smith"))
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "source": DOC_SOURCE }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let diary = "---\n\
id: diary-2026-04-19\n\
type: memories\n\
visibility: private\n\
updated: 2026-04-19\n\
---\n\
# Tuesday notes\n\
\n\
Confidential reflection on the Acme project.\n";
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/v1/t/{tid}/vault/docs/diary-2026-04-19"))
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "source": diary }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Search without the `private` visibility in scope — the diary
    // must not surface even though the query word matches its body.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/v1/t/{tid}/index/search?q=acme&visibility=work,public"
                ))
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp.into_body()).await;
    let ids: Vec<String> = body["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["doc_id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&"rel-jane-smith".to_string()));
    assert!(
        !ids.contains(&"diary-2026-04-19".to_string()),
        "private doc must not surface when visibility scope omits `private`"
    );

    // With `private` in scope, the diary comes through.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/v1/t/{tid}/index/search?q=acme&visibility=work,private"
                ))
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = read_json(resp.into_body()).await;
    let ids: Vec<String> = body["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["doc_id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&"diary-2026-04-19".to_string()));
}

#[sqlx::test(migrations = "./migrations")]
async fn audit_chain_records_writes(db: PgPool) {
    let app = router(AppState::new(db));
    let (secret, tid) = bootstrap_user(&app, "audit@example.com").await;

    // One write → one `vault.write` entry at seq 0.
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/v1/t/{tid}/vault/docs/rel-jane-smith"))
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "source": DOC_SOURCE }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // One read → one `vault.read` entry at seq 1, chained off seq 0.
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/t/{tid}/vault/docs/rel-jane-smith"))
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/t/{tid}/audit"))
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp.into_body()).await;
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "expected one write + one read entry");
    assert_eq!(entries[0]["seq"], 0);
    assert_eq!(entries[0]["action"], "vault.write");
    assert_eq!(entries[1]["seq"], 1);
    assert_eq!(entries[1]["action"], "vault.read");
    // Chain linkage: entry[1].prev_hash == entry[0].hash.
    assert_eq!(entries[1]["prev_hash"], entries[0]["hash"]);
    assert_eq!(body["head_hash"], entries[1]["hash"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn tokens_issue_and_revoke(db: PgPool) {
    let app = router(AppState::new(db));
    let (secret, tid) = bootstrap_user(&app, "tokens@example.com").await;

    // Issue a token.
    let issue = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/t/{tid}/tokens"))
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "label": "claude-desktop",
                        "scope": ["work", "public"],
                        "mode": "read"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(issue.status(), StatusCode::CREATED);
    let body = read_json(issue.into_body()).await;
    let token_id = body["token"]["id"].as_str().unwrap().to_string();
    assert!(body["secret"].as_str().unwrap().starts_with("mtx_"));

    // List contains it.
    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/t/{tid}/tokens"))
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = read_json(list.into_body()).await;
    assert_eq!(body["tokens"].as_array().unwrap().len(), 1);

    // Revoke it — 204.
    let rev = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/v1/t/{tid}/tokens/{token_id}"))
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rev.status(), StatusCode::NO_CONTENT);

    // Revoking again is now 404 because the first revoke flipped
    // `revoked_at`; the UPDATE ... WHERE revoked_at IS NULL matches zero
    // rows. That's the enumeration-safe behavior we want.
    let rev2 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/v1/t/{tid}/tokens/{token_id}"))
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rev2.status(), StatusCode::NOT_FOUND);
}
