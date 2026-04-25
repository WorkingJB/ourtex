//! Per-tenant audit chain stored in Postgres.
//!
//! Same shape as `ourtex-audit` (seq, ts, actor, action, document_id,
//! scope_used, outcome, prev_hash, hash) so a future "export audit" job
//! can emit JSONL that `ourtex-audit::verify` ingests unchanged. The only
//! expansion over the v1 wire format is that `actor` here additionally
//! accepts `account:<uuid>` for actions performed by a logged-in user
//! via the HTTP surface (v1 only modelled owner + MCP token actors).
//!
//! `append` is called from inside a DB transaction — it reads the last
//! (seq, hash) under `FOR UPDATE` then inserts the next entry. Chain
//! integrity follows from the transaction serializing the read and the
//! insert together; concurrent writes for the same tenant are rare
//! enough at v1 scale that the resulting contention is fine.

use crate::{error::ApiError, tenants::TenantContext, AppState};
use axum::{
    extract::{Query, State},
    routing::get,
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{postgres::PgRow, FromRow, Postgres, Row, Transaction};
use uuid::Uuid;

pub const ZERO_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

/// Unencoded actor. `as_encoded` gives the wire string stored in
/// `audit_entries.actor`; `parse` is the inverse. Mirrors
/// `ourtex_audit::Actor` with one additional variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    Owner,
    Token(String),
    Account(Uuid),
}

impl Actor {
    pub fn as_encoded(&self) -> String {
        match self {
            Self::Owner => "owner".to_string(),
            Self::Token(id) => format!("tok:{id}"),
            Self::Account(id) => format!("account:{id}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Denied,
    Error,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Denied => "denied",
            Self::Error => "error",
        }
    }
}

/// Caller-provided record — `seq`, `ts`, `prev_hash`, and `hash` are
/// computed inside `append`.
#[derive(Debug, Clone)]
pub struct AppendRecord {
    pub actor: Actor,
    pub action: String,
    pub document_id: Option<String>,
    pub scope_used: Vec<String>,
    pub outcome: Outcome,
}

#[derive(Debug, Serialize)]
pub struct AuditRow {
    pub seq: i64,
    pub ts: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub document_id: Option<String>,
    pub scope_used: Vec<String>,
    pub outcome: String,
    pub prev_hash: String,
    pub hash: String,
}

impl<'r> FromRow<'r, PgRow> for AuditRow {
    fn from_row(row: &'r PgRow) -> Result<Self, sqlx::Error> {
        Ok(AuditRow {
            seq: row.try_get("seq")?,
            ts: row.try_get("ts")?,
            actor: row.try_get("actor")?,
            action: row.try_get("action")?,
            document_id: row.try_get("document_id")?,
            scope_used: row.try_get("scope_used")?,
            outcome: row.try_get("outcome")?,
            prev_hash: row.try_get("prev_hash")?,
            hash: row.try_get("hash")?,
        })
    }
}

/// Append one entry to the tenant's audit chain inside an open
/// transaction. Must be called only after any business writes in the
/// same transaction, so a hash-recorded entry cannot outlive a
/// rolled-back mutation.
pub async fn append(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    record: AppendRecord,
) -> Result<AuditRow, ApiError> {
    // Lock the tenant's chain head: the `FOR UPDATE` keeps concurrent
    // appends serialized. `COALESCE` gives the genesis seq + zero hash
    // when the chain is empty.
    let row: Option<(i64, String)> = sqlx::query_as(
        r#"
        SELECT seq, hash FROM audit_entries
        WHERE tenant_id = $1
        ORDER BY seq DESC
        LIMIT 1
        FOR UPDATE
        "#,
    )
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await?;

    let (next_seq, prev_hash) = match row {
        Some((last_seq, last_hash)) => (last_seq + 1, last_hash),
        None => (0, ZERO_HASH.to_string()),
    };
    let ts = Utc::now();
    let actor_str = record.actor.as_encoded();
    let outcome_str = record.outcome.as_str();

    let hash = compute_hash(
        next_seq as u64,
        &ts,
        &actor_str,
        &record.action,
        record.document_id.as_deref(),
        &record.scope_used,
        outcome_str,
        &prev_hash,
    )?;

    sqlx::query(
        r#"
        INSERT INTO audit_entries
            (tenant_id, seq, ts, actor, action, document_id, scope_used, outcome, prev_hash, hash)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
    )
    .bind(tenant_id)
    .bind(next_seq)
    .bind(ts)
    .bind(&actor_str)
    .bind(&record.action)
    .bind(record.document_id.as_deref())
    .bind(&record.scope_used)
    .bind(outcome_str)
    .bind(&prev_hash)
    .bind(&hash)
    .execute(&mut **tx)
    .await?;

    Ok(AuditRow {
        seq: next_seq,
        ts,
        actor: actor_str,
        action: record.action,
        document_id: record.document_id,
        scope_used: record.scope_used,
        outcome: outcome_str.to_string(),
        prev_hash,
        hash,
    })
}

/// Canonical hash input. Field order is the struct order on the wire —
/// identical to `ourtex-audit`'s `HashInput` to keep both verifiers
/// compatible.
#[derive(Serialize)]
struct HashInput<'a> {
    seq: u64,
    ts: &'a DateTime<Utc>,
    actor: &'a str,
    action: &'a str,
    document_id: Option<&'a str>,
    scope_used: &'a [String],
    outcome: &'a str,
    prev_hash: &'a str,
}

fn compute_hash(
    seq: u64,
    ts: &DateTime<Utc>,
    actor: &str,
    action: &str,
    document_id: Option<&str>,
    scope_used: &[String],
    outcome: &str,
    prev_hash: &str,
) -> Result<String, ApiError> {
    let input = HashInput {
        seq,
        ts,
        actor,
        action,
        document_id,
        scope_used,
        outcome,
        prev_hash,
    };
    let bytes =
        serde_json::to_vec(&input).map_err(|e| ApiError::Internal(Box::new(e)))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

// ---------- HTTP surface ----------

pub fn router() -> Router<AppState> {
    Router::new().route("/audit", get(list_audit))
}

#[derive(Debug, Deserialize)]
struct AuditQuery {
    /// Return entries with `seq > after`. Clients paginate by passing
    /// the last `seq` they've seen. `None` starts from the genesis.
    after: Option<i64>,
    /// Hard cap on rows. Default 100; cap at 500.
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
struct AuditResponse {
    entries: Vec<AuditRow>,
    /// Convenience: the chain's current head hash. Clients can run a
    /// local rehash + check against this to prove the batch is intact.
    head_hash: Option<String>,
}

async fn list_audit(
    State(state): State<AppState>,
    Extension(tc): Extension<TenantContext>,
    Query(q): Query<AuditQuery>,
) -> Result<Json<AuditResponse>, ApiError> {
    let after = q.after.unwrap_or(-1);
    let limit = q.limit.unwrap_or(100).clamp(1, 500);

    let entries: Vec<AuditRow> = sqlx::query_as(
        r#"
        SELECT seq, ts, actor, action, document_id, scope_used,
               outcome, prev_hash, hash
        FROM audit_entries
        WHERE tenant_id = $1 AND seq > $2
        ORDER BY seq ASC
        LIMIT $3
        "#,
    )
    .bind(tc.tenant_id)
    .bind(after)
    .bind(limit)
    .fetch_all(&state.db)
    .await?;

    let head_hash: Option<(String,)> = sqlx::query_as(
        "SELECT hash FROM audit_entries WHERE tenant_id = $1 ORDER BY seq DESC LIMIT 1",
    )
    .bind(tc.tenant_id)
    .fetch_optional(&state.db)
    .await?;

    Ok(Json(AuditResponse {
        entries,
        head_hash: head_hash.map(|(h,)| h),
    }))
}
