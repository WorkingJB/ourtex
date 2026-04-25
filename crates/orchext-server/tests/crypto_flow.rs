//! End-to-end encryption tests: seed → publish → write encrypted →
//! read decrypted → lock → vault_locked on subsequent reads.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use ourtex_crypto::{
    derive_master_key, unwrap_content_key, wrap_content_key, ContentKey, Salt, SealedBlob,
};
use ourtex_server::{router, AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

const MAX_BODY: usize = 4 * 1024 * 1024;

async fn read_json(body: Body) -> Value {
    let bytes = to_bytes(body, MAX_BODY).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::Null)
}

async fn bootstrap(app: &axum::Router, email: &str) -> (String, String) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/signup")
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

    let t = app
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
    let tb = read_json(t.into_body()).await;
    let tid = tb["memberships"][0]["tenant_id"]
        .as_str()
        .unwrap()
        .to_string();
    (secret, tid)
}

const DOC_SOURCE: &str = "---\n\
id: rel-jane-smith\n\
type: relationships\n\
visibility: work\n\
updated: 2026-04-19\n\
---\n\
# Jane Smith\n\
\n\
My manager at Acme.\n";

/// Seed crypto client-side (same dance the desktop does in
/// `workspace_unlock`). Returns the plaintext content key for later
/// publish calls.
async fn seed_crypto(app: &axum::Router, tid: &str, secret: &str) -> ContentKey {
    let salt = Salt::generate();
    let master = derive_master_key("correct horse battery staple", &salt).unwrap();
    let content = ContentKey::generate();
    let wrapped = wrap_content_key(&content, &master).unwrap();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/t/{tid}/vault/init-crypto"))
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "kdf_salt": salt.to_wire(),
                        "wrapped_content_key": wrapped.to_wire(),
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    content
}

async fn publish_key(app: &axum::Router, tid: &str, secret: &str, key: &ContentKey) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/t/{tid}/session-key"))
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "key": key.to_wire() }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[sqlx::test(migrations = "./migrations")]
async fn encrypted_round_trip(db: PgPool) {
    let app = router(AppState::new(db));
    let (secret, tid) = bootstrap(&app, "crypto@example.com").await;
    let content = seed_crypto(&app, &tid, &secret).await;
    publish_key(&app, &tid, &secret, &content).await;

    // Write — server encrypts using the published key.
    let w = app
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
    assert_eq!(w.status(), StatusCode::OK);

    // Read — server decrypts and returns the canonical source
    // identical to what we wrote.
    let r = app
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
    assert_eq!(r.status(), StatusCode::OK);
    let b = read_json(r.into_body()).await;
    let parsed = ourtex_vault::Document::parse(b["source"].as_str().unwrap()).unwrap();
    assert_eq!(parsed.frontmatter.id.as_str(), "rel-jane-smith");
    assert!(parsed.body.contains("My manager at Acme."));
}

#[sqlx::test(migrations = "./migrations")]
async fn vault_locked_without_key(db: PgPool) {
    let app = router(AppState::new(db));
    let (secret, tid) = bootstrap(&app, "locked@example.com").await;
    let content = seed_crypto(&app, &tid, &secret).await;
    publish_key(&app, &tid, &secret, &content).await;

    // Write an encrypted doc.
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

    // Revoke the key. Server now has no live content key for this
    // tenant.
    let rv = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/v1/t/{tid}/session-key"))
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rv.status(), StatusCode::NO_CONTENT);

    // Read of the encrypted row must now return 423 Locked with tag
    // `vault_locked`.
    let r = app
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
    assert_eq!(r.status(), StatusCode::LOCKED);
    let b = read_json(r.into_body()).await;
    assert_eq!(b["error"]["tag"], "vault_locked");

    // Write of a new doc also fails while locked.
    let w = app
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
    assert_eq!(w.status(), StatusCode::LOCKED);
}

#[sqlx::test(migrations = "./migrations")]
async fn wrong_passphrase_fails_to_unwrap(db: PgPool) {
    // The server never sees the passphrase; it only stores the
    // wrapped content key. This test proves the client-side flow —
    // same fetch + unwrap path the desktop runs — fails for the
    // wrong passphrase.
    let app = router(AppState::new(db));
    let (secret, tid) = bootstrap(&app, "wrongpass@example.com").await;

    let salt = Salt::generate();
    let master = derive_master_key("correct horse battery staple", &salt).unwrap();
    let content = ContentKey::generate();
    let wrapped = wrap_content_key(&content, &master).unwrap();
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/t/{tid}/vault/init-crypto"))
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "kdf_salt": salt.to_wire(),
                        "wrapped_content_key": wrapped.to_wire(),
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Now simulate a "second device" trying to unlock with the wrong
    // passphrase. It fetches the crypto state, derives a master key
    // with the wrong passphrase, and fails to unwrap.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/t/{tid}/vault/crypto"))
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let b = read_json(resp.into_body()).await;
    let salt_wire = b["kdf_salt"].as_str().unwrap();
    let wrapped_wire = b["wrapped_content_key"].as_str().unwrap();

    let bad_master = derive_master_key(
        "wrong horse battery staple",
        &Salt::from_wire(salt_wire).unwrap(),
    )
    .unwrap();
    let wrapped_blob = SealedBlob::from_wire(wrapped_wire).unwrap();
    assert!(unwrap_content_key(&wrapped_blob, &bad_master).is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn init_crypto_is_idempotent_forbidden(db: PgPool) {
    // A second `init-crypto` must 409 — the tenant is already seeded
    // and overwriting the wrapped key would orphan existing ciphertext.
    let app = router(AppState::new(db));
    let (secret, tid) = bootstrap(&app, "reseed@example.com").await;
    let _ = seed_crypto(&app, &tid, &secret).await;

    let salt = Salt::generate();
    let master = derive_master_key("correct horse battery staple", &salt).unwrap();
    let content = ContentKey::generate();
    let wrapped = wrap_content_key(&content, &master).unwrap();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/t/{tid}/vault/init-crypto"))
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "kdf_salt": salt.to_wire(),
                        "wrapped_content_key": wrapped.to_wire(),
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let b = read_json(resp.into_body()).await;
    assert_eq!(b["error"]["message"], "crypto_already_seeded");
}

#[sqlx::test(migrations = "./migrations")]
async fn plaintext_legacy_rows_still_readable(db: PgPool) {
    // A tenant without crypto seeded continues to operate in plaintext
    // mode. This pins the 2b.2 compatibility guarantee: adding 2b.3
    // doesn't force migration of existing data.
    let app = router(AppState::new(db));
    let (secret, tid) = bootstrap(&app, "plain@example.com").await;

    // Write without seeding crypto → plaintext storage.
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

    let r = app
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
    assert_eq!(r.status(), StatusCode::OK);
    let b = read_json(r.into_body()).await;
    assert!(b["source"].as_str().unwrap().contains("My manager at Acme."));
}
