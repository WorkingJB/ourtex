//! End-to-end Phase 3 platform Slice 2 tests:
//!   * teams + team_memberships CRUD
//!   * role gates (org-admin / team-manager / org-member)
//!   * `visibility = 'team'` document filter (member sees / non-member
//!     doesn't / org admin sees) and the strict
//!     `team_id ⇔ visibility = 'team'` coupling enforcement.
//!   * Org logo upload (admin-gated, size cap, magic-byte sniff, ETag).
//!
//! Hits real Postgres via `sqlx::test`. Skipped when DATABASE_URL isn't
//! set (cargo test will report 0 ran for this file in that case).

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use orchext_server::{config::DeploymentMode, router, AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

const MAX_BODY: usize = 1 << 20;

// ---------- teams CRUD ----------

#[sqlx::test(migrations = "./migrations")]
async fn admin_creates_and_lists_team(db: PgPool) {
    let app = self_hosted_router(db);
    let owner = signup(&app, "owner@example.com").await;
    let org_id = sole_org(&app, &owner).await;

    let team = create_team(&app, &owner, &org_id, "Marketing Ops").await;
    assert_eq!(team["name"], "Marketing Ops");
    assert_eq!(team["slug"], "marketing-ops");

    let teams = list_teams(&app, &owner, &org_id).await;
    let arr = teams["teams"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "Marketing Ops");
    // The viewer (the org owner) is not a team member by default.
    assert!(arr[0]["viewer_role"].is_null());
    assert_eq!(arr[0]["member_count"], 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn member_cannot_create_team(db: PgPool) {
    let app = self_hosted_router(db);
    let owner = signup(&app, "owner@example.com").await;
    let org_id = sole_org(&app, &owner).await;
    let member = approve_member(&app, &owner, &org_id, "alice@example.com").await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/orgs/{org_id}/teams"))
                .header("authorization", format!("Bearer {member}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"name": "x"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "./migrations")]
async fn duplicate_slug_conflicts(db: PgPool) {
    let app = self_hosted_router(db);
    let owner = signup(&app, "owner@example.com").await;
    let org_id = sole_org(&app, &owner).await;

    let _ = create_team(&app, &owner, &org_id, "Marketing Ops").await;

    // Same name → same slug → 409.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/orgs/{org_id}/teams"))
                .header("authorization", format!("Bearer {owner}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"name": "Marketing Ops"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

// ---------- team membership ----------

#[sqlx::test(migrations = "./migrations")]
async fn admin_adds_member_to_team(db: PgPool) {
    let app = self_hosted_router(db);
    let owner = signup(&app, "owner@example.com").await;
    let org_id = sole_org(&app, &owner).await;
    let team = create_team(&app, &owner, &org_id, "Eng").await;
    let team_id = team["id"].as_str().unwrap().to_string();

    let alice_secret = approve_member(&app, &owner, &org_id, "alice@example.com").await;
    let alice_account = me_account_id(&app, &alice_secret).await;

    let added = add_team_member(&app, &owner, &org_id, &team_id, &alice_account, "manager").await;
    assert_eq!(added["role"], "manager");
    assert_eq!(added["email"], "alice@example.com");

    let listed = list_team_members(&app, &owner, &org_id, &team_id).await;
    assert_eq!(listed["members"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn team_manager_can_add_their_own_team_members(db: PgPool) {
    let app = self_hosted_router(db);
    let owner = signup(&app, "owner@example.com").await;
    let org_id = sole_org(&app, &owner).await;
    let team = create_team(&app, &owner, &org_id, "Eng").await;
    let team_id = team["id"].as_str().unwrap().to_string();

    let alice = approve_member(&app, &owner, &org_id, "alice@example.com").await;
    let alice_id = me_account_id(&app, &alice).await;
    let _ = add_team_member(&app, &owner, &org_id, &team_id, &alice_id, "manager").await;

    let bob = approve_member(&app, &owner, &org_id, "bob@example.com").await;
    let bob_id = me_account_id(&app, &bob).await;

    // Manager (alice) can add bob to her own team.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/orgs/{org_id}/teams/{team_id}/members"))
                .header("authorization", format!("Bearer {alice}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"account_id": bob_id, "role": "member"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[sqlx::test(migrations = "./migrations")]
async fn team_member_cannot_manage_team(db: PgPool) {
    let app = self_hosted_router(db);
    let owner = signup(&app, "owner@example.com").await;
    let org_id = sole_org(&app, &owner).await;
    let team = create_team(&app, &owner, &org_id, "Eng").await;
    let team_id = team["id"].as_str().unwrap().to_string();

    let alice = approve_member(&app, &owner, &org_id, "alice@example.com").await;
    let alice_id = me_account_id(&app, &alice).await;
    let _ = add_team_member(&app, &owner, &org_id, &team_id, &alice_id, "member").await;

    let bob = approve_member(&app, &owner, &org_id, "bob@example.com").await;
    let bob_id = me_account_id(&app, &bob).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/orgs/{org_id}/teams/{team_id}/members"))
                .header("authorization", format!("Bearer {alice}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"account_id": bob_id, "role": "member"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ---------- team-visibility doc filter ----------

#[sqlx::test(migrations = "./migrations")]
async fn team_doc_visible_only_to_team_members(db: PgPool) {
    let app = self_hosted_router(db);
    let owner = signup(&app, "owner@example.com").await;
    let owner_orgs = get_orgs(&app, &owner).await;
    let org_id = owner_orgs["memberships"][0]["org_id"]
        .as_str()
        .unwrap()
        .to_string();
    let tenant_id = owner_orgs["memberships"][0]["tenant_id"]
        .as_str()
        .unwrap()
        .to_string();
    let team = create_team(&app, &owner, &org_id, "Eng").await;
    let team_id = team["id"].as_str().unwrap().to_string();

    let alice = approve_member(&app, &owner, &org_id, "alice@example.com").await;
    let bob = approve_member(&app, &owner, &org_id, "bob@example.com").await;
    let alice_id = me_account_id(&app, &alice).await;
    // Alice is a manager of Eng; Bob is not a member of any team.
    let _ = add_team_member(&app, &owner, &org_id, &team_id, &alice_id, "manager").await;

    // Alice writes a team-visibility doc.
    write_team_doc(&app, &alice, &tenant_id, "eng-handbook", &team_id).await;

    // Alice (team member): sees it.
    let alice_list = doc_list(&app, &alice, &tenant_id).await;
    assert!(
        alice_list
            .iter()
            .any(|e| e["doc_id"] == "eng-handbook"),
        "team manager should see her team's doc"
    );

    // Bob (org member, not a team member): does NOT see it.
    let bob_list = doc_list(&app, &bob, &tenant_id).await;
    assert!(
        !bob_list.iter().any(|e| e["doc_id"] == "eng-handbook"),
        "non-team-member should not see team docs"
    );

    // Owner (org admin): sees it.
    let owner_list = doc_list(&app, &owner, &tenant_id).await;
    assert!(
        owner_list
            .iter()
            .any(|e| e["doc_id"] == "eng-handbook"),
        "org owner should see all team docs"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn team_id_filter_narrows_listing(db: PgPool) {
    let app = self_hosted_router(db);
    let owner = signup(&app, "owner@example.com").await;
    let owner_orgs = get_orgs(&app, &owner).await;
    let org_id = owner_orgs["memberships"][0]["org_id"]
        .as_str()
        .unwrap()
        .to_string();
    let tenant_id = owner_orgs["memberships"][0]["tenant_id"]
        .as_str()
        .unwrap()
        .to_string();
    let eng = create_team(&app, &owner, &org_id, "Eng").await;
    let eng_id = eng["id"].as_str().unwrap().to_string();
    let product = create_team(&app, &owner, &org_id, "Product").await;
    let product_id = product["id"].as_str().unwrap().to_string();

    write_team_doc(&app, &owner, &tenant_id, "eng-handbook", &eng_id).await;
    write_team_doc(&app, &owner, &tenant_id, "product-roadmap", &product_id).await;

    // No filter → both team docs visible to the owner (org admin).
    let all = doc_list(&app, &owner, &tenant_id).await;
    let titles: Vec<&str> = all
        .iter()
        .filter_map(|e| e["doc_id"].as_str())
        .collect();
    assert!(titles.contains(&"eng-handbook"));
    assert!(titles.contains(&"product-roadmap"));

    // Filter to Eng → only the Eng doc.
    let eng_only = doc_list_with_query(
        &app,
        &owner,
        &tenant_id,
        &format!("team_id={eng_id}"),
    )
    .await;
    let eng_titles: Vec<&str> = eng_only
        .iter()
        .filter_map(|e| e["doc_id"].as_str())
        .collect();
    assert_eq!(eng_titles, vec!["eng-handbook"]);

    // Filter to Product → only the Product doc.
    let product_only = doc_list_with_query(
        &app,
        &owner,
        &tenant_id,
        &format!("team_id={product_id}"),
    )
    .await;
    let product_titles: Vec<&str> = product_only
        .iter()
        .filter_map(|e| e["doc_id"].as_str())
        .collect();
    assert_eq!(product_titles, vec!["product-roadmap"]);

    // Non-member filtering by team_id sees nothing — team docs are
    // visibility-gated regardless of the filter, so no info leaks
    // about which teams have content.
    let bob = approve_member(&app, &owner, &org_id, "bob@example.com").await;
    let bob_filtered = doc_list_with_query(
        &app,
        &bob,
        &tenant_id,
        &format!("team_id={eng_id}"),
    )
    .await;
    assert!(
        bob_filtered.is_empty(),
        "non-member must see empty list when filtering by team_id"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn team_doc_requires_team_id(db: PgPool) {
    let app = self_hosted_router(db);
    let owner = signup(&app, "owner@example.com").await;
    let orgs = get_orgs(&app, &owner).await;
    let tenant_id = orgs["memberships"][0]["tenant_id"]
        .as_str()
        .unwrap()
        .to_string();

    // visibility=team without team_id → 400 InvalidArgument.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/v1/t/{tenant_id}/vault/docs/eng-handbook"))
                .header("authorization", format!("Bearer {owner}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"source": team_doc_source("eng-handbook")}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn non_team_doc_rejects_team_id(db: PgPool) {
    let app = self_hosted_router(db);
    let owner = signup(&app, "owner@example.com").await;
    let orgs = get_orgs(&app, &owner).await;
    let org_id = orgs["memberships"][0]["org_id"].as_str().unwrap().to_string();
    let tenant_id = orgs["memberships"][0]["tenant_id"]
        .as_str()
        .unwrap()
        .to_string();
    let team = create_team(&app, &owner, &org_id, "Eng").await;
    let team_id = team["id"].as_str().unwrap();

    // visibility=org with team_id → 400 InvalidArgument.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/v1/t/{tenant_id}/vault/docs/mission"))
                .header("authorization", format!("Bearer {owner}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "source": ORG_DOC_SOURCE,
                        "team_id": team_id,
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ---------- logo upload ----------

#[sqlx::test(migrations = "./migrations")]
async fn admin_uploads_and_serves_logo(db: PgPool) {
    let app = self_hosted_router(db);
    let owner = signup(&app, "owner@example.com").await;
    let orgs = get_orgs(&app, &owner).await;
    let org_id = orgs["memberships"][0]["org_id"].as_str().unwrap().to_string();

    // Minimal valid PNG (8-byte signature + tiny IEND chunk-ish body).
    // The server only sniffs the first 8 magic bytes; payload size
    // doesn't matter for the validation path.
    let png = vec![
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xDE, 0xAD, 0xBE, 0xEF,
    ];
    let upload = upload_logo(&app, &owner, &org_id, &png, "logo.png", "image/png").await;
    assert_eq!(upload["content_type"], "image/png");
    assert_eq!(upload["bytes"], png.len());
    assert_eq!(
        upload["logo_url"].as_str().unwrap(),
        format!(
            "/v1/orgs/{org_id}/logo?v={}",
            &upload["sha256"].as_str().unwrap()[..16]
        )
    );

    // GET round-trips the bytes.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/orgs/{org_id}/logo"))
                .header("authorization", format!("Bearer {owner}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "image/png"
    );
    let etag = resp.headers().get("etag").unwrap().to_str().unwrap().to_string();
    let bytes = to_bytes(resp.into_body(), MAX_BODY).await.unwrap();
    assert_eq!(bytes.as_ref(), png.as_slice());

    // ETag round-trip: re-request with If-None-Match → 304.
    let resp2 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/orgs/{org_id}/logo"))
                .header("authorization", format!("Bearer {owner}"))
                .header("if-none-match", &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::NOT_MODIFIED);
}

#[sqlx::test(migrations = "./migrations")]
async fn logo_upload_rejects_non_image(db: PgPool) {
    let app = self_hosted_router(db);
    let owner = signup(&app, "owner@example.com").await;
    let orgs = get_orgs(&app, &owner).await;
    let org_id = orgs["memberships"][0]["org_id"].as_str().unwrap().to_string();

    let payload = b"not an image at all";
    let resp = upload_logo_raw(&app, &owner, &org_id, payload, "evil.svg", "image/svg+xml").await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn member_cannot_upload_logo(db: PgPool) {
    let app = self_hosted_router(db);
    let owner = signup(&app, "owner@example.com").await;
    let orgs = get_orgs(&app, &owner).await;
    let org_id = orgs["memberships"][0]["org_id"].as_str().unwrap().to_string();
    let member = approve_member(&app, &owner, &org_id, "alice@example.com").await;

    let png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0];
    let resp = upload_logo_raw(&app, &member, &org_id, &png, "logo.png", "image/png").await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ---------- helpers ----------

fn self_hosted_router(db: PgPool) -> axum::Router {
    router(
        AppState::new(db)
            .with_rate_limit_auth(false)
            .with_deployment_mode(DeploymentMode::SelfHosted),
    )
}

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
    assert_eq!(resp.status(), StatusCode::CREATED, "signup failed for {email}");
    let body: Value = read_json(resp.into_body()).await;
    body["session"]["secret"].as_str().unwrap().to_string()
}

async fn me_account_id(app: &axum::Router, bearer: &str) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/auth/me")
                .header("authorization", format!("Bearer {bearer}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: Value = read_json(resp.into_body()).await;
    body["account"]["id"].as_str().unwrap().to_string()
}

async fn get_orgs(app: &axum::Router, bearer: &str) -> Value {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/orgs")
                .header("authorization", format!("Bearer {bearer}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    read_json(resp.into_body()).await
}

async fn sole_org(app: &axum::Router, bearer: &str) -> String {
    let orgs = get_orgs(app, bearer).await;
    orgs["memberships"][0]["org_id"].as_str().unwrap().to_string()
}

async fn approve_member(
    app: &axum::Router,
    owner_bearer: &str,
    org_id: &str,
    email: &str,
) -> String {
    let secret = signup(app, email).await;
    let account_id = me_account_id(app, &secret).await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/orgs/{org_id}/pending/{account_id}/approve"
                ))
                .header("authorization", format!("Bearer {owner_bearer}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"role": "member"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.status().is_success(), "approve failed: {:?}", resp.status());
    secret
}

async fn create_team(
    app: &axum::Router,
    bearer: &str,
    org_id: &str,
    name: &str,
) -> Value {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/orgs/{org_id}/teams"))
                .header("authorization", format!("Bearer {bearer}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"name": name}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body = read_json(resp.into_body()).await;
    assert!(status.is_success(), "create_team {status}: {body}");
    body
}

async fn list_teams(app: &axum::Router, bearer: &str, org_id: &str) -> Value {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/orgs/{org_id}/teams"))
                .header("authorization", format!("Bearer {bearer}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    read_json(resp.into_body()).await
}

async fn add_team_member(
    app: &axum::Router,
    bearer: &str,
    org_id: &str,
    team_id: &str,
    account_id: &str,
    role: &str,
) -> Value {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/orgs/{org_id}/teams/{team_id}/members"
                ))
                .header("authorization", format!("Bearer {bearer}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"account_id": account_id, "role": role}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body = read_json(resp.into_body()).await;
    assert!(status.is_success(), "add_team_member {status}: {body}");
    body
}

async fn list_team_members(
    app: &axum::Router,
    bearer: &str,
    org_id: &str,
    team_id: &str,
) -> Value {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/v1/orgs/{org_id}/teams/{team_id}/members"
                ))
                .header("authorization", format!("Bearer {bearer}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    read_json(resp.into_body()).await
}

async fn write_team_doc(
    app: &axum::Router,
    bearer: &str,
    tenant_id: &str,
    doc_id: &str,
    team_id: &str,
) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/v1/t/{tenant_id}/vault/docs/{doc_id}"))
                .header("authorization", format!("Bearer {bearer}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "source": team_doc_source(doc_id),
                        "team_id": team_id,
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "write_team_doc unexpected status: {:?}",
        resp.status()
    );
}

async fn doc_list(app: &axum::Router, bearer: &str, tenant_id: &str) -> Vec<Value> {
    doc_list_with_query(app, bearer, tenant_id, "").await
}

async fn doc_list_with_query(
    app: &axum::Router,
    bearer: &str,
    tenant_id: &str,
    query: &str,
) -> Vec<Value> {
    let uri = if query.is_empty() {
        format!("/v1/t/{tenant_id}/vault/docs")
    } else {
        format!("/v1/t/{tenant_id}/vault/docs?{query}")
    };
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
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = read_json(resp.into_body()).await;
    body["entries"].as_array().cloned().unwrap_or_default()
}

async fn upload_logo(
    app: &axum::Router,
    bearer: &str,
    org_id: &str,
    bytes: &[u8],
    filename: &str,
    mime: &str,
) -> Value {
    let resp = upload_logo_raw(app, bearer, org_id, bytes, filename, mime).await;
    let status = resp.status();
    let body = read_json(resp.into_body()).await;
    assert!(status.is_success(), "upload_logo {status}: {body}");
    body
}

async fn upload_logo_raw(
    app: &axum::Router,
    bearer: &str,
    org_id: &str,
    bytes: &[u8],
    filename: &str,
    mime: &str,
) -> axum::http::Response<Body> {
    let boundary = "ORCHEXTBOUNDARY42";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {mime}\r\n\r\n").as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/orgs/{org_id}/logo"))
                .header("authorization", format!("Bearer {bearer}"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn read_json(body: Body) -> Value {
    let bytes = to_bytes(body, MAX_BODY).await.unwrap();
    if bytes.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn team_doc_source(doc_id: &str) -> String {
    format!(
        "---\n\
id: {doc_id}\n\
type: handbook\n\
visibility: team\n\
tags: []\n\
links: []\n\
updated: 2026-04-27\n\
---\n\
# Eng Handbook\n\
\n\
internal team docs.\n",
    )
}

const ORG_DOC_SOURCE: &str = "---\n\
id: mission\n\
type: org\n\
visibility: org\n\
tags: []\n\
links: []\n\
updated: 2026-04-27\n\
---\n\
# Mission\n\
\n\
We orchestrate context across teams.\n";
