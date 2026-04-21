//! Tenants + memberships.
//!
//! `GET /v1/tenants` is the first call a freshly-logged-in client makes —
//! it returns the set of workspaces the caller can attach to. The same
//! module owns the `TenantContext` extension that every `/v1/t/:tid/*`
//! route requires: it validates that the caller has a membership in the
//! URL-scoped tenant and pins the `role` on the request so downstream
//! handlers can do permission checks without another DB hop.
//!
//! Two layers to keep in mind:
//!   * session middleware (`auth::session_auth`) → puts `SessionContext`
//!     on the request.
//!   * tenant middleware (here, `tenant_auth`) → reads `SessionContext`,
//!     extracts `:tid`, joins `memberships`, puts `TenantContext` on
//!     the request.
//!
//! A request that reaches a tenant-scoped handler has both.

use crate::{error::ApiError, sessions::SessionContext, AppState};
use axum::{
    extract::{FromRequestParts, Path, Request, State},
    middleware::Next,
    response::Response,
    routing::get,
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use std::collections::HashMap;
use uuid::Uuid;

/// Attached to the request extensions by `tenant_auth` once membership
/// is verified. Handlers pull it with `Extension<TenantContext>`.
#[derive(Debug, Clone)]
pub struct TenantContext {
    pub tenant_id: Uuid,
    pub account_id: Uuid,
    pub role: String,
}

impl TenantContext {
    /// True if the caller can write org-level context or admin the
    /// tenant. Members can only read + propose (D11). Unused in 2b.2
    /// because `org/*` writes land in 2c, but pre-wired so we don't
    /// redo the role plumbing later.
    pub fn is_admin(&self) -> bool {
        matches!(self.role.as_str(), "owner" | "admin")
    }
}

#[derive(Debug, Serialize, FromRow)]
pub struct Membership {
    pub tenant_id: Uuid,
    pub name: String,
    pub kind: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct MembershipsResponse {
    memberships: Vec<Membership>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/tenants", get(list_memberships))
}

async fn list_memberships(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
) -> Result<Json<MembershipsResponse>, ApiError> {
    let rows: Vec<Membership> = sqlx::query_as(
        r#"
        SELECT t.id AS tenant_id, t.name, t.kind, m.role, m.created_at
        FROM memberships m
        JOIN tenants t ON t.id = m.tenant_id
        WHERE m.account_id = $1
        ORDER BY t.created_at ASC
        "#,
    )
    .bind(ctx.account_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(MembershipsResponse { memberships: rows }))
}

/// Middleware attached to every `/v1/t/:tid/*` route. Requires
/// `SessionContext` to already be on the request (session middleware
/// must run first). Loads the caller's role for the tenant; returns
/// `not_found` on a tenant the caller isn't a member of, which doubles
/// as enumeration resistance — a user cannot probe tenants they don't
/// belong to by URL walking.
pub async fn tenant_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let session = req
        .extensions()
        .get::<SessionContext>()
        .cloned()
        .ok_or(ApiError::Unauthorized)?;

    let (mut parts, body) = req.into_parts();
    // Extract *all* path params and pull `tid` out. Using a HashMap so
    // this middleware works regardless of how many sibling params the
    // downstream route defines (`:tid`, `:doc_id`, etc.).
    let Path(params): Path<HashMap<String, String>> =
        Path::from_request_parts(&mut parts, &state)
            .await
            .map_err(|_| ApiError::InvalidArgument("missing tenant id".into()))?;
    let tid_str = params
        .get("tid")
        .ok_or_else(|| ApiError::InvalidArgument("missing tenant id".into()))?;
    let tenant_id: Uuid = tid_str
        .parse()
        .map_err(|_| ApiError::InvalidArgument("invalid tenant id".into()))?;

    let row: Option<(String,)> = sqlx::query_as(
        "SELECT role FROM memberships WHERE tenant_id = $1 AND account_id = $2",
    )
    .bind(tenant_id)
    .bind(session.account_id)
    .fetch_optional(&state.db)
    .await?;

    let Some((role,)) = row else {
        return Err(ApiError::NotFound);
    };

    let mut req = Request::from_parts(parts, body);
    req.extensions_mut().insert(TenantContext {
        tenant_id,
        account_id: session.account_id,
        role,
    });
    Ok(next.run(req).await)
}
