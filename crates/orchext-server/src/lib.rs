//! Orchext server — HTTP API.
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
pub mod mcp;
pub mod oauth;
pub mod password;
pub mod proposals;
pub mod session_keys;
pub mod sessions;
pub mod tenants;
pub mod tokens;

use axum::{
    http::{header, HeaderValue, Method},
    middleware,
    routing::get,
    Router,
};
use sqlx::PgPool;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

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
    /// Per-IP throttle on `/v1/auth/{signup,login}` (and the `/native/*`
    /// twins). Defaults to true. Integration tests turn it off because
    /// `tower::ServiceExt::oneshot` doesn't attach a peer `SocketAddr`,
    /// which the IP key extractor needs.
    pub rate_limit_auth: bool,
}

impl AppState {
    /// Production-style constructor: secure cookies on, rate limiting
    /// on. Tests use `with_*` builders to opt out where needed.
    pub fn new(db: PgPool) -> Self {
        let sessions = Arc::new(sessions::SessionService::new(db.clone()));
        let session_keys = Arc::new(session_keys::SessionKeyStore::new());
        AppState {
            db,
            sessions,
            session_keys,
            secure_cookies: true,
            rate_limit_auth: true,
        }
    }

    pub fn with_secure_cookies(mut self, secure: bool) -> Self {
        self.secure_cookies = secure;
        self
    }

    pub fn with_rate_limit_auth(mut self, on: bool) -> Self {
        self.rate_limit_auth = on;
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
        .merge(proposals::router())
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

    // OAuth: `/authorize` requires a logged-in user (session-authed).
    // `/token` does not — the agent client only holds the auth code,
    // not a session, and the code itself is the credential. Mounted
    // as siblings under `/v1/oauth`.
    let oauth_authorize: Router<AppState> = oauth::authorize_router()
        .route_layer(middleware::from_fn(auth::csrf_guard))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::session_auth,
        ));
    let oauth_routes: Router<AppState> = oauth::router().merge(oauth_authorize);

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .nest("/v1/auth", auth::router(state.clone()))
        .nest("/v1/oauth", oauth_routes)
        .nest("/v1/mcp", mcp::router())
        .nest("/v1", tenants_route)
        .nest("/v1/t/:tid", tenant_routes)
        .with_state(state)
}

async fn healthz() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({ "ok": true }))
}

// `/readyz` is a deeper check than `/healthz`: it confirms the server can
// actually talk to Postgres. Orchestrators (Fly's `[[http_service.checks]]`,
// Kubernetes readiness probes, docker-compose's `healthcheck`) should hit
// this one — a process that's listening but can't reach its DB shouldn't
// receive traffic. `/healthz` stays cheap and DB-free for liveness.
async fn readyz(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<axum::Json<serde_json::Value>, axum::http::StatusCode> {
    sqlx::query("SELECT 1")
        .execute(&state.db)
        .await
        .map_err(|err| {
            tracing::warn!(error = %err, "readyz: db check failed");
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        })?;
    Ok(axum::Json(serde_json::json!({ "ok": true })))
}

/// Run embedded migrations against the provided pool. Called from
/// `main` on startup so the server is usable out of the box; tests
/// call it explicitly.
pub async fn migrate(db: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(db).await
}

/// Build a CORS layer from a list of allowed origins. Returns `None`
/// when the list is empty — the caller should then mount no CORS
/// middleware at all, which is the secure default. The hosted SaaS
/// deployment runs same-origin via Vercel rewrites so production
/// returns `None`; self-hosters who serve the web app from a separate
/// origin opt in via `ORCHEXT_CORS_ALLOW_ORIGINS`.
///
/// `allow_credentials(true)` is required because the SPA relies on
/// cookies (`credentials: 'include'`); the trade-off is that origins
/// must be enumerated explicitly (the spec forbids `*` with
/// credentials).
pub fn cors_layer(origins: &[String]) -> Option<CorsLayer> {
    if origins.is_empty() {
        return None;
    }
    let parsed: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    if parsed.is_empty() {
        // Every origin failed to parse; treat the same as empty.
        return None;
    }
    Some(
        CorsLayer::new()
            .allow_origin(parsed)
            .allow_credentials(true)
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers([
                header::CONTENT_TYPE,
                header::AUTHORIZATION,
                axum::http::HeaderName::from_static("x-orchext-csrf"),
            ])
            .max_age(std::time::Duration::from_secs(3600)),
    )
}
