//! Phase 3 platform Slice 1 follow-up: org_invitations email-pre-
//! approval flow. Admin adds an email + role; when that email signs
//! up, the membership is materialized directly with no awaiting-
//! approval gate.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use orchext_server::{config::DeploymentMode, router, AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

const MAX_BODY: usize = 1 << 20;

#[sqlx::test(migrations = "./migrations")]
async fn invited_email_skips_pending_on_signup(db: PgPool) {
    let app = router(
        AppState::new(db)
            .with_rate_limit_auth(false)
            .with_deployment_mode(DeploymentMode::SelfHosted),
    );

    let owner = signup(&app, "owner@example.com").await;
    let owner_orgs = json_get(&app, &owner, "/v1/orgs").await;
    let org_id = owner_orgs["memberships"][0]["org_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Invite alice@example.com as a member.
    let invite = post_json(
        &app,
        &owner,
        &format!("/v1/orgs/{org_id}/invitations"),
        json!({"email": "alice@example.com", "role": "member"}),
    )
    .await;
    assert_eq!(invite["email"], "alice@example.com");
    assert_eq!(invite["role"], "member");
    assert!(invite["redeemed_at"].is_null());

    // Alice signs up — should land directly as a member, no pending row.
    let alice = signup(&app, "alice@example.com").await;
    let alice_orgs = json_get(&app, &alice, "/v1/orgs").await;
    let memberships = alice_orgs["memberships"].as_array().unwrap();
    assert_eq!(memberships.len(), 1, "invited signup gets the membership directly");
    assert_eq!(memberships[0]["role"], "member");
    assert_eq!(memberships[0]["org_id"], org_id);
    assert!(
        alice_orgs["pending"].as_array().unwrap().is_empty(),
        "no pending row should exist for an invited signup"
    );

    // The invitation is now redeemed.
    let list = json_get(
        &app,
        &owner,
        &format!("/v1/orgs/{org_id}/invitations?status=redeemed"),
    )
    .await;
    assert_eq!(list["invitations"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn invitation_email_match_is_case_insensitive(db: PgPool) {
    let app = router(
        AppState::new(db)
            .with_rate_limit_auth(false)
            .with_deployment_mode(DeploymentMode::SelfHosted),
    );

    let owner = signup(&app, "owner@example.com").await;
    let owner_orgs = json_get(&app, &owner, "/v1/orgs").await;
    let org_id = owner_orgs["memberships"][0]["org_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Invite with mixed case; signup with lowercase still matches.
    let _ = post_json(
        &app,
        &owner,
        &format!("/v1/orgs/{org_id}/invitations"),
        json!({"email": "Alice@Example.com"}),
    )
    .await;

    let alice = signup(&app, "alice@example.com").await;
    let alice_orgs = json_get(&app, &alice, "/v1/orgs").await;
    assert_eq!(alice_orgs["memberships"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn duplicate_open_invitation_409s(db: PgPool) {
    let app = router(
        AppState::new(db)
            .with_rate_limit_auth(false)
            .with_deployment_mode(DeploymentMode::SelfHosted),
    );

    let owner = signup(&app, "owner@example.com").await;
    let owner_orgs = json_get(&app, &owner, "/v1/orgs").await;
    let org_id = owner_orgs["memberships"][0]["org_id"]
        .as_str()
        .unwrap()
        .to_string();

    let _ = post_json(
        &app,
        &owner,
        &format!("/v1/orgs/{org_id}/invitations"),
        json!({"email": "alice@example.com"}),
    )
    .await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/orgs/{org_id}/invitations"))
                .header("authorization", format!("Bearer {owner}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"email": "alice@example.com"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_invitation_removes_open_row(db: PgPool) {
    let app = router(
        AppState::new(db)
            .with_rate_limit_auth(false)
            .with_deployment_mode(DeploymentMode::SelfHosted),
    );

    let owner = signup(&app, "owner@example.com").await;
    let owner_orgs = json_get(&app, &owner, "/v1/orgs").await;
    let org_id = owner_orgs["memberships"][0]["org_id"]
        .as_str()
        .unwrap()
        .to_string();

    let inv = post_json(
        &app,
        &owner,
        &format!("/v1/orgs/{org_id}/invitations"),
        json!({"email": "alice@example.com"}),
    )
    .await;
    let invitation_id = inv["id"].as_str().unwrap().to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/v1/orgs/{org_id}/invitations/{invitation_id}"
                ))
                .header("authorization", format!("Bearer {owner}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let list = json_get(
        &app,
        &owner,
        &format!("/v1/orgs/{org_id}/invitations"),
    )
    .await;
    assert!(list["invitations"].as_array().unwrap().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn member_cannot_create_invitation(db: PgPool) {
    let app = router(
        AppState::new(db)
            .with_rate_limit_auth(false)
            .with_deployment_mode(DeploymentMode::SelfHosted),
    );

    let owner = signup(&app, "owner@example.com").await;
    let owner_orgs = json_get(&app, &owner, "/v1/orgs").await;
    let org_id = owner_orgs["memberships"][0]["org_id"]
        .as_str()
        .unwrap()
        .to_string();
    let member = approve_member(&app, &owner, &org_id, "member@example.com").await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/orgs/{org_id}/invitations"))
                .header("authorization", format!("Bearer {member}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"email": "alice@example.com"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ---------- helpers ----------

async fn signup(app: &axum::Router, email: &str) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/native/signup")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "email": email,
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
    body["session"]["secret"].as_str().unwrap().to_string()
}

async fn approve_member(
    app: &axum::Router,
    owner_bearer: &str,
    org_id: &str,
    email: &str,
) -> String {
    let bearer = signup(app, email).await;
    let me = json_get(app, &bearer, "/v1/auth/me").await;
    let account_id = me["account"]["id"].as_str().unwrap().to_string();
    let _ = post_json(
        app,
        owner_bearer,
        &format!("/v1/orgs/{org_id}/pending/{account_id}/approve"),
        json!({"role": "member"}),
    )
    .await;
    bearer
}

async fn json_get(app: &axum::Router, bearer: &str, uri: &str) -> Value {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .header("authorization", format!("Bearer {bearer}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "GET {uri} unexpected status");
    read_json(resp.into_body()).await
}

async fn post_json(app: &axum::Router, bearer: &str, uri: &str, body: Value) -> Value {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("authorization", format!("Bearer {bearer}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body = read_json(resp.into_body()).await;
    assert!(status.is_success(), "POST {uri} got {status}: {body}");
    body
}

async fn read_json(body: Body) -> Value {
    let bytes = to_bytes(body, MAX_BODY).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
