use chrono::Duration;
use ourtex_auth::{AuthError, IssueRequest, Limits, Mode, Scope, TokenService};
use ourtex_vault::Visibility;
use tempfile::TempDir;

fn request(labels: &[&str]) -> IssueRequest {
    IssueRequest {
        label: "test".to_string(),
        scope: Scope::new(labels.iter().copied()).unwrap(),
        mode: Mode::Read,
        limits: Limits::default(),
        ttl: None,
    }
}

#[tokio::test]
async fn issue_and_authenticate_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let svc = TokenService::open(tmp.path().join("tokens.json"))
        .await
        .unwrap();

    let issued = svc.issue(request(&["work", "public"])).await.unwrap();
    let auth = svc.authenticate(issued.secret.expose()).await.unwrap();

    assert_eq!(auth.id, issued.info.id);
    assert!(auth.scope.allows_label("work"));
    assert!(auth.scope.allows_label("public"));
    assert!(!auth.scope.allows_label("personal"));
    assert!(!auth.scope.allows_label("private"));
}

#[tokio::test]
async fn wrong_secret_rejected() {
    let tmp = TempDir::new().unwrap();
    let svc = TokenService::open(tmp.path().join("tokens.json"))
        .await
        .unwrap();
    let _ = svc.issue(request(&["work"])).await.unwrap();
    let err = svc.authenticate("otx_not-the-real-secret").await.unwrap_err();
    assert!(matches!(err, AuthError::UnknownToken));
}

#[tokio::test]
async fn malformed_secret_rejected() {
    let tmp = TempDir::new().unwrap();
    let svc = TokenService::open(tmp.path().join("tokens.json"))
        .await
        .unwrap();
    let err = svc.authenticate("not-a-ourtex-token").await.unwrap_err();
    assert!(matches!(err, AuthError::InvalidSecret));
}

#[tokio::test]
async fn revoked_token_rejected() {
    let tmp = TempDir::new().unwrap();
    let svc = TokenService::open(tmp.path().join("tokens.json"))
        .await
        .unwrap();
    let issued = svc.issue(request(&["work"])).await.unwrap();
    svc.revoke(&issued.info.id).await.unwrap();
    let err = svc.authenticate(issued.secret.expose()).await.unwrap_err();
    assert!(matches!(err, AuthError::Revoked));
}

#[tokio::test]
async fn expired_token_rejected() {
    let tmp = TempDir::new().unwrap();
    let svc = TokenService::open(tmp.path().join("tokens.json"))
        .await
        .unwrap();
    // Issue with a 1ns TTL: by the time authenticate runs it's expired.
    let mut req = request(&["work"]);
    req.ttl = Some(Duration::nanoseconds(1));
    let issued = svc.issue(req).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let err = svc.authenticate(issued.secret.expose()).await.unwrap_err();
    assert!(matches!(err, AuthError::Expired));
}

#[tokio::test]
async fn private_floor_is_enforced() {
    let tmp = TempDir::new().unwrap();
    let svc = TokenService::open(tmp.path().join("tokens.json"))
        .await
        .unwrap();

    // Token without `private` in scope.
    let issued = svc
        .issue(request(&["public", "work", "personal"]))
        .await
        .unwrap();
    let auth = svc.authenticate(issued.secret.expose()).await.unwrap();
    assert!(!auth.scope.allows(&Visibility::Private));
    assert!(!auth.scope.includes_private());
    assert!(auth.scope.allows(&Visibility::Personal));

    // Token WITH `private` in scope.
    let issued_priv = svc.issue(request(&["private"])).await.unwrap();
    let auth_priv = svc.authenticate(issued_priv.secret.expose()).await.unwrap();
    assert!(auth_priv.scope.allows(&Visibility::Private));
    assert!(auth_priv.scope.includes_private());
}

#[tokio::test]
async fn list_does_not_leak_hashes() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("tokens.json");
    let svc = TokenService::open(&path).await.unwrap();
    svc.issue(request(&["work"])).await.unwrap();

    let list = svc.list().await;
    assert_eq!(list.len(), 1);

    // The public info struct doesn't serialize a hash field.
    let json = serde_json::to_string(&list[0]).unwrap();
    assert!(!json.contains("argon2"));
    assert!(!json.contains("\"hash\""));
}

#[tokio::test]
async fn persists_across_reopen() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("tokens.json");

    let secret_str = {
        let svc = TokenService::open(&path).await.unwrap();
        let issued = svc.issue(request(&["work"])).await.unwrap();
        issued.secret.expose().to_string()
    };

    let svc2 = TokenService::open(&path).await.unwrap();
    let auth = svc2.authenticate(&secret_str).await.unwrap();
    assert!(auth.scope.allows_label("work"));
}

#[tokio::test]
async fn mark_used_updates_timestamp() {
    let tmp = TempDir::new().unwrap();
    let svc = TokenService::open(tmp.path().join("tokens.json"))
        .await
        .unwrap();
    let issued = svc.issue(request(&["work"])).await.unwrap();
    assert!(issued.info.last_used.is_none());

    let now = chrono::Utc::now();
    svc.mark_used(&issued.info.id, now).await.unwrap();

    let list = svc.list().await;
    assert_eq!(list[0].last_used, Some(now));
}
