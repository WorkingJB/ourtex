//! Ourtex server — HTTP API.
//!
//! Phase 2b.1 shipped user auth (`/v1/auth/*`). Phase 2b.2 adds the
//! tenant-scoped vault + index + tokens + audit surface under
//! `/v1/t/:tid/*`. Encryption at rest lands in 2b.3; MCP HTTP/SSE +
//! `context.propose` in 2b.4. See `docs/implementation-status.md` §Phase 2b.

#![forbid(unsafe_code)]

pub mod accounts;
pub mod audit;
pub mod auth;
pub mod config;
pub mod cookies;
pub mod crypto_api;
pub mod documents;
pub mod error;
pub mod idx;
pub mod password;
pub mod session_keys;
pub mod sessions;
pub mod tenants;
pub mod tokens;

use axum::{middleware, routing::get, Router};
use sqlx::PgPool;
use std::sync::Arc;

/// Shared handle passed to every request handler.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub sessions: Arc<sessions::SessionService>,
    pub session_keys: Arc<session_keys::SessionKeyStore>,
    /// Whether `Set-Cookie` headers carry the `Secure` flag. `false`
    /// is only valid for local-HTTP development; production deployments
    /// must keep this `true` so the browser refuses to send cookies
    /// over a downgrade.
    pub secure_cookies: bool,
}

impl AppState {
    /// Production-style constructor with `Secure` cookies enabled.
    /// Tests and `main` call this; integration tests can override the
    /// flag with `with_secure_cookies(false)` before mounting the
    /// router if they need to exercise the cookie path over plain HTTP.
    pub fn new(db: PgPool) -> Self {
        let sessions = Arc::new(sessions::SessionService::new(db.clone()));
        let session_keys = Arc::new(session_keys::SessionKeyStore::new());
        AppState {
            db,
            sessions,
            session_keys,
            secure_cookies: true,
        }
    }

    pub fn with_secure_cookies(mut self, secure: bool) -> Self {
        self.secure_cookies = secure;
        self
    }
}

/// Build the full `axum::Router` with every route mounted. Callers are
/// responsible for binding to an address and running the server — this
/// lets integration tests stand it up with `tower::ServiceExt` without
/// a real network listener.
pub fn router(state: AppState) -> Router {
    // Tenant-scoped routes sit under `/v1/t/:tid`. axum's `route_layer`
    // calls run outermost-last, so the layers below execute, top-to-bottom
    // on the request:
    //   `session_auth` → attaches `SessionContext` (or 401)
    //   `csrf_guard`   → enforces double-submit on cookie-authed mutating requests
    //   `tenant_auth`  → checks membership + attaches `TenantContext`
    let tenant_routes: Router<AppState> = Router::new()
        .merge(documents::router())
        .merge(idx::router())
        .merge(tokens::router())
        .merge(audit::router())
        .merge(crypto_api::router())
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            tenants::tenant_auth,
        ))
        .route_layer(middleware::from_fn(auth::csrf_guard))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::session_auth,
        ));

    // Session-authed, non-tenant-scoped (membership listing).
    let tenants_route: Router<AppState> = tenants::router()
        .route_layer(middleware::from_fn(auth::csrf_guard))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::session_auth,
        ));

    Router::new()
        .route("/healthz", get(healthz))
        .nest("/v1/auth", auth::router(state.clone()))
        .nest("/v1", tenants_route)
        .nest("/v1/t/:tid", tenant_routes)
        .with_state(state)
}

async fn healthz() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({ "ok": true }))
}

/// Run embedded migrations against the provided pool. Called from
/// `main` on startup so the server is usable out of the box; tests
/// call it explicitly.
pub async fn migrate(db: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(db).await
}
