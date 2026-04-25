//! End-to-end auth flow tests: signup → login → session validation →
//! logout.
//!
//! These hit a real Postgres via `sqlx::test`, which creates a fresh
//! isolated database per test and runs migrations automatically. They
//! require Postgres to be reachable via `DATABASE_URL` (or whatever
//! sqlx's test harness resolves via `.env` / env vars).
//!
//! If Postgres is not available, running `cargo test -p ourtex-server`
//! will still pass because these tests are in a separate integration
//! test target that `sqlx` simply won't execute without a DB — each
//! `#[sqlx::test]` function connects at test start and errors cleanly.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use ourtex_server::{router, AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt; // for `oneshot`

/// One MiB is plenty for any auth-flow response.
const MAX_BODY: usize = 1 << 20;

#[sqlx::test(migrations = "./migrations")]
async fn signup_then_me_roundtrip(db: PgPool) {
    let app = router(AppState::new(db));

    // Signup.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/signup")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "email": "user@example.com",
                        "password": "correct horse battery staple",
                        "display_name": "User"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: Value = read_json(resp.into_body()).await;
    let secret = body["session"]["secret"].as_str().unwrap().to_string();
    assert!(secret.starts_with("otx_"));
    let account_id = body["account"]["id"].as_str().unwrap().to_string();

    // Me with bearer — should return the same account.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/auth/me")
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let me: Value = read_json(resp.into_body()).await;
    assert_eq!(me["account"]["id"], account_id);
    assert_eq!(me["account"]["email"], "user@example.com");
}

#[sqlx::test(migrations = "./migrations")]
async fn login_after_signup_succeeds(db: PgPool) {
    let app = router(AppState::new(db));

    // Signup.
    let _ = app
        .clone()
        .oneshot(signup_req("user@example.com", "correct horse battery staple"))
        .await
        .unwrap();

    // Login with same creds.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "email": "user@example.com",
                        "password": "correct horse battery staple"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = read_json(resp.into_body()).await;
    let secret = body["session"]["secret"].as_str().unwrap();
    assert!(secret.starts_with("otx_"));
}

#[sqlx::test(migrations = "./migrations")]
async fn login_wrong_password_unauthorized(db: PgPool) {
    let app = router(AppState::new(db));

    let _ = app
        .clone()
        .oneshot(signup_req("user@example.com", "correct horse battery staple"))
        .await
        .unwrap();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "email": "user@example.com",
                        "password": "wrong-password-for-user"
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
async fn login_unknown_email_indistinguishable_from_wrong_password(db: PgPool) {
    // The enumeration-resistance invariant: both failures map to 401
    // with the same tag. If this regresses, an attacker can probe
    // which emails have accounts.
    let app = router(AppState::new(db));

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "email": "nobody@example.com",
                        "password": "correct horse battery staple"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body: Value = read_json(resp.into_body()).await;
    assert_eq!(body["error"]["tag"], "unauthorized");
}

#[sqlx::test(migrations = "./migrations")]
async fn duplicate_signup_conflicts(db: PgPool) {
    let app = router(AppState::new(db));
    let _ = app
        .clone()
        .oneshot(signup_req("user@example.com", "correct horse battery staple"))
        .await
        .unwrap();

    let resp = app
        .clone()
        .oneshot(signup_req("user@example.com", "another valid password"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "./migrations")]
async fn logout_revokes_session(db: PgPool) {
    let app = router(AppState::new(db));
    let signup_resp = app
        .clone()
        .oneshot(signup_req("user@example.com", "correct horse battery staple"))
        .await
        .unwrap();
    let body: Value = read_json(signup_resp.into_body()).await;
    let secret = body["session"]["secret"].as_str().unwrap().to_string();

    // Logout.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/auth/logout")
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Subsequent me with the same bearer must fail. Note: the 60s
    // in-memory cache would normally return a cached context, but
    // `revoke` invalidates cache entries for the session_id, so this
    // works immediately.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/auth/me")
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn me_without_bearer_unauthorized(db: PgPool) {
    let app = router(AppState::new(db));
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/auth/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn short_password_rejected(db: PgPool) {
    let app = router(AppState::new(db));
    let resp = app
        .oneshot(signup_req("user@example.com", "short"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn healthz_ok(db: PgPool) {
    let app = router(AppState::new(db));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ---------- helpers ----------

fn signup_req(email: &str, password: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/auth/signup")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "email": email,
                "password": password,
            })
            .to_string(),
        ))
        .unwrap()
}

async fn read_json(body: Body) -> Value {
    let bytes = to_bytes(body, MAX_BODY).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
