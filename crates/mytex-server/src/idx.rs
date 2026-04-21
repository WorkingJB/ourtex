//! Index endpoints — search, list (filtered), graph, backlinks.
//!
//! Wire parity with the `mytex-index` local surface: request shapes and
//! response fields are drop-in compatible with `SearchQuery`, `ListFilter`,
//! and the graph/backlinks helpers, so the sync client can fan them out
//! onto HTTP without a translation layer.
//!
//! Backing store is the Postgres `documents` + `doc_tags` + `doc_links`
//! tables. Full-text search uses a stored `tsvector` column matched by
//! `websearch_to_tsquery` (supports `"quoted phrases"`, `OR`, `-negated`).

use crate::{error::ApiError, tenants::TenantContext, AppState};
use axum::{
    extract::{Path, Query, State},
    routing::get,
    Extension, Json, Router,
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/index/search", get(search))
        .route("/index/list", get(list))
        .route("/index/graph", get(graph))
        .route("/index/backlinks/:doc_id", get(backlinks))
        .route("/index/outbound/:doc_id", get(outbound_links))
}

// ---------- search ----------

#[derive(Debug, Deserialize)]
struct SearchParams {
    /// Free-text query. Passed to `websearch_to_tsquery`.
    q: String,
    /// Optional comma-separated type filters.
    #[serde(default)]
    types: Option<String>,
    /// Optional comma-separated tag filters (any-match).
    #[serde(default)]
    tags: Option<String>,
    /// Comma-separated visibility labels the caller is allowed to see.
    /// The sync client passes this so the `private` hard floor is
    /// enforced at the DB layer — omitting "private" makes it
    /// impossible for a match on a private doc to surface.
    #[serde(default)]
    visibility: Option<String>,
    /// ISO-8601 date; only docs with frontmatter.updated >= this date
    /// are returned.
    #[serde(default)]
    updated_since: Option<NaiveDate>,
    /// Result cap. Clamped to [1, 500]; default 20.
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
struct SearchHit {
    doc_id: String,
    type_: String,
    title: String,
    snippet: String,
    score: f64,
    visibility: String,
    tags: Vec<String>,
    updated: Option<NaiveDate>,
}

#[derive(Debug, Serialize)]
struct SearchResponse {
    hits: Vec<SearchHit>,
}

async fn search(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
    Query(p): Query<SearchParams>,
) -> Result<Json<SearchResponse>, ApiError> {
    let types = csv_list(&p.types);
    let tags = csv_list(&p.tags);
    let visibility = csv_list(&p.visibility);
    let limit = p.limit.unwrap_or(20).clamp(1, 500);

    // Build the WHERE clause. Using `array_agg` for tags keeps this
    // to one round trip. `ts_headline` produces the snippet; stripping
    // the default `<b>/</b>` markers since clients display text only.
    #[derive(FromRow)]
    struct Row {
        doc_id: String,
        type_: String,
        title: String,
        snippet: String,
        score: f64,
        visibility: String,
        tags: Option<Vec<String>>,
        updated: Option<NaiveDate>,
    }

    let rows: Vec<Row> = sqlx::query_as(
        r#"
        SELECT
            d.doc_id,
            d.type_,
            d.title,
            ts_headline(
                'english', d.body, websearch_to_tsquery('english', $2),
                'StartSel=, StopSel=, MaxFragments=1, MaxWords=18, MinWords=5'
            ) AS snippet,
            ts_rank_cd(d.tsv, websearch_to_tsquery('english', $2))::float8 AS score,
            d.visibility,
            (SELECT array_agg(t.tag ORDER BY t.tag)
               FROM doc_tags t
              WHERE t.tenant_id = d.tenant_id AND t.doc_id = d.doc_id) AS tags,
            (d.frontmatter->>'updated')::date AS updated
        FROM documents d
        WHERE d.tenant_id = $1
          AND d.tsv @@ websearch_to_tsquery('english', $2)
          AND ($3::text[] IS NULL OR d.type_ = ANY($3))
          AND ($4::text[] IS NULL OR d.visibility = ANY($4))
          AND ($6::date IS NULL OR (d.frontmatter->>'updated')::date >= $6)
          AND ($5::text[] IS NULL OR EXISTS (
                SELECT 1 FROM doc_tags t
                 WHERE t.tenant_id = d.tenant_id
                   AND t.doc_id = d.doc_id
                   AND t.tag = ANY($5)))
        ORDER BY score DESC
        LIMIT $7
        "#,
    )
    .bind(tc.tenant_id)
    .bind(&p.q)
    .bind(nullable_array(&types))
    .bind(nullable_array(&visibility))
    .bind(nullable_array(&tags))
    .bind(p.updated_since)
    .bind(limit)
    .fetch_all(&state.db)
    .await?;

    let hits = rows
        .into_iter()
        .map(|r| SearchHit {
            doc_id: r.doc_id,
            type_: r.type_,
            title: r.title,
            snippet: r.snippet,
            score: r.score,
            visibility: r.visibility,
            tags: r.tags.unwrap_or_default(),
            updated: r.updated,
        })
        .collect();
    Ok(Json(SearchResponse { hits }))
}

// ---------- list ----------

#[derive(Debug, Deserialize)]
struct ListParams {
    #[serde(default)]
    types: Option<String>,
    #[serde(default)]
    tags: Option<String>,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    updated_since: Option<NaiveDate>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
struct ListItem {
    doc_id: String,
    type_: String,
    title: String,
    visibility: String,
    tags: Vec<String>,
    updated: Option<NaiveDate>,
}

#[derive(Debug, Serialize)]
struct ListResponse {
    items: Vec<ListItem>,
}

async fn list(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
    Query(p): Query<ListParams>,
) -> Result<Json<ListResponse>, ApiError> {
    let types = csv_list(&p.types);
    let tags = csv_list(&p.tags);
    let visibility = csv_list(&p.visibility);
    let limit = p.limit.unwrap_or(100).clamp(1, 500);

    #[derive(FromRow)]
    struct Row {
        doc_id: String,
        type_: String,
        title: String,
        visibility: String,
        tags: Option<Vec<String>>,
        updated: Option<NaiveDate>,
    }

    let rows: Vec<Row> = sqlx::query_as(
        r#"
        SELECT
            d.doc_id,
            d.type_,
            d.title,
            d.visibility,
            (SELECT array_agg(t.tag ORDER BY t.tag)
               FROM doc_tags t
              WHERE t.tenant_id = d.tenant_id AND t.doc_id = d.doc_id) AS tags,
            (d.frontmatter->>'updated')::date AS updated
        FROM documents d
        WHERE d.tenant_id = $1
          AND ($2::text[] IS NULL OR d.type_ = ANY($2))
          AND ($3::text[] IS NULL OR d.visibility = ANY($3))
          AND ($5::date IS NULL OR (d.frontmatter->>'updated')::date >= $5)
          AND ($4::text[] IS NULL OR EXISTS (
                SELECT 1 FROM doc_tags t
                 WHERE t.tenant_id = d.tenant_id
                   AND t.doc_id = d.doc_id
                   AND t.tag = ANY($4)))
        ORDER BY COALESCE((d.frontmatter->>'updated')::date, '0001-01-01'::date) DESC,
                 d.doc_id ASC
        LIMIT $6
        "#,
    )
    .bind(tc.tenant_id)
    .bind(nullable_array(&types))
    .bind(nullable_array(&visibility))
    .bind(nullable_array(&tags))
    .bind(p.updated_since)
    .bind(limit)
    .fetch_all(&state.db)
    .await?;

    let items = rows
        .into_iter()
        .map(|r| ListItem {
            doc_id: r.doc_id,
            type_: r.type_,
            title: r.title,
            visibility: r.visibility,
            tags: r.tags.unwrap_or_default(),
            updated: r.updated,
        })
        .collect();
    Ok(Json(ListResponse { items }))
}

// ---------- graph ----------

#[derive(Debug, Serialize)]
struct GraphEdge {
    source: String,
    target: String,
}

#[derive(Debug, Serialize)]
struct GraphResponse {
    /// Node ids that actually exist in the vault. Matches the desktop
    /// graph view's "filter orphan edges" rule — we only emit edges
    /// whose endpoints are present.
    nodes: Vec<String>,
    edges: Vec<GraphEdge>,
}

async fn graph(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
) -> Result<Json<GraphResponse>, ApiError> {
    let nodes: Vec<(String,)> = sqlx::query_as(
        "SELECT doc_id FROM documents WHERE tenant_id = $1 ORDER BY doc_id",
    )
    .bind(tc.tenant_id)
    .fetch_all(&state.db)
    .await?;

    // Only edges whose target is also a document in the same tenant.
    // Same "orphan edge filter" the desktop graph view applies.
    let edges: Vec<(String, String)> = sqlx::query_as(
        r#"
        SELECT l.source, l.target
        FROM doc_links l
        WHERE l.tenant_id = $1
          AND EXISTS (
                SELECT 1 FROM documents d
                 WHERE d.tenant_id = l.tenant_id AND d.doc_id = l.target
              )
        ORDER BY l.source, l.target
        "#,
    )
    .bind(tc.tenant_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(GraphResponse {
        nodes: nodes.into_iter().map(|(s,)| s).collect(),
        edges: edges
            .into_iter()
            .map(|(source, target)| GraphEdge { source, target })
            .collect(),
    }))
}

// ---------- backlinks / outbound ----------

#[derive(Debug, Serialize)]
struct LinkList {
    links: Vec<String>,
}

async fn backlinks(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
    Path((_tid, doc_id)): Path<(Uuid, String)>,
) -> Result<Json<LinkList>, ApiError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT source FROM doc_links WHERE tenant_id = $1 AND target = $2 ORDER BY source",
    )
    .bind(tc.tenant_id)
    .bind(&doc_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(LinkList {
        links: rows.into_iter().map(|(s,)| s).collect(),
    }))
}

async fn outbound_links(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
    Path((_tid, doc_id)): Path<(Uuid, String)>,
) -> Result<Json<LinkList>, ApiError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT target FROM doc_links WHERE tenant_id = $1 AND source = $2 ORDER BY target",
    )
    .bind(tc.tenant_id)
    .bind(&doc_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(LinkList {
        links: rows.into_iter().map(|(s,)| s).collect(),
    }))
}

// ---------- helpers ----------

fn csv_list(s: &Option<String>) -> Vec<String> {
    match s {
        None => Vec::new(),
        Some(s) => s
            .split(',')
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .map(|p| p.to_string())
            .collect(),
    }
}

/// `Vec::is_empty() → None` so sqlx binds a SQL NULL and the
/// `$n::text[] IS NULL OR ...` short-circuit skips the filter.
fn nullable_array(v: &[String]) -> Option<Vec<String>> {
    if v.is_empty() {
        None
    } else {
        Some(v.to_vec())
    }
}
