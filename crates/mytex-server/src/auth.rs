//! Authentication HTTP surface: `/v1/auth/*`.
//!
//! Routes:
//! - `POST   /v1/auth/signup`   — create an account
//! - `POST   /v1/auth/login`    — exchange credentials for a session
//! - `GET    /v1/auth/me`       — current account (authenticated)
//! - `GET    /v1/auth/sessions` — list active sessions (authenticated)
//! - `DELETE /v1/auth/logout`   — revoke the current session
//!
//! Session middleware on the authenticated routes extracts the bearer
//! token, validates it against the sessions table, and attaches a
//! `SessionContext` to the request extensions.

use crate::{
    accounts::{self, Account, SignupInput},
    error::ApiError,
    sessions::{SessionContext, SessionSummary},
    AppState,
};
use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::{delete, get, post},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Build the `/v1/auth/*` router. Takes an `AppState` by value so the
/// session-auth middleware on protected routes can be constructed with
/// `from_fn_with_state` (which binds state at layer-construction time,
/// not at request time).
pub fn router(state: AppState) -> Router<AppState> {
    let public = Router::new()
        .route("/signup", post(signup_handler))
        .route("/login", post(login_handler));

    let protected = Router::new()
        .route("/me", get(me_handler))
        .route("/sessions", get(sessions_handler))
        .route("/logout", delete(logout_handler))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            session_auth,
        ));

    public.merge(protected)
}

// ---------- handlers ----------

#[derive(Debug, Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
    label: Option<String>,
}

#[derive(Debug, Serialize)]
struct LoginResponse {
    account: AccountDto,
    session: SessionIssuedDto,
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

#[derive(Debug, Serialize)]
struct SessionIssuedDto {
    id: Uuid,
    /// Shown exactly once. Use it as the bearer token for subsequent
    /// requests.
    secret: String,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct SignupResponse {
    account: AccountDto,
    session: SessionIssuedDto,
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

async fn signup_handler(
    State(state): State<AppState>,
    Json(input): Json<SignupInput>,
) -> Result<(StatusCode, Json<SignupResponse>), ApiError> {
    let account = accounts::signup(&state.db, input).await?;
    let issued = state.sessions.issue(account.id, None).await?;
    Ok((
        StatusCode::CREATED,
        Json(SignupResponse {
            account: account.into(),
            session: SessionIssuedDto {
                id: issued.id,
                secret: issued.secret,
                expires_at: issued.expires_at,
            },
        }),
    ))
}

async fn login_handler(
    State(state): State<AppState>,
    Json(input): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let account =
        accounts::verify_password(&state.db, &input.email, &input.password).await?;
    let issued = state.sessions.issue(account.id, input.label).await?;
    Ok(Json(LoginResponse {
        account: account.into(),
        session: SessionIssuedDto {
            id: issued.id,
            secret: issued.secret,
            expires_at: issued.expires_at,
        },
    }))
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

async fn logout_handler(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
) -> Result<StatusCode, ApiError> {
    state.sessions.revoke(ctx.session_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- session middleware ----------

/// Validates `Authorization: Bearer <token>`, attaches `SessionContext`
/// to the request extensions, and hands off to the next handler.
/// Any failure short-circuits with `401 Unauthorized`.
pub async fn session_auth(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = bearer_from_headers(req.headers()).ok_or(ApiError::Unauthorized)?;
    let ctx = state.sessions.authenticate(&token).await?;
    req.extensions_mut().insert(ctx);
    Ok(next.run(req).await)
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
            "Bearer mtx_example".parse().unwrap(),
        );
        assert_eq!(bearer_from_headers(&h).as_deref(), Some("mtx_example"));
    }

    #[test]
    fn bearer_case_insensitive_scheme() {
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            "bearer mtx_example".parse().unwrap(),
        );
        assert_eq!(bearer_from_headers(&h).as_deref(), Some("mtx_example"));
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
