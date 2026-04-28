//! End-to-end auth flow tests: signup → login → session validation →
//! logout.
//!
//! These hit a real Postgres via `sqlx::test`, which creates a fresh
//! isolated database per test and runs migrations automatically. They
//! require Postgres to be reachable via `DATABASE_URL` (or whatever
//! sqlx's test harness resolves via `.env` / env vars).
//!
//! If Postgres is not available, running `cargo test -p orchext-server`
//! will still pass because these tests are in a separate integration
//! test target that `sqlx` simply won't execute without a DB — each
//! `#[sqlx::test]` function connects at test start and errors cleanly.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use orchext_server::{router, AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt; // for `oneshot`

/// One MiB is plenty for any auth-flow response.
const MAX_BODY: usize = 1 << 20;

#[sqlx::test(migrations = "./migrations")]
async fn signup_then_me_roundtrip(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));

    // Signup.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/native/signup")
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
    assert!(secret.starts_with("ocx_"));
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
    let app = router(AppState::new(db).with_rate_limit_auth(false));

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
                .uri("/v1/auth/native/login")
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
    assert!(secret.starts_with("ocx_"));
}

#[sqlx::test(migrations = "./migrations")]
async fn login_wrong_password_unauthorized(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));

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
                .uri("/v1/auth/native/login")
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
    let app = router(AppState::new(db).with_rate_limit_auth(false));

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/native/login")
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
    let app = router(AppState::new(db).with_rate_limit_auth(false));
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
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let signup_resp = app
        .clone()
        .oneshot(native_signup_req(
            "user@example.com",
            "correct horse battery staple",
        ))
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
    let app = router(AppState::new(db).with_rate_limit_auth(false));
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
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let resp = app
        .oneshot(signup_req("user@example.com", "short"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn browser_signup_does_not_leak_secret(db: PgPool) {
    // Pins the P1 fix from the adversarial review: browser endpoints
    // must NOT include the bearer secret in the JSON body. The session
    // reaches the browser only via the HttpOnly cookie set in the
    // response.
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/signup")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "email": "browser@example.com",
                        "password": "correct horse battery staple"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let cookie_header = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|h| h.to_str().ok())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        cookie_header.contains("orchext_session="),
        "browser signup must set the session cookie; got cookies: {cookie_header}"
    );
    let body: Value = read_json(resp.into_body()).await;
    assert!(
        body["session"].get("secret").is_none(),
        "browser signup must not return the bearer secret in JSON; got {body}"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn healthz_ok(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
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

#[sqlx::test(migrations = "./migrations")]
async fn readyz_ok_when_db_reachable(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[sqlx::test(migrations = "./migrations")]
async fn signup_succeeds_with_rate_limiter_enabled_and_xff(db: PgPool) {
    // Regression: production shipped with `rate_limit_auth: true` (the
    // default) but `tower_governor`'s key extractor couldn't find a
    // client IP — both signup and login 500'd with "Unable To Extract
    // Key" on every request. The fix wires `SmartIpKeyExtractor` (which
    // reads X-Forwarded-For) and `into_make_service_with_connect_info`
    // in the binary. This test pins the XFF path: rate_limit ON, peer
    // IP unavailable (oneshot), but XFF set — must succeed.
    let app = router(AppState::new(db));
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/native/signup")
                .header("content-type", "application/json")
                .header("x-forwarded-for", "203.0.113.7")
                .body(Body::from(
                    json!({
                        "email": "ratelimit@example.com",
                        "password": "correct horse battery staple",
                        "display_name": "RL"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "signup must succeed when XFF is present and rate-limit is enabled"
    );
}

#[test]
fn cors_layer_returns_none_for_empty_origin_list() {
    assert!(orchext_server::cors_layer(&[]).is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn cors_echoes_allowed_origin(db: PgPool) {
    let allowed = "https://app.example.com".to_string();
    let cors = orchext_server::cors_layer(&[allowed.clone()])
        .expect("cors layer present when origin given");
    let app = router(AppState::new(db).with_rate_limit_auth(false)).layer(cors);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .header("origin", &allowed)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some(allowed.as_str()),
        "allowed origin must be echoed back"
    );
    assert_eq!(
        resp.headers()
            .get("access-control-allow-credentials")
            .and_then(|v| v.to_str().ok()),
        Some("true"),
        "credentials must be allowed for cookie auth"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn cors_does_not_echo_disallowed_origin(db: PgPool) {
    let cors = orchext_server::cors_layer(&["https://app.example.com".to_string()])
        .expect("cors layer present when origin given");
    let app = router(AppState::new(db).with_rate_limit_auth(false)).layer(cors);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .header("origin", "https://evil.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Response still completes (CORS is a browser-side enforcement),
    // but no allow-origin header for the wrong origin.
    assert!(
        resp.headers().get("access-control-allow-origin").is_none(),
        "disallowed origin must not be echoed"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn update_account_changes_display_name(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let signup_resp = app
        .clone()
        .oneshot(native_signup_req(
            "user@example.com",
            "correct horse battery staple",
        ))
        .await
        .unwrap();
    let body: Value = read_json(signup_resp.into_body()).await;
    let secret = body["session"]["secret"].as_str().unwrap().to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/v1/auth/account")
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "display_name": "  Renamed  " }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let updated: Value = read_json(resp.into_body()).await;
    assert_eq!(updated["display_name"], "Renamed");

    // GET /me must reflect the new name.
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
    let me: Value = read_json(resp.into_body()).await;
    assert_eq!(me["account"]["display_name"], "Renamed");
}

#[sqlx::test(migrations = "./migrations")]
async fn update_account_rejects_empty_display_name(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let signup_resp = app
        .clone()
        .oneshot(native_signup_req(
            "user@example.com",
            "correct horse battery staple",
        ))
        .await
        .unwrap();
    let body: Value = read_json(signup_resp.into_body()).await;
    let secret = body["session"]["secret"].as_str().unwrap().to_string();

    let resp = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/v1/auth/account")
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "display_name": "   " }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn change_password_old_no_longer_works(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let signup_resp = app
        .clone()
        .oneshot(native_signup_req(
            "user@example.com",
            "correct horse battery staple",
        ))
        .await
        .unwrap();
    let body: Value = read_json(signup_resp.into_body()).await;
    let secret = body["session"]["secret"].as_str().unwrap().to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/password")
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "current_password": "correct horse battery staple",
                        "new_password": "another sturdy passphrase"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Old password no longer logs in.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/native/login")
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
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // New password does.
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/native/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "email": "user@example.com",
                        "password": "another sturdy passphrase"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[sqlx::test(migrations = "./migrations")]
async fn change_password_wrong_current_unauthorized(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let signup_resp = app
        .clone()
        .oneshot(native_signup_req(
            "user@example.com",
            "correct horse battery staple",
        ))
        .await
        .unwrap();
    let body: Value = read_json(signup_resp.into_body()).await;
    let secret = body["session"]["secret"].as_str().unwrap().to_string();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/password")
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "current_password": "wrong-current-password",
                        "new_password": "another sturdy passphrase"
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
async fn change_password_short_new_rejected(db: PgPool) {
    let app = router(AppState::new(db).with_rate_limit_auth(false));
    let signup_resp = app
        .clone()
        .oneshot(native_signup_req(
            "user@example.com",
            "correct horse battery staple",
        ))
        .await
        .unwrap();
    let body: Value = read_json(signup_resp.into_body()).await;
    let secret = body["session"]["secret"].as_str().unwrap().to_string();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/password")
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "current_password": "correct horse battery staple",
                        "new_password": "short"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
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

/// Native (bearer-returning) signup. Use when the test needs the session
/// secret in the response body — the browser endpoint only sets a cookie.
fn native_signup_req(email: &str, password: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/auth/native/signup")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "email": email,
                "password": password,
                "display_name": "User",
            })
            .to_string(),
        ))
        .unwrap()
}

async fn read_json(body: Body) -> Value {
    let bytes = to_bytes(body, MAX_BODY).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
