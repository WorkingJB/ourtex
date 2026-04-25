use ourtex_vault::{Document, DocumentId, PlainFileDriver, VaultDriver, Visibility};
use std::collections::BTreeMap;
use tempfile::TempDir;

fn sample_doc(id: &str, type_: &str, visibility: Visibility) -> Document {
    let fm = ourtex_vault::Frontmatter {
        id: DocumentId::new(id).unwrap(),
        type_: type_.to_string(),
        visibility,
        tags: vec!["test".to_string()],
        links: vec![],
        aliases: vec![],
        created: None,
        updated: None,
        source: None,
        principal: None,
        schema: None,
        extras: BTreeMap::new(),
    };
    Document {
        frontmatter: fm,
        body: format!("# {id}\n\nbody content\n"),
    }
}

#[tokio::test]
async fn write_then_read_roundtrips() {
    let tmp = TempDir::new().unwrap();
    let driver = PlainFileDriver::new(tmp.path());

    let id = DocumentId::new("me").unwrap();
    let doc = sample_doc("me", "identity", Visibility::Personal);
    driver.write(&id, &doc).await.unwrap();

    let loaded = driver.read(&id).await.unwrap();
    assert_eq!(loaded.frontmatter.id, id);
    assert_eq!(loaded.frontmatter.type_, "identity");
    assert!(matches!(loaded.frontmatter.visibility, Visibility::Personal));
    assert_eq!(loaded.body, doc.body);
}

#[tokio::test]
async fn list_returns_all_types_and_filters() {
    let tmp = TempDir::new().unwrap();
    let driver = PlainFileDriver::new(tmp.path());

    driver
        .write(
            &DocumentId::new("me").unwrap(),
            &sample_doc("me", "identity", Visibility::Personal),
        )
        .await
        .unwrap();
    driver
        .write(
            &DocumentId::new("g1").unwrap(),
            &sample_doc("g1", "goals", Visibility::Work),
        )
        .await
        .unwrap();
    driver
        .write(
            &DocumentId::new("g2").unwrap(),
            &sample_doc("g2", "goals", Visibility::Work),
        )
        .await
        .unwrap();

    let all = driver.list(None).await.unwrap();
    assert_eq!(all.len(), 3);

    let goals = driver.list(Some("goals")).await.unwrap();
    assert_eq!(goals.len(), 2);
    assert!(goals.iter().all(|e| e.type_ == "goals"));
}

#[tokio::test]
async fn list_skips_dot_ourtex_directory() {
    let tmp = TempDir::new().unwrap();
    let ourtex_dir = tmp.path().join(".ourtex");
    tokio::fs::create_dir_all(&ourtex_dir).await.unwrap();
    tokio::fs::write(ourtex_dir.join("config.json"), "{}").await.unwrap();

    let driver = PlainFileDriver::new(tmp.path());
    driver
        .write(
            &DocumentId::new("me").unwrap(),
            &sample_doc("me", "identity", Visibility::Personal),
        )
        .await
        .unwrap();

    let all = driver.list(None).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id.as_str(), "me");
}

#[tokio::test]
async fn read_missing_returns_not_found() {
    let tmp = TempDir::new().unwrap();
    let driver = PlainFileDriver::new(tmp.path());
    let err = driver
        .read(&DocumentId::new("nope").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, ourtex_vault::VaultError::NotFound(_)));
}

#[tokio::test]
async fn delete_removes_file() {
    let tmp = TempDir::new().unwrap();
    let driver = PlainFileDriver::new(tmp.path());
    let id = DocumentId::new("me").unwrap();
    driver
        .write(&id, &sample_doc("me", "identity", Visibility::Personal))
        .await
        .unwrap();
    driver.delete(&id).await.unwrap();

    let err = driver.read(&id).await.unwrap_err();
    assert!(matches!(err, ourtex_vault::VaultError::NotFound(_)));
}

#[tokio::test]
async fn write_rejects_id_mismatch() {
    let tmp = TempDir::new().unwrap();
    let driver = PlainFileDriver::new(tmp.path());
    let path_id = DocumentId::new("path-id").unwrap();
    let doc = sample_doc("frontmatter-id", "identity", Visibility::Personal);
    let err = driver.write(&path_id, &doc).await.unwrap_err();
    assert!(matches!(err, ourtex_vault::VaultError::InvalidId(_)));
}
