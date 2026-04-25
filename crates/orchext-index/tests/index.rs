use chrono::NaiveDate;
use ourtex_index::{Index, ListFilter, SearchQuery};
use ourtex_vault::{Document, DocumentId, Frontmatter, PlainFileDriver, VaultDriver, Visibility};
use std::collections::BTreeMap;
use tempfile::TempDir;

fn doc(
    id: &str,
    type_: &str,
    visibility: Visibility,
    title: &str,
    body_extra: &str,
    tags: &[&str],
    links: &[&str],
    updated: Option<NaiveDate>,
) -> Document {
    let fm = Frontmatter {
        id: DocumentId::new(id).unwrap(),
        type_: type_.to_string(),
        visibility,
        tags: tags.iter().map(|s| s.to_string()).collect(),
        links: links.iter().map(|s| s.to_string()).collect(),
        aliases: vec![],
        created: None,
        updated,
        source: None,
        principal: None,
        schema: None,
        extras: BTreeMap::new(),
    };
    Document {
        frontmatter: fm,
        body: format!("# {title}\n\n{body_extra}\n"),
    }
}

#[tokio::test]
async fn reindex_from_vault_and_search() {
    let tmp = TempDir::new().unwrap();
    let vault = PlainFileDriver::new(tmp.path());

    vault
        .write(
            &DocumentId::new("pref-comms").unwrap(),
            &doc(
                "pref-comms",
                "preferences",
                Visibility::Work,
                "Communication style",
                "Prefer written async updates over meetings.",
                &["style", "work"],
                &[],
                Some(NaiveDate::from_ymd_opt(2026, 4, 18).unwrap()),
            ),
        )
        .await
        .unwrap();
    vault
        .write(
            &DocumentId::new("rel-jane").unwrap(),
            &doc(
                "rel-jane",
                "relationships",
                Visibility::Work,
                "Jane Smith",
                "My manager at Acme. Prefers written updates.",
                &["manager", "acme"],
                &["pref-comms"],
                Some(NaiveDate::from_ymd_opt(2026, 4, 18).unwrap()),
            ),
        )
        .await
        .unwrap();
    vault
        .write(
            &DocumentId::new("diary").unwrap(),
            &doc(
                "diary",
                "memories",
                Visibility::Private,
                "Diary",
                "Some private thoughts.",
                &["personal"],
                &[],
                None,
            ),
        )
        .await
        .unwrap();

    let idx_path = tmp.path().join(".ourtex").join("index.sqlite");
    let idx = Index::open(&idx_path).await.unwrap();
    let stats = idx.reindex_from(&vault).await.unwrap();
    assert_eq!(stats.documents, 3);
    assert_eq!(stats.links, 1);

    let hits = idx
        .search(SearchQuery {
            query: "written updates".to_string(),
            allowed_visibility: vec!["work".to_string(), "public".to_string()],
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|h| h.visibility == "work"));
}

#[tokio::test]
async fn search_respects_scope_filter_and_private_floor() {
    let tmp = TempDir::new().unwrap();
    let idx_path = tmp.path().join("index.sqlite");
    let idx = Index::open(&idx_path).await.unwrap();

    idx.upsert(
        "relationships",
        &doc(
            "work-doc",
            "relationships",
            Visibility::Work,
            "Work note",
            "confidential stuff about acme project",
            &["acme"],
            &[],
            None,
        ),
    )
    .await
    .unwrap();
    idx.upsert(
        "memories",
        &doc(
            "private-doc",
            "memories",
            Visibility::Private,
            "Private note",
            "confidential stuff about my health",
            &[],
            &[],
            None,
        ),
    )
    .await
    .unwrap();

    // With a scope that does NOT include `private`, we should never see the
    // private document — even when the search terms match.
    let scoped = idx
        .search(SearchQuery {
            query: "confidential".to_string(),
            allowed_visibility: vec!["work".to_string(), "public".to_string(), "personal".to_string()],
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].id, "work-doc");

    // With `private` explicitly in scope, both come back.
    let with_private = idx
        .search(SearchQuery {
            query: "confidential".to_string(),
            allowed_visibility: vec!["work".to_string(), "private".to_string()],
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(with_private.len(), 2);
}

#[tokio::test]
async fn list_filters_by_type_and_tag() {
    let tmp = TempDir::new().unwrap();
    let idx = Index::open(tmp.path().join("i.sqlite")).await.unwrap();

    for (id, type_, tags) in [
        ("g1", "goals", vec!["q2-2026"]),
        ("g2", "goals", vec!["q3-2026"]),
        ("r1", "relationships", vec!["q2-2026"]),
    ] {
        idx.upsert(
            type_,
            &doc(
                id,
                type_,
                Visibility::Work,
                id,
                "body",
                &tags.iter().copied().collect::<Vec<_>>(),
                &[],
                None,
            ),
        )
        .await
        .unwrap();
    }

    let just_goals = idx
        .list(ListFilter {
            types: vec!["goals".to_string()],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(just_goals.len(), 2);

    let q2_only = idx
        .list(ListFilter {
            tags: vec!["q2-2026".to_string()],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(q2_only.len(), 2);
    let ids: Vec<_> = q2_only.iter().map(|i| i.id.clone()).collect();
    assert!(ids.contains(&"g1".to_string()));
    assert!(ids.contains(&"r1".to_string()));
}

#[tokio::test]
async fn backlinks_and_outbound() {
    let tmp = TempDir::new().unwrap();
    let idx = Index::open(tmp.path().join("i.sqlite")).await.unwrap();

    idx.upsert("goals", &doc("g1", "goals", Visibility::Work, "g1", "b", &[], &["r1", "r2"], None))
        .await
        .unwrap();
    idx.upsert("relationships", &doc("r1", "relationships", Visibility::Work, "r1", "b", &[], &[], None))
        .await
        .unwrap();
    idx.upsert("relationships", &doc("r2", "relationships", Visibility::Work, "r2", "b", &[], &["r1"], None))
        .await
        .unwrap();

    let out = idx.outbound_links(&DocumentId::new("g1").unwrap()).await.unwrap();
    assert_eq!(out, vec!["r1".to_string(), "r2".to_string()]);

    let back = idx.backlinks(&DocumentId::new("r1").unwrap()).await.unwrap();
    assert_eq!(back, vec!["g1".to_string(), "r2".to_string()]);
}

#[tokio::test]
async fn remove_drops_from_all_tables_including_fts() {
    let tmp = TempDir::new().unwrap();
    let idx = Index::open(tmp.path().join("i.sqlite")).await.unwrap();

    idx.upsert(
        "goals",
        &doc("g1", "goals", Visibility::Work, "gone", "findable text", &["t"], &["other"], None),
    )
    .await
    .unwrap();
    idx.upsert(
        "goals",
        &doc("other", "goals", Visibility::Work, "other", "body", &[], &[], None),
    )
    .await
    .unwrap();

    // Present before remove.
    let hits = idx
        .search(SearchQuery {
            query: "findable".to_string(),
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);

    idx.remove(&DocumentId::new("g1").unwrap()).await.unwrap();

    // Gone from FTS.
    let hits = idx
        .search(SearchQuery {
            query: "findable".to_string(),
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(hits.is_empty());

    // Gone from outbound links.
    let out = idx.outbound_links(&DocumentId::new("g1").unwrap()).await.unwrap();
    assert!(out.is_empty());

    // Gone from list.
    let list = idx
        .list(ListFilter {
            types: vec!["goals".to_string()],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "other");
}

#[tokio::test]
async fn upsert_replaces_tags_and_links() {
    let tmp = TempDir::new().unwrap();
    let idx = Index::open(tmp.path().join("i.sqlite")).await.unwrap();

    idx.upsert(
        "goals",
        &doc("g1", "goals", Visibility::Work, "v1", "body", &["old-tag"], &["old-link"], None),
    )
    .await
    .unwrap();
    idx.upsert(
        "goals",
        &doc("g1", "goals", Visibility::Work, "v2", "body", &["new-tag"], &["new-link"], None),
    )
    .await
    .unwrap();

    let items = idx
        .list(ListFilter {
            tags: vec!["old-tag".to_string()],
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(items.is_empty(), "old tag should be gone");

    let new_items = idx
        .list(ListFilter {
            tags: vec!["new-tag".to_string()],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(new_items.len(), 1);

    let out = idx.outbound_links(&DocumentId::new("g1").unwrap()).await.unwrap();
    assert_eq!(out, vec!["new-link".to_string()]);
}
