//! Authentication HTTP surface: `/v1/auth/*`.
//!
//! Two parallel surfaces, with distinct response shapes so a browser
//! never accidentally receives the bearer secret a native client
//! needs:
//!
//! Browser flow (cookie-based, default):
//! - `POST   /v1/auth/signup`        — sets HttpOnly session cookie
//! - `POST   /v1/auth/login`         — sets HttpOnly session cookie
//!
//! Native flow (bearer-based, opt-in via path):
//! - `POST   /v1/auth/native/signup` — returns secret in JSON body
//! - `POST   /v1/auth/native/login`  — returns secret in JSON body
//!
//! Shared protected routes:
//! - `GET    /v1/auth/me`       — current account (authenticated)
//! - `GET    /v1/auth/sessions` — list active sessions (authenticated)
//! - `PATCH  /v1/auth/account`  — update display name
//! - `POST   /v1/auth/password` — change password (current + new)
//! - `DELETE /v1/auth/logout`   — revoke the current session
//!
//! Session middleware on the authenticated routes extracts the bearer
//! token (or session cookie), validates it against the sessions
//! table, and attaches a `SessionContext` to the request extensions.

use crate::{
    accounts::{self, Account, SignupInput},
    cookies,
    error::ApiError,
    sessions::{AuthSource, SessionContext, SessionSummary},
    AppState,
};
use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post},
    Extension, Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor, GovernorLayer,
};
use uuid::Uuid;

/// Build the `/v1/auth/*` router. Takes an `AppState` by value so the
/// session-auth middleware on protected routes can be constructed with
/// `from_fn_with_state` (which binds state at layer-construction time,
/// not at request time).
pub fn router(state: AppState) -> Router<AppState> {
    let mut browser_public = Router::new()
        .route("/signup", post(signup_browser_handler))
        .route("/login", post(login_browser_handler));

    let mut native_public = Router::new()
        .route("/signup", post(signup_native_handler))
        .route("/login", post(login_native_handler));

    if state.rate_limit_auth {
        // Per-IP throttle on the unauthenticated signup/login surface.
        // ~10 attempts/minute sustained with a small burst — enough
        // for a human fat-fingering a password but slow enough that
        // credential stuffing or signup-flood campaigns become
        // unattractive. This is in-process; multi-instance deployments
        // need a Redis-backed limiter or an upstream proxy rule.
        //
        // Key extractor: `SmartIpKeyExtractor` reads `X-Forwarded-For`
        // / `X-Real-IP` / `Forwarded` first, then falls back to the
        // axum `ConnectInfo<SocketAddr>` extension. Behind Fly the XFF
        // header is always set; without this the default
        // `PeerIpKeyExtractor` 500s with "Unable To Extract Key" since
        // every request would otherwise share the Fly proxy's peer IP
        // (or have no peer at all). `main.rs` wires `ConnectInfo` so
        // the fallback works for direct/self-host deploys too.
        let mut builder = GovernorConfigBuilder::default();
        builder.per_second(6).burst_size(5);
        let governor_conf = Arc::new(
            builder
                .key_extractor(SmartIpKeyExtractor)
                .finish()
                .expect("governor config should be valid"),
        );
        let governor_layer = GovernorLayer {
            config: governor_conf,
        };
        browser_public = browser_public.layer(governor_layer.clone());
        native_public = native_public.layer(governor_layer);
    }

    let protected = Router::new()
        .route("/me", get(me_handler))
        .route("/sessions", get(sessions_handler))
        .route("/account", patch(update_account_handler))
        .route("/password", post(change_password_handler))
        .route("/logout", delete(logout_handler))
        .route_layer(middleware::from_fn(csrf_guard))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            session_auth,
        ));

    browser_public
        .nest("/native", native_public)
        .merge(protected)
}

// ---------- handlers ----------

#[derive(Debug, Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
    label: Option<String>,
}

#[derive(Debug, Serialize)]
struct AccountDto {
    id: Uuid,
    email: String,
    display_name: String,
    created_at: DateTime<Utc>,
}

impl From<Account> for AccountDto {
    fn from(a: Account) -> Self {
        AccountDto {
            id: a.id,
            email: a.email,
            display_name: a.display_name,
            created_at: a.created_at,
        }
    }
}

/// Browser response shape — no `secret` field. The session secret
/// reaches the client only through the `HttpOnly` cookie set in the
/// same response, so JS / XSS can't read it.
#[derive(Debug, Serialize)]
struct BrowserSession {
    id: Uuid,
    expires_at: DateTime<Utc>,
}

/// Native response shape — returns the bearer secret, no cookies.
/// Used by desktop, agents, anything that holds tokens out-of-browser.
#[derive(Debug, Serialize)]
struct NativeSession {
    id: Uuid,
    /// Shown exactly once. Use it as the bearer token for subsequent
    /// requests.
    secret: String,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct BrowserAuthResponse {
    account: AccountDto,
    session: BrowserSession,
}

#[derive(Debug, Serialize)]
struct NativeAuthResponse {
    account: AccountDto,
    session: NativeSession,
}

#[derive(Debug, Serialize)]
struct MeResponse {
    account: AccountDto,
    session_id: Uuid,
}

#[derive(Debug, Serialize)]
struct SessionsResponse {
    sessions: Vec<SessionSummary>,
}

// ---------- browser handlers ----------

async fn signup_browser_handler(
    State(state): State<AppState>,
    Json(input): Json<SignupInput>,
) -> Result<Response, ApiError> {
    let result = accounts::signup(&state.db, state.deployment_mode, input).await?;
    let account = result.account;
    let issued = state.sessions.issue(account.id, None).await?;
    let csrf = generate_csrf_token();
    let max_age = (issued.expires_at - Utc::now()).num_seconds().max(0);

    let body = Json(BrowserAuthResponse {
        account: account.into(),
        session: BrowserSession {
            id: issued.id,
            expires_at: issued.expires_at,
        },
    });
    let mut resp = (StatusCode::CREATED, body).into_response();
    attach_session_cookies(
        resp.headers_mut(),
        &issued.secret,
        &csrf,
        max_age,
        state.secure_cookies,
    );
    Ok(resp)
}

async fn login_browser_handler(
    State(state): State<AppState>,
    Json(input): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    let account =
        accounts::verify_password(&state.db, &input.email, &input.password).await?;
    let issued = state.sessions.issue(account.id, input.label).await?;
    let csrf = generate_csrf_token();
    let max_age = (issued.expires_at - Utc::now()).num_seconds().max(0);

    let body = Json(BrowserAuthResponse {
        account: account.into(),
        session: BrowserSession {
            id: issued.id,
            expires_at: issued.expires_at,
        },
    });
    let mut resp = body.into_response();
    attach_session_cookies(
        resp.headers_mut(),
        &issued.secret,
        &csrf,
        max_age,
        state.secure_cookies,
    );
    Ok(resp)
}

// ---------- native handlers ----------

async fn signup_native_handler(
    State(state): State<AppState>,
    Json(input): Json<SignupInput>,
) -> Result<Response, ApiError> {
    let result = accounts::signup(&state.db, state.deployment_mode, input).await?;
    let account = result.account;
    let issued = state.sessions.issue(account.id, None).await?;
    let body = Json(NativeAuthResponse {
        account: account.into(),
        session: NativeSession {
            id: issued.id,
            secret: issued.secret,
            expires_at: issued.expires_at,
        },
    });
    Ok((StatusCode::CREATED, body).into_response())
}

async fn login_native_handler(
    State(state): State<AppState>,
    Json(input): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    let account =
        accounts::verify_password(&state.db, &input.email, &input.password).await?;
    let issued = state.sessions.issue(account.id, input.label).await?;
    let body = Json(NativeAuthResponse {
        account: account.into(),
        session: NativeSession {
            id: issued.id,
            secret: issued.secret,
            expires_at: issued.expires_at,
        },
    });
    Ok(body.into_response())
}

fn attach_session_cookies(
    headers: &mut HeaderMap,
    session_secret: &str,
    csrf_token: &str,
    max_age_secs: i64,
    secure: bool,
) {
    headers.append(
        header::SET_COOKIE,
        cookies::build_session(session_secret, max_age_secs, secure),
    );
    headers.append(
        header::SET_COOKIE,
        cookies::build_csrf(csrf_token, max_age_secs, secure),
    );
}

fn generate_csrf_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

async fn me_handler(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
) -> Result<Json<MeResponse>, ApiError> {
    let account = accounts::by_id(&state.db, ctx.account_id).await?;
    Ok(Json(MeResponse {
        account: account.into(),
        session_id: ctx.session_id,
    }))
}

async fn sessions_handler(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
) -> Result<Json<SessionsResponse>, ApiError> {
    let sessions = state.sessions.list_for_account(ctx.account_id).await?;
    Ok(Json(SessionsResponse { sessions }))
}

#[derive(Debug, Deserialize)]
struct UpdateAccountRequest {
    display_name: String,
}

async fn update_account_handler(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
    Json(input): Json<UpdateAccountRequest>,
) -> Result<Json<AccountDto>, ApiError> {
    let updated =
        accounts::update_display_name(&state.db, ctx.account_id, &input.display_name).await?;
    Ok(Json(updated.into()))
}

#[derive(Debug, Deserialize)]
struct ChangePasswordRequest {
    current_password: String,
    new_password: String,
}

async fn change_password_handler(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
    Json(input): Json<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    accounts::change_password(
        &state.db,
        ctx.account_id,
        &input.current_password,
        &input.new_password,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn logout_handler(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
) -> Result<Response, ApiError> {
    state.sessions.revoke(ctx.session_id).await?;
    // Drop any in-memory content keys this session published. Without
    // this, an authenticated logout would leave decryption live until
    // the per-key TTL expired — see `session_keys::revoke_for_session`.
    state.session_keys.revoke_for_session(ctx.session_id);
    let mut resp = StatusCode::NO_CONTENT.into_response();
    resp.headers_mut().append(
        header::SET_COOKIE,
        cookies::clear_session(state.secure_cookies),
    );
    resp.headers_mut()
        .append(header::SET_COOKIE, cookies::clear_csrf(state.secure_cookies));
    Ok(resp)
}

// ---------- session middleware ----------

/// Resolves the caller's session. Tries `Authorization: Bearer` first
/// (desktop / native clients / agents), then falls back to the
/// `orchext_session` cookie (browser SPA). Tags the resulting
/// `SessionContext` with `AuthSource` so the CSRF guard can decide
/// whether the request needs a double-submit token.
pub async fn session_auth(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let ctx = if let Some(bearer) = bearer_from_headers(req.headers()) {
        state.sessions.authenticate(&bearer, AuthSource::Bearer).await?
    } else if let Some(cookie) = session_cookie(req.headers()) {
        state
            .sessions
            .authenticate(&cookie, AuthSource::Cookie)
            .await?
    } else {
        return Err(ApiError::Unauthorized);
    };
    req.extensions_mut().insert(ctx);
    Ok(next.run(req).await)
}

/// CSRF middleware. Runs after `session_auth` so it can read the
/// `SessionContext`. Pass-through unless:
///   * the method mutates state (POST / PUT / PATCH / DELETE), AND
///   * the session was authenticated via cookie.
/// In that case the request must double-submit the CSRF token —
/// `X-Orchext-CSRF` header value must match the `orchext_csrf` cookie
/// value, constant-time-compared.
pub async fn csrf_guard(req: Request, next: Next) -> Result<Response, ApiError> {
    let method = req.method().clone();
    let mutates = matches!(
        method.as_str(),
        "POST" | "PUT" | "PATCH" | "DELETE"
    );
    let auth_source = req
        .extensions()
        .get::<SessionContext>()
        .map(|c| c.auth_source);

    if !mutates || auth_source != Some(AuthSource::Cookie) {
        return Ok(next.run(req).await);
    }

    let header_token = req
        .headers()
        .get("x-orchext-csrf")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let cookie_token =
        cookies::parse(req.headers()).get(cookies::CSRF_COOKIE).cloned();

    match (header_token, cookie_token) {
        (Some(h), Some(c)) if !h.is_empty() && constant_time_eq(h.as_bytes(), c.as_bytes()) => {
            Ok(next.run(req).await)
        }
        _ => Err(ApiError::CsrfFailed),
    }
}

fn session_cookie(headers: &HeaderMap) -> Option<String> {
    cookies::parse(headers).remove(cookies::SESSION_COOKIE)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.ct_eq(b).into()
}

fn bearer_from_headers(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, value) = raw.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let v = value.trim();
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_parses_standard() {
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            "Bearer ocx_example".parse().unwrap(),
        );
        assert_eq!(bearer_from_headers(&h).as_deref(), Some("ocx_example"));
    }

    #[test]
    fn bearer_case_insensitive_scheme() {
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            "bearer ocx_example".parse().unwrap(),
        );
        assert_eq!(bearer_from_headers(&h).as_deref(), Some("ocx_example"));
    }

    #[test]
    fn bearer_rejects_other_schemes() {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, "Basic abc123".parse().unwrap());
        assert!(bearer_from_headers(&h).is_none());
    }

    #[test]
    fn bearer_absent_returns_none() {
        let h = HeaderMap::new();
        assert!(bearer_from_headers(&h).is_none());
    }
}
