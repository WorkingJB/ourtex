use ourtex_audit::{verify, Actor, AuditError, AuditRecord, AuditWriter, Iter, Outcome};
use tempfile::TempDir;

fn record(action: &str, actor: Actor, outcome: Outcome) -> AuditRecord {
    AuditRecord {
        actor,
        action: action.to_string(),
        document_id: Some("doc-1".to_string()),
        scope_used: vec!["work".to_string()],
        outcome,
    }
}

#[tokio::test]
async fn appends_and_verifies() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("audit.log");
    let writer = AuditWriter::open(&path).await.unwrap();

    let e0 = writer
        .append(record("context.read", Actor::Owner, Outcome::Ok))
        .await
        .unwrap();
    let e1 = writer
        .append(record(
            "context.read",
            Actor::Token("abc".to_string()),
            Outcome::Ok,
        ))
        .await
        .unwrap();
    let e2 = writer
        .append(record(
            "context.read",
            Actor::Token("abc".to_string()),
            Outcome::Denied,
        ))
        .await
        .unwrap();

    assert_eq!(e0.seq, 0);
    assert_eq!(e1.seq, 1);
    assert_eq!(e2.seq, 2);
    assert_eq!(e1.prev_hash, e0.hash);
    assert_eq!(e2.prev_hash, e1.hash);

    let report = verify(&path).await.unwrap();
    assert_eq!(report.total_entries, 3);
    assert_eq!(report.last_seq, Some(2));
    assert_eq!(report.last_hash.as_deref(), Some(e2.hash.as_str()));
}

#[tokio::test]
async fn reopen_preserves_chain() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("audit.log");
    {
        let writer = AuditWriter::open(&path).await.unwrap();
        writer
            .append(record("a", Actor::Owner, Outcome::Ok))
            .await
            .unwrap();
        writer
            .append(record("b", Actor::Owner, Outcome::Ok))
            .await
            .unwrap();
    }
    let writer = AuditWriter::open(&path).await.unwrap();
    let e = writer
        .append(record("c", Actor::Owner, Outcome::Ok))
        .await
        .unwrap();
    assert_eq!(e.seq, 2);

    let report = verify(&path).await.unwrap();
    assert_eq!(report.total_entries, 3);
    assert_eq!(report.last_seq, Some(2));
}

#[tokio::test]
async fn detects_tamper() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("audit.log");
    let writer = AuditWriter::open(&path).await.unwrap();
    writer
        .append(record("a", Actor::Owner, Outcome::Ok))
        .await
        .unwrap();
    writer
        .append(record("b", Actor::Owner, Outcome::Ok))
        .await
        .unwrap();
    writer
        .append(record("c", Actor::Owner, Outcome::Ok))
        .await
        .unwrap();
    drop(writer);

    // Corrupt the middle entry by flipping its action.
    let contents = tokio::fs::read_to_string(&path).await.unwrap();
    let corrupted = contents.replacen("\"action\":\"b\"", "\"action\":\"B\"", 1);
    tokio::fs::write(&path, corrupted).await.unwrap();

    let err = verify(&path).await.unwrap_err();
    match err {
        AuditError::ChainBroken { seq, .. } => assert_eq!(seq, 1),
        other => panic!("expected ChainBroken, got {other:?}"),
    }
}

#[tokio::test]
async fn empty_log_verifies() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("audit.log");
    tokio::fs::File::create(&path).await.unwrap();
    let report = verify(&path).await.unwrap();
    assert_eq!(report.total_entries, 0);
    assert_eq!(report.last_seq, None);
    assert_eq!(report.last_hash, None);
}

#[tokio::test]
async fn iter_yields_entries_in_order() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("audit.log");
    let writer = AuditWriter::open(&path).await.unwrap();
    for action in ["a", "b", "c"] {
        writer
            .append(record(action, Actor::Owner, Outcome::Ok))
            .await
            .unwrap();
    }

    let mut iter = Iter::open(&path).await.unwrap();
    let mut actions = Vec::new();
    while let Some(entry) = iter.next().await.unwrap() {
        actions.push(entry.action);
    }
    assert_eq!(actions, vec!["a", "b", "c"]);
}
