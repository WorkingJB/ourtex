//! OAuth 2.1 + PKCE end-to-end tests.
//!
//! Each `#[sqlx::test]` provisions a fresh Postgres DB, runs migrations,
//! and exercises the `/v1/oauth/{authorize,token}` surface against a
//! real router. Mirrors the shape of `vault_flow.rs` (signup → tenants
//! → action) so the user/tenant bootstrap stays one place to change.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use orchext_server::{router, AppState};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tower::ServiceExt;

const MAX_BODY: usize = 1 << 20;

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

/// Generate a 64-char verifier (alpha+digits only) and its S256
/// challenge. RFC 7636: verifier = 43..=128 chars from the unreserved
/// set; challenge = base64url(SHA-256(verifier)) without padding.
fn pkce_pair() -> (String, String) {
    let verifier: String =
        "abcDEF123-._~xyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".to_string();
    let mut h = Sha256::new();
    h.update(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(h.finalize());
    (verifier, challenge)
}

fn authorize_req(
    bearer: &str,
    tenant_id: &str,
    redirect_uri: &str,
    challenge: &str,
    method: &str,
) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/oauth/authorize")
        .header("authorization", format!("Bearer {bearer}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "tenant_id": tenant_id,
                "client_label": "Test agent",
                "redirect_uri": redirect_uri,
                "scope": ["work"],
                "code_challenge": challenge,
                "code_challenge_method": method,
            })
            .to_string(),
        ))
        .unwrap()
}

fn token_req(
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/oauth/token")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "grant_type": "authorization_code",
                "code": code,
                "code_verifier": verifier,
                "redirect_uri": redirect_uri,
            })
            .to_string(),
        ))
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn oauth_happy_path(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (secret, tid) = bootstrap_user(&app, "oauth@example.com").await;
    let (verifier, challenge) = pkce_pair();
    let redirect = "http://127.0.0.1:5555/cb";

    let resp = app
        .clone()
        .oneshot(authorize_req(&secret, &tid, redirect, &challenge, "S256"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = read_json(resp.into_body()).await;
    let code = body["code"].as_str().unwrap().to_string();
    assert!(code.starts_with("oac_"), "unexpected code: {code}");

    let resp = app
        .clone()
        .oneshot(token_req(&code, &verifier, redirect))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp.into_body()).await;
    let access = body["access_token"].as_str().unwrap();
    assert!(access.starts_with("ocx_"));
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["scope"], "work");
    assert_eq!(body["tenant_id"].as_str().unwrap(), tid);

    // The new bearer should be accepted by tenant-scoped routes — list
    // tokens to prove the row exists and the secret authenticates.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/t/{tid}/tokens"))
                // The OAuth-issued token isn't a session, so this lookup
                // uses the original session secret. We're proving the
                // *issued* token row exists.
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp.into_body()).await;
    let tokens = body["tokens"].as_array().unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0]["label"], "Test agent");
    assert_eq!(tokens[0]["scope"][0], "work");
}

#[sqlx::test(migrations = "./migrations")]
async fn oauth_wrong_verifier_rejected(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (secret, tid) = bootstrap_user(&app, "wrongv@example.com").await;
    let (_verifier, challenge) = pkce_pair();
    let redirect = "http://127.0.0.1:5555/cb";

    let resp = app
        .clone()
        .oneshot(authorize_req(&secret, &tid, redirect, &challenge, "S256"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let code = read_json(resp.into_body()).await["code"]
        .as_str()
        .unwrap()
        .to_string();

    // Submit a verifier that doesn't hash to the challenge.
    let bad_verifier = "x".repeat(64);
    let resp = app
        .clone()
        .oneshot(token_req(&code, &bad_verifier, redirect))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn oauth_code_single_use(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (secret, tid) = bootstrap_user(&app, "single@example.com").await;
    let (verifier, challenge) = pkce_pair();
    let redirect = "http://127.0.0.1:5555/cb";

    let code = read_json(
        app.clone()
            .oneshot(authorize_req(&secret, &tid, redirect, &challenge, "S256"))
            .await
            .unwrap()
            .into_body(),
    )
    .await["code"]
        .as_str()
        .unwrap()
        .to_string();

    let first = app
        .clone()
        .oneshot(token_req(&code, &verifier, redirect))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = app
        .clone()
        .oneshot(token_req(&code, &verifier, redirect))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn oauth_redirect_uri_must_match(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (secret, tid) = bootstrap_user(&app, "redir@example.com").await;
    let (verifier, challenge) = pkce_pair();
    let redirect = "http://127.0.0.1:5555/cb";

    let code = read_json(
        app.clone()
            .oneshot(authorize_req(&secret, &tid, redirect, &challenge, "S256"))
            .await
            .unwrap()
            .into_body(),
    )
    .await["code"]
        .as_str()
        .unwrap()
        .to_string();

    let resp = app
        .clone()
        .oneshot(token_req(&code, &verifier, "http://127.0.0.1:5555/other"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn oauth_plain_method_rejected(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (secret, tid) = bootstrap_user(&app, "plain@example.com").await;
    let (_verifier, challenge) = pkce_pair();
    let redirect = "http://127.0.0.1:5555/cb";

    let resp = app
        .clone()
        .oneshot(authorize_req(&secret, &tid, redirect, &challenge, "plain"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = read_json(resp.into_body()).await;
    assert_eq!(body["error"]["tag"], "invalid_argument");
}

#[sqlx::test(migrations = "./migrations")]
async fn oauth_non_loopback_http_redirect_rejected(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (secret, tid) = bootstrap_user(&app, "rhttp@example.com").await;
    let (_verifier, challenge) = pkce_pair();

    let resp = app
        .clone()
        .oneshot(authorize_req(
            &secret,
            &tid,
            "http://example.com/cb",
            &challenge,
            "S256",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn oauth_authorize_requires_session(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (_verifier, challenge) = pkce_pair();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/oauth/authorize")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "tenant_id": "00000000-0000-0000-0000-000000000000",
                        "client_label": "Anonymous",
                        "redirect_uri": "http://127.0.0.1:5555/cb",
                        "scope": ["work"],
                        "code_challenge": challenge,
                        "code_challenge_method": "S256",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn oauth_authorize_non_member_tenant_404(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let (secret, _own_tid) = bootstrap_user(&app, "nm@example.com").await;
    let (_verifier, challenge) = pkce_pair();

    // Random UUID — the user has no membership in it.
    let resp = app
        .clone()
        .oneshot(authorize_req(
            &secret,
            "11111111-1111-1111-1111-111111111111",
            "http://127.0.0.1:5555/cb",
            &challenge,
            "S256",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "./migrations")]
async fn oauth_token_grant_type_required(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/oauth/token")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "grant_type": "client_credentials",
                        "code": "oac_irrelevant",
                        "code_verifier": "x".repeat(64),
                        "redirect_uri": "http://127.0.0.1:5555/cb",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
