//! Organizations: metadata layer above the storage tenant
//! (Phase 3 platform Slice 1, D10 revised).
//!
//! Holds two roles:
//!   1. **HTTP routes** under `/v1/orgs/*` for read/update/create.
//!   2. **Signup helpers** invoked from `accounts::signup` —
//!      `bootstrap_self_hosted` and `bootstrap_saas` — that decide
//!      whether a fresh signup becomes the first owner of a new org
//!      or lands in `pending_signups` for an existing one.
//!
//! v1 enforces a 1:1 mapping between `organizations` and `kind='org'`
//! tenants via the UNIQUE FK. The schema leaves room to decouple
//! later if a customer asks (D10 revised).

use crate::{
    accounts::Account, error::ApiError, sessions::SessionContext, AppState,
};
use axum::{
    extract::{Path, State},
    routing::get,
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

// ---------- types ----------

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Organization {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub logo_url: Option<String>,
    pub allowed_domains: serde_json::Value,
    pub settings: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct OrgMembership {
    pub org_id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub logo_url: Option<String>,
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct PendingSignup {
    pub id: Uuid,
    pub org_id: Uuid,
    pub org_name: String,
    pub requested_role: String,
    pub status: String,
    pub requested_at: DateTime<Utc>,
}

/// Outcome of a signup. Returned by `accounts::signup` so callers can
/// log + (eventually) shape responses based on whether the new account
/// has an immediate org membership or is awaiting approval.
#[derive(Debug, Clone)]
pub enum SignupOutcome {
    /// New org was created and the signup became its `owner`.
    BootstrappedOrg { org_id: Uuid, tenant_id: Uuid },
    /// Account exists but has no org membership yet — pending row
    /// landed for an admin to approve.
    AwaitingApproval { org_id: Uuid, pending_id: Uuid },
}

// ---------- signup helpers (called from accounts::signup) ----------

/// Self-hosted: first signup → owner of new singleton org.
/// Subsequent signups → pending for the existing singleton.
///
/// Race note: two concurrent first-signups can each see "no org" and
/// both create one. The result is a server with two orgs and two
/// owners — recoverable via admin cleanup. Acceptable for v1; tighten
/// with an advisory lock if it ever bites in practice.
pub async fn bootstrap_self_hosted(
    tx: &mut Transaction<'_, Postgres>,
    account: &Account,
) -> Result<SignupOutcome, ApiError> {
    let existing: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM organizations ORDER BY created_at ASC LIMIT 1",
    )
    .fetch_optional(&mut **tx)
    .await?;

    match existing {
        None => {
            let (org_id, tenant_id) =
                create_org_and_membership(tx, account, "Organization", &[]).await?;
            Ok(SignupOutcome::BootstrappedOrg { org_id, tenant_id })
        }
        Some((org_id,)) => {
            let pending_id = create_pending(tx, account.id, org_id).await?;
            Ok(SignupOutcome::AwaitingApproval { org_id, pending_id })
        }
    }
}

/// SaaS: signup with email domain matching some org's `allowed_domains`
/// → pending for that org. Otherwise → owner of a new org claiming
/// the email domain.
///
/// D17e (deferred): once email verification ships, the matching-domain
/// path will skip pending and create membership directly. Until then,
/// matching-domain still pends so a `mallory@acme.com` who never had
/// access to acme.com can't auto-land inside Acme's org.
pub async fn bootstrap_saas(
    tx: &mut Transaction<'_, Postgres>,
    account: &Account,
    email: &str,
) -> Result<SignupOutcome, ApiError> {
    let domain = email.split('@').nth(1).unwrap_or("").to_lowercase();
    if domain.is_empty() {
        return Err(ApiError::InvalidArgument("email must contain '@'".into()));
    }

    let matching: Option<(Uuid,)> = sqlx::query_as(
        r#"
        SELECT id FROM organizations
        WHERE allowed_domains @> to_jsonb($1::text)
        ORDER BY created_at ASC
        LIMIT 1
        "#,
    )
    .bind(&domain)
    .fetch_optional(&mut **tx)
    .await?;

    match matching {
        Some((org_id,)) => {
            let pending_id = create_pending(tx, account.id, org_id).await?;
            Ok(SignupOutcome::AwaitingApproval { org_id, pending_id })
        }
        None => {
            let (org_id, tenant_id) = create_org_and_membership(
                tx,
                account,
                &default_org_name(&domain),
                &[domain],
            )
            .await?;
            Ok(SignupOutcome::BootstrappedOrg { org_id, tenant_id })
        }
    }
}

async fn create_org_and_membership(
    tx: &mut Transaction<'_, Postgres>,
    owner: &Account,
    name: &str,
    allowed_domains: &[String],
) -> Result<(Uuid, Uuid), ApiError> {
    let tenant_row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO tenants (name, kind)
        VALUES ($1, 'org')
        RETURNING id
        "#,
    )
    .bind(name)
    .fetch_one(&mut **tx)
    .await?;

    let allowed_domains_json = serde_json::to_value(allowed_domains)
        .unwrap_or(serde_json::Value::Array(vec![]));

    let org_row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO organizations (tenant_id, name, allowed_domains)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(tenant_row.0)
    .bind(name)
    .bind(allowed_domains_json)
    .fetch_one(&mut **tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO memberships (tenant_id, account_id, role)
        VALUES ($1, $2, 'owner')
        "#,
    )
    .bind(tenant_row.0)
    .bind(owner.id)
    .execute(&mut **tx)
    .await?;

    Ok((org_row.0, tenant_row.0))
}

async fn create_pending(
    tx: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
    org_id: Uuid,
) -> Result<Uuid, ApiError> {
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO pending_signups (account_id, org_id, requested_role)
        VALUES ($1, $2, 'member')
        RETURNING id
        "#,
    )
    .bind(account_id)
    .bind(org_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.0)
}

fn default_org_name(domain: &str) -> String {
    // "acme.com" → "Acme". Strip TLD and title-case the head.
    let head = domain.split('.').next().unwrap_or(domain);
    if head.is_empty() {
        return "Organization".into();
    }
    let mut chars = head.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Organization".into(),
    }
}

// ---------- HTTP routes ----------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/orgs", get(list_orgs).post(create_org))
        .route("/orgs/:org_id", get(get_org).patch(update_org))
        .route("/orgs/:org_id/members", get(list_members))
        .route(
            "/orgs/:org_id/members/:account_id",
            axum::routing::patch(patch_member).delete(remove_member),
        )
        .route("/orgs/:org_id/pending", get(list_pending))
        .route(
            "/orgs/:org_id/pending/:account_id/approve",
            axum::routing::post(approve_pending),
        )
        .route(
            "/orgs/:org_id/pending/:account_id/reject",
            axum::routing::post(reject_pending),
        )
}

#[derive(Debug, Serialize)]
struct OrgsListResponse {
    memberships: Vec<OrgMembership>,
    pending: Vec<PendingSignup>,
}

async fn list_orgs(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
) -> Result<Json<OrgsListResponse>, ApiError> {
    let memberships: Vec<OrgMembership> = sqlx::query_as(
        r#"
        SELECT
            o.id          AS org_id,
            o.tenant_id   AS tenant_id,
            o.name        AS name,
            o.logo_url    AS logo_url,
            m.role        AS role,
            m.created_at  AS joined_at
        FROM memberships m
        JOIN tenants t      ON t.id = m.tenant_id AND t.kind = 'org'
        JOIN organizations o ON o.tenant_id = t.id
        WHERE m.account_id = $1
        ORDER BY m.created_at ASC
        "#,
    )
    .bind(ctx.account_id)
    .fetch_all(&state.db)
    .await?;

    let pending: Vec<PendingSignup> = sqlx::query_as(
        r#"
        SELECT
            p.id             AS id,
            p.org_id         AS org_id,
            o.name           AS org_name,
            p.requested_role AS requested_role,
            p.status         AS status,
            p.requested_at   AS requested_at
        FROM pending_signups p
        JOIN organizations o ON o.id = p.org_id
        WHERE p.account_id = $1 AND p.status = 'pending'
        ORDER BY p.requested_at ASC
        "#,
    )
    .bind(ctx.account_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(OrgsListResponse {
        memberships,
        pending,
    }))
}

async fn get_org(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
    Path(org_id): Path<Uuid>,
) -> Result<Json<Organization>, ApiError> {
    require_membership(&state.db, ctx.account_id, org_id).await?;
    let org = fetch_org(&state.db, org_id).await?;
    Ok(Json(org))
}

#[derive(Debug, Deserialize)]
struct UpdateOrgInput {
    name: Option<String>,
    logo_url: Option<String>,
    allowed_domains: Option<Vec<String>>,
    settings: Option<serde_json::Value>,
}

async fn update_org(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
    Path(org_id): Path<Uuid>,
    Json(input): Json<UpdateOrgInput>,
) -> Result<Json<Organization>, ApiError> {
    let role = require_membership(&state.db, ctx.account_id, org_id).await?;
    if !matches!(role.as_str(), "owner" | "admin") {
        return Err(ApiError::Forbidden);
    }

    let mut tx = state.db.begin().await?;
    if let Some(name) = input.name.as_deref() {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(ApiError::InvalidArgument("name must not be empty".into()));
        }
        sqlx::query("UPDATE organizations SET name = $1 WHERE id = $2")
            .bind(trimmed)
            .bind(org_id)
            .execute(&mut *tx)
            .await?;
        // Mirror the org name into the underlying tenant row so the
        // existing `/v1/tenants` listing stays human-readable.
        sqlx::query(
            r#"
            UPDATE tenants
            SET name = $1
            WHERE id = (SELECT tenant_id FROM organizations WHERE id = $2)
            "#,
        )
        .bind(trimmed)
        .bind(org_id)
        .execute(&mut *tx)
        .await?;
    }
    if let Some(logo_url) = input.logo_url.as_ref() {
        sqlx::query("UPDATE organizations SET logo_url = $1 WHERE id = $2")
            .bind(if logo_url.trim().is_empty() {
                None
            } else {
                Some(logo_url.as_str())
            })
            .bind(org_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(domains) = input.allowed_domains.as_ref() {
        let normalized: Vec<String> = domains
            .iter()
            .map(|d| d.trim().to_lowercase())
            .filter(|d| !d.is_empty())
            .collect();
        sqlx::query("UPDATE organizations SET allowed_domains = $1 WHERE id = $2")
            .bind(serde_json::to_value(&normalized).unwrap())
            .bind(org_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(settings) = input.settings.as_ref() {
        sqlx::query("UPDATE organizations SET settings = $1 WHERE id = $2")
            .bind(settings)
            .bind(org_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    Ok(Json(fetch_org(&state.db, org_id).await?))
}

#[derive(Debug, Deserialize)]
struct CreateOrgInput {
    name: String,
}

async fn create_org(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
    Json(input): Json<CreateOrgInput>,
) -> Result<Json<Organization>, ApiError> {
    let trimmed = input.name.trim();
    if trimmed.is_empty() {
        return Err(ApiError::InvalidArgument("name must not be empty".into()));
    }

    let mut tx = state.db.begin().await?;
    let account = crate::accounts::by_id_in(&mut tx, ctx.account_id).await?;
    let (org_id, _tenant_id) =
        create_org_and_membership(&mut tx, &account, trimmed, &[]).await?;
    tx.commit().await?;

    let org = fetch_org(&state.db, org_id).await?;
    Ok(Json(org))
}

// ---------- admin: members ----------

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct MemberDetail {
    pub account_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct MembersResponse {
    members: Vec<MemberDetail>,
}

async fn list_members(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
    Path(org_id): Path<Uuid>,
) -> Result<Json<MembersResponse>, ApiError> {
    require_admin(&state.db, ctx.account_id, org_id).await?;
    let members: Vec<MemberDetail> = sqlx::query_as(
        r#"
        SELECT
            a.id           AS account_id,
            a.email        AS email,
            a.display_name AS display_name,
            m.role         AS role,
            m.created_at   AS joined_at
        FROM memberships m
        JOIN accounts a       ON a.id = m.account_id
        JOIN organizations o  ON o.tenant_id = m.tenant_id
        WHERE o.id = $1
        ORDER BY m.created_at ASC
        "#,
    )
    .bind(org_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(MembersResponse { members }))
}

#[derive(Debug, Deserialize)]
struct PatchMemberInput {
    role: String,
}

async fn patch_member(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
    Path((org_id, target_account_id)): Path<(Uuid, Uuid)>,
    Json(input): Json<PatchMemberInput>,
) -> Result<Json<MemberDetail>, ApiError> {
    let caller_role = require_admin(&state.db, ctx.account_id, org_id).await?;
    let new_role = input.role.trim().to_lowercase();
    if !matches!(
        new_role.as_str(),
        "owner" | "admin" | "org_editor" | "member"
    ) {
        return Err(ApiError::InvalidArgument(
            "role must be one of owner, admin, org_editor, member".into(),
        ));
    }

    let mut tx = state.db.begin().await?;

    let target_role = current_role(&mut tx, org_id, target_account_id).await?;

    // Only an owner can promote to or demote from owner.
    if (target_role == "owner" || new_role == "owner") && caller_role != "owner" {
        return Err(ApiError::Forbidden);
    }

    // Last-owner guard: if demoting an owner, the org must retain at
    // least one owner.
    if target_role == "owner" && new_role != "owner" {
        ensure_other_owner_remains(&mut tx, org_id, target_account_id).await?;
    }

    sqlx::query(
        r#"
        UPDATE memberships
        SET role = $1
        WHERE account_id = $2
          AND tenant_id = (SELECT tenant_id FROM organizations WHERE id = $3)
        "#,
    )
    .bind(&new_role)
    .bind(target_account_id)
    .bind(org_id)
    .execute(&mut *tx)
    .await?;

    let updated = fetch_member(&mut tx, org_id, target_account_id).await?;
    tx.commit().await?;
    Ok(Json(updated))
}

async fn remove_member(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
    Path((org_id, target_account_id)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    let caller_role = require_admin(&state.db, ctx.account_id, org_id).await?;

    let mut tx = state.db.begin().await?;
    let target_role = current_role(&mut tx, org_id, target_account_id).await?;

    // Only an owner can remove an owner.
    if target_role == "owner" && caller_role != "owner" {
        return Err(ApiError::Forbidden);
    }
    if target_role == "owner" {
        ensure_other_owner_remains(&mut tx, org_id, target_account_id).await?;
    }

    sqlx::query(
        r#"
        DELETE FROM memberships
        WHERE account_id = $1
          AND tenant_id = (SELECT tenant_id FROM organizations WHERE id = $2)
        "#,
    )
    .bind(target_account_id)
    .bind(org_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ---------- admin: pending signups ----------

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct PendingDetail {
    pub id: Uuid,
    pub account_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub requested_role: String,
    pub status: String,
    pub note: Option<String>,
    pub requested_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct PendingResponse {
    pending: Vec<PendingDetail>,
}

#[derive(Debug, Deserialize)]
struct PendingQuery {
    /// Filter by status. Defaults to `pending` so the queue view stays
    /// quiet by default.
    #[serde(default)]
    status: Option<String>,
}

async fn list_pending(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
    Path(org_id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<PendingQuery>,
) -> Result<Json<PendingResponse>, ApiError> {
    require_admin(&state.db, ctx.account_id, org_id).await?;
    let status = q.status.as_deref().unwrap_or("pending");
    if !matches!(status, "pending" | "approved" | "rejected" | "all") {
        return Err(ApiError::InvalidArgument(
            "status must be pending|approved|rejected|all".into(),
        ));
    }
    let pending: Vec<PendingDetail> = if status == "all" {
        sqlx::query_as(
            r#"
            SELECT
                p.id             AS id,
                p.account_id     AS account_id,
                a.email          AS email,
                a.display_name   AS display_name,
                p.requested_role AS requested_role,
                p.status         AS status,
                p.note           AS note,
                p.requested_at   AS requested_at
            FROM pending_signups p
            JOIN accounts a ON a.id = p.account_id
            WHERE p.org_id = $1
            ORDER BY p.requested_at ASC
            "#,
        )
        .bind(org_id)
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query_as(
            r#"
            SELECT
                p.id             AS id,
                p.account_id     AS account_id,
                a.email          AS email,
                a.display_name   AS display_name,
                p.requested_role AS requested_role,
                p.status         AS status,
                p.note           AS note,
                p.requested_at   AS requested_at
            FROM pending_signups p
            JOIN accounts a ON a.id = p.account_id
            WHERE p.org_id = $1 AND p.status = $2
            ORDER BY p.requested_at ASC
            "#,
        )
        .bind(org_id)
        .bind(status)
        .fetch_all(&state.db)
        .await?
    };
    Ok(Json(PendingResponse { pending }))
}

#[derive(Debug, Default, Deserialize)]
struct ApproveInput {
    /// Optional override for the role to grant. Defaults to the
    /// `requested_role` recorded on the pending row.
    #[serde(default)]
    role: Option<String>,
}

async fn approve_pending(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
    Path((org_id, target_account_id)): Path<(Uuid, Uuid)>,
    Json(input): Json<ApproveInput>,
) -> Result<Json<MemberDetail>, ApiError> {
    let caller_role = require_admin(&state.db, ctx.account_id, org_id).await?;

    let mut tx = state.db.begin().await?;

    // Pending row + ownership of granted role.
    let pending: Option<(Uuid, String, String)> = sqlx::query_as(
        r#"
        SELECT id, status, requested_role
        FROM pending_signups
        WHERE account_id = $1 AND org_id = $2
        FOR UPDATE
        "#,
    )
    .bind(target_account_id)
    .bind(org_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((pending_id, status, requested_role)) = pending else {
        return Err(ApiError::NotFound);
    };
    if status != "pending" {
        return Err(ApiError::Conflict("pending row already decided"));
    }

    let role = input.role.unwrap_or(requested_role).to_lowercase();
    if !matches!(role.as_str(), "owner" | "admin" | "org_editor" | "member") {
        return Err(ApiError::InvalidArgument(
            "role must be one of owner, admin, org_editor, member".into(),
        ));
    }
    if role == "owner" && caller_role != "owner" {
        return Err(ApiError::Forbidden);
    }

    let tenant_id: (Uuid,) =
        sqlx::query_as("SELECT tenant_id FROM organizations WHERE id = $1")
            .bind(org_id)
            .fetch_one(&mut *tx)
            .await?;

    sqlx::query(
        r#"
        INSERT INTO memberships (tenant_id, account_id, role)
        VALUES ($1, $2, $3)
        ON CONFLICT (tenant_id, account_id) DO UPDATE SET role = EXCLUDED.role
        "#,
    )
    .bind(tenant_id.0)
    .bind(target_account_id)
    .bind(&role)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        UPDATE pending_signups
        SET status = 'approved', decided_by = $1, decided_at = now()
        WHERE id = $2
        "#,
    )
    .bind(ctx.account_id)
    .bind(pending_id)
    .execute(&mut *tx)
    .await?;

    let member = fetch_member(&mut tx, org_id, target_account_id).await?;
    tx.commit().await?;
    Ok(Json(member))
}

async fn reject_pending(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
    Path((org_id, target_account_id)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    require_admin(&state.db, ctx.account_id, org_id).await?;

    let mut tx = state.db.begin().await?;
    let pending: Option<(Uuid, String)> = sqlx::query_as(
        r#"
        SELECT id, status
        FROM pending_signups
        WHERE account_id = $1 AND org_id = $2
        FOR UPDATE
        "#,
    )
    .bind(target_account_id)
    .bind(org_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((pending_id, status)) = pending else {
        return Err(ApiError::NotFound);
    };
    if status != "pending" {
        return Err(ApiError::Conflict("pending row already decided"));
    }

    sqlx::query(
        r#"
        UPDATE pending_signups
        SET status = 'rejected', decided_by = $1, decided_at = now()
        WHERE id = $2
        "#,
    )
    .bind(ctx.account_id)
    .bind(pending_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ---------- helpers ----------

async fn require_membership(
    db: &sqlx::PgPool,
    account_id: Uuid,
    org_id: Uuid,
) -> Result<String, ApiError> {
    let row: Option<(String,)> = sqlx::query_as(
        r#"
        SELECT m.role
        FROM memberships m
        JOIN organizations o ON o.tenant_id = m.tenant_id
        WHERE m.account_id = $1 AND o.id = $2
        "#,
    )
    .bind(account_id)
    .bind(org_id)
    .fetch_optional(db)
    .await?;
    row.map(|(role,)| role).ok_or(ApiError::NotFound)
}

/// Combines the membership check and the admin-or-owner gate so admin
/// endpoints don't have to duplicate the role match. Returns the
/// caller's role so handlers can branch (e.g. owner-only operations).
async fn require_admin(
    db: &sqlx::PgPool,
    account_id: Uuid,
    org_id: Uuid,
) -> Result<String, ApiError> {
    let role = require_membership(db, account_id, org_id).await?;
    if !matches!(role.as_str(), "owner" | "admin") {
        return Err(ApiError::Forbidden);
    }
    Ok(role)
}

async fn current_role(
    tx: &mut Transaction<'_, Postgres>,
    org_id: Uuid,
    account_id: Uuid,
) -> Result<String, ApiError> {
    let row: Option<(String,)> = sqlx::query_as(
        r#"
        SELECT m.role
        FROM memberships m
        JOIN organizations o ON o.tenant_id = m.tenant_id
        WHERE m.account_id = $1 AND o.id = $2
        FOR UPDATE OF m
        "#,
    )
    .bind(account_id)
    .bind(org_id)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|(role,)| role).ok_or(ApiError::NotFound)
}

/// Refuses if `target_account_id` is the only `owner` of `org_id`.
/// Use before demoting / removing an owner.
async fn ensure_other_owner_remains(
    tx: &mut Transaction<'_, Postgres>,
    org_id: Uuid,
    target_account_id: Uuid,
) -> Result<(), ApiError> {
    let other_owners: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM memberships m
        JOIN organizations o ON o.tenant_id = m.tenant_id
        WHERE o.id = $1 AND m.role = 'owner' AND m.account_id <> $2
        "#,
    )
    .bind(org_id)
    .bind(target_account_id)
    .fetch_one(&mut **tx)
    .await?;
    if other_owners.0 == 0 {
        return Err(ApiError::Conflict("org must retain at least one owner"));
    }
    Ok(())
}

async fn fetch_member(
    tx: &mut Transaction<'_, Postgres>,
    org_id: Uuid,
    account_id: Uuid,
) -> Result<MemberDetail, ApiError> {
    let row: Option<MemberDetail> = sqlx::query_as(
        r#"
        SELECT
            a.id           AS account_id,
            a.email        AS email,
            a.display_name AS display_name,
            m.role         AS role,
            m.created_at   AS joined_at
        FROM memberships m
        JOIN accounts a       ON a.id = m.account_id
        JOIN organizations o  ON o.tenant_id = m.tenant_id
        WHERE m.account_id = $1 AND o.id = $2
        "#,
    )
    .bind(account_id)
    .bind(org_id)
    .fetch_optional(&mut **tx)
    .await?;
    row.ok_or(ApiError::NotFound)
}

async fn fetch_org(
    db: &sqlx::PgPool,
    org_id: Uuid,
) -> Result<Organization, ApiError> {
    let org: Option<Organization> = sqlx::query_as(
        r#"
        SELECT id, tenant_id, name, logo_url, allowed_domains, settings, created_at
        FROM organizations
        WHERE id = $1
        "#,
    )
    .bind(org_id)
    .fetch_optional(db)
    .await?;
    org.ok_or(ApiError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_org_name_strips_tld_and_titlecases() {
        assert_eq!(default_org_name("acme.com"), "Acme");
        assert_eq!(default_org_name("foo-bar.io"), "Foo-bar");
        assert_eq!(default_org_name("a.b.c"), "A");
        assert_eq!(default_org_name(""), "Organization");
    }
}
