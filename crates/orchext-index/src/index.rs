use crate::error::{IndexError, Result};
use crate::query::{ListFilter, ListItem, SearchHit, SearchQuery};
use crate::schema::SCHEMA_SQL;
use crate::title::extract_title;
use chrono::NaiveDate;
use ourtex_vault::{Document, DocumentId, VaultDriver};
use rusqlite::{params_from_iter, types::Value, Connection};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub struct IndexStats {
    pub documents: u64,
    pub tags: u64,
    pub links: u64,
}

pub struct Index {
    path: PathBuf,
    conn: Arc<Mutex<Connection>>,
}

impl Index {
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        let owned_path = path.clone();
        let conn = tokio::task::spawn_blocking(move || -> Result<Connection> {
            let conn = Connection::open(&owned_path)?;
            conn.execute_batch(SCHEMA_SQL)?;
            Ok(conn)
        })
        .await
        .map_err(|e| IndexError::Join(e.to_string()))??;
        Ok(Self {
            path,
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn clear(&self) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.lock().unwrap();
            conn.execute_batch(
                "DELETE FROM search; DELETE FROM links; DELETE FROM tags; DELETE FROM documents;",
            )?;
            Ok(())
        })
        .await
        .map_err(|e| IndexError::Join(e.to_string()))??;
        Ok(())
    }

    pub async fn upsert(&self, type_: &str, doc: &Document) -> Result<()> {
        let id = doc.frontmatter.id.to_string();
        let type_owned = type_.to_string();
        let visibility = doc.frontmatter.visibility.as_label().to_string();
        let title = extract_title(&doc.body, &id);
        let body = doc.body.clone();
        let created = doc.frontmatter.created.map(date_to_iso);
        let updated = doc.frontmatter.updated.map(date_to_iso);
        let tags = doc.frontmatter.tags.clone();
        let links = doc.frontmatter.links.clone();

        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut conn = conn.lock().unwrap();
            let tx = conn.transaction()?;
            upsert_tx(&tx, &id, &type_owned, &visibility, &title, &body, created.as_deref(), updated.as_deref(), &tags, &links)?;
            tx.commit()?;
            Ok(())
        })
        .await
        .map_err(|e| IndexError::Join(e.to_string()))??;
        Ok(())
    }

    pub async fn remove(&self, id: &DocumentId) -> Result<()> {
        let id = id.to_string();
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut conn = conn.lock().unwrap();
            let tx = conn.transaction()?;
            // ON DELETE CASCADE handles tags and links via foreign key.
            tx.execute("DELETE FROM documents WHERE id = ?1", [&id])?;
            tx.execute("DELETE FROM search WHERE id = ?1", [&id])?;
            tx.commit()?;
            Ok(())
        })
        .await
        .map_err(|e| IndexError::Join(e.to_string()))??;
        Ok(())
    }

    pub async fn reindex_from(&self, driver: &dyn VaultDriver) -> Result<IndexStats> {
        let entries = driver
            .list(None)
            .await
            .map_err(|e| IndexError::Vault(e.to_string()))?;
        self.clear().await?;
        let mut stats = IndexStats::default();
        for entry in entries {
            let doc = driver
                .read(&entry.id)
                .await
                .map_err(|e| IndexError::Vault(e.to_string()))?;
            stats.tags += doc.frontmatter.tags.len() as u64;
            stats.links += doc.frontmatter.links.len() as u64;
            self.upsert(&entry.type_, &doc).await?;
            stats.documents += 1;
        }
        Ok(stats)
    }

    pub async fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<SearchHit>> {
            let conn = conn.lock().unwrap();
            run_search(&conn, query)
        })
        .await
        .map_err(|e| IndexError::Join(e.to_string()))?
    }

    pub async fn list(&self, filter: ListFilter) -> Result<Vec<ListItem>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<ListItem>> {
            let conn = conn.lock().unwrap();
            run_list(&conn, filter)
        })
        .await
        .map_err(|e| IndexError::Join(e.to_string()))?
    }

    pub async fn backlinks(&self, id: &DocumentId) -> Result<Vec<String>> {
        let id = id.to_string();
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT source_id FROM links WHERE target_id = ?1 ORDER BY source_id",
            )?;
            let rows = stmt
                .query_map([&id], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .map_err(|e| IndexError::Join(e.to_string()))?
    }

    pub async fn outbound_links(&self, id: &DocumentId) -> Result<Vec<String>> {
        let id = id.to_string();
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT target_id FROM links WHERE source_id = ?1 ORDER BY target_id",
            )?;
            let rows = stmt
                .query_map([&id], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .map_err(|e| IndexError::Join(e.to_string()))?
    }

    /// Every `(source_id, target_id)` link row. Used by the desktop
    /// graph view to render the whole link graph in one trip.
    pub async fn all_edges(&self) -> Result<Vec<(String, String)>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<(String, String)>> {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT source_id, target_id FROM links ORDER BY source_id, target_id",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .map_err(|e| IndexError::Join(e.to_string()))?
    }
}

fn date_to_iso(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn parse_iso_date(s: Option<String>) -> Option<NaiveDate> {
    s.as_deref()
        .and_then(|v| NaiveDate::parse_from_str(v, "%Y-%m-%d").ok())
}

#[allow(clippy::too_many_arguments)]
fn upsert_tx(
    tx: &rusqlite::Transaction<'_>,
    id: &str,
    type_: &str,
    visibility: &str,
    title: &str,
    body: &str,
    created: Option<&str>,
    updated: Option<&str>,
    tags: &[String],
    links: &[String],
) -> Result<()> {
    tx.execute(
        "INSERT INTO documents(id, type, visibility, title, body, created, updated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
             type = excluded.type,
             visibility = excluded.visibility,
             title = excluded.title,
             body = excluded.body,
             created = excluded.created,
             updated = excluded.updated",
        rusqlite::params![id, type_, visibility, title, body, created, updated],
    )?;

    tx.execute("DELETE FROM tags WHERE document_id = ?1", [id])?;
    {
        let mut insert_tag =
            tx.prepare("INSERT OR IGNORE INTO tags(document_id, tag) VALUES (?1, ?2)")?;
        for tag in tags {
            insert_tag.execute(rusqlite::params![id, tag])?;
        }
    }

    tx.execute("DELETE FROM links WHERE source_id = ?1", [id])?;
    {
        let mut insert_link =
            tx.prepare("INSERT OR IGNORE INTO links(source_id, target_id) VALUES (?1, ?2)")?;
        for target in links {
            insert_link.execute(rusqlite::params![id, target])?;
        }
    }

    tx.execute("DELETE FROM search WHERE id = ?1", [id])?;
    tx.execute(
        "INSERT INTO search(id, title, body) VALUES (?1, ?2, ?3)",
        rusqlite::params![id, title, body],
    )?;

    Ok(())
}

fn run_search(conn: &Connection, query: SearchQuery) -> Result<Vec<SearchHit>> {
    let mut sql = String::from(
        "SELECT d.id, d.type, d.visibility, d.title, d.updated, \
                snippet(search, 2, '', '', '…', 12) AS snip, \
                bm25(search) AS score \
         FROM search JOIN documents d ON d.id = search.id \
         WHERE search MATCH ?1",
    );
    let mut params: Vec<Value> = vec![Value::Text(query.query.clone())];

    if !query.types.is_empty() {
        sql.push_str(" AND d.type IN (");
        push_placeholder_list(&mut sql, query.types.len(), &mut params, &query.types);
        sql.push(')');
    }
    if !query.allowed_visibility.is_empty() {
        sql.push_str(" AND d.visibility IN (");
        push_placeholder_list(
            &mut sql,
            query.allowed_visibility.len(),
            &mut params,
            &query.allowed_visibility,
        );
        sql.push(')');
    }
    if !query.tags.is_empty() {
        sql.push_str(
            " AND d.id IN (SELECT document_id FROM tags WHERE tag IN (",
        );
        push_placeholder_list(&mut sql, query.tags.len(), &mut params, &query.tags);
        sql.push_str("))");
    }
    if let Some(date) = query.updated_since {
        sql.push_str(" AND d.updated >= ?");
        params.push(Value::Text(date_to_iso(date)));
        let n = params.len();
        sql.push_str(&n.to_string());
    }
    sql.push_str(" ORDER BY score LIMIT ?");
    let n = params.len() + 1;
    sql.push_str(&n.to_string());
    params.push(Value::Integer(query.limit as i64));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(params.iter()), |row| {
            Ok(RawHit {
                id: row.get(0)?,
                type_: row.get(1)?,
                visibility: row.get(2)?,
                title: row.get(3)?,
                updated: row.get(4)?,
                snippet: row.get(5)?,
                score: row.get(6)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut hits = Vec::with_capacity(rows.len());
    for raw in rows {
        let tags = load_tags(conn, &raw.id)?;
        hits.push(SearchHit {
            id: raw.id,
            type_: raw.type_,
            title: raw.title,
            snippet: raw.snippet,
            score: raw.score,
            visibility: raw.visibility,
            tags,
            updated: parse_iso_date(raw.updated),
        });
    }
    Ok(hits)
}

fn run_list(conn: &Connection, filter: ListFilter) -> Result<Vec<ListItem>> {
    let mut sql = String::from(
        "SELECT d.id, d.type, d.visibility, d.title, d.updated \
         FROM documents d WHERE 1=1",
    );
    let mut params: Vec<Value> = Vec::new();

    if !filter.types.is_empty() {
        sql.push_str(" AND d.type IN (");
        push_placeholder_list(&mut sql, filter.types.len(), &mut params, &filter.types);
        sql.push(')');
    }
    if !filter.allowed_visibility.is_empty() {
        sql.push_str(" AND d.visibility IN (");
        push_placeholder_list(
            &mut sql,
            filter.allowed_visibility.len(),
            &mut params,
            &filter.allowed_visibility,
        );
        sql.push(')');
    }
    if !filter.tags.is_empty() {
        sql.push_str(" AND d.id IN (SELECT document_id FROM tags WHERE tag IN (");
        push_placeholder_list(&mut sql, filter.tags.len(), &mut params, &filter.tags);
        sql.push_str("))");
    }
    if let Some(date) = filter.updated_since {
        params.push(Value::Text(date_to_iso(date)));
        let n = params.len();
        sql.push_str(&format!(" AND d.updated >= ?{n}"));
    }

    sql.push_str(" ORDER BY COALESCE(d.updated, '') DESC, d.id ASC LIMIT ?");
    let n = params.len() + 1;
    sql.push_str(&n.to_string());
    params.push(Value::Integer(filter.limit as i64));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(params.iter()), |row| {
            Ok(RawListItem {
                id: row.get(0)?,
                type_: row.get(1)?,
                visibility: row.get(2)?,
                title: row.get(3)?,
                updated: row.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut items = Vec::with_capacity(rows.len());
    for raw in rows {
        let tags = load_tags(conn, &raw.id)?;
        items.push(ListItem {
            id: raw.id,
            type_: raw.type_,
            title: raw.title,
            visibility: raw.visibility,
            tags,
            updated: parse_iso_date(raw.updated),
        });
    }
    Ok(items)
}

fn load_tags(conn: &Connection, id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT tag FROM tags WHERE document_id = ?1 ORDER BY tag")?;
    let tags = stmt
        .query_map([id], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(tags)
}

fn push_placeholder_list(
    sql: &mut String,
    count: usize,
    params: &mut Vec<Value>,
    values: &[String],
) {
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            sql.push(',');
        }
        params.push(Value::Text(v.clone()));
        let n = params.len();
        sql.push('?');
        sql.push_str(&n.to_string());
    }
    let _ = count;
}

struct RawHit {
    id: String,
    type_: String,
    visibility: String,
    title: String,
    updated: Option<String>,
    snippet: String,
    score: f64,
}

struct RawListItem {
    id: String,
    type_: String,
    visibility: String,
    title: String,
    updated: Option<String>,
}
