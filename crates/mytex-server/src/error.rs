//! HTTP error type for the server.
//!
//! `ApiError` maps a domain failure to `(StatusCode, JSON body)`.
//! Handlers return `Result<Json<T>, ApiError>`; the `IntoResponse`
//! impl does the rest.
//!
//! Enumeration resistance (ARCH §5.3 / MCP.md §7): authentication
//! failures are collapsed to a single `unauthorized` error so an
//! attacker probing the signup/login surface cannot distinguish
//! "no such account" from "wrong password" from "revoked session."

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("unauthorized")]
    Unauthorized,

    #[error("not found")]
    NotFound,

    #[error("conflict: {0}")]
    Conflict(&'static str),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// Tenant's content key is not currently live on the server. The
    /// requested read/write on an encrypted resource can't proceed
    /// until an unlocked client re-publishes the key.
    #[error("vault locked")]
    VaultLocked,

    #[error("internal server error")]
    Internal(#[source] BoxError),
}

pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

impl ApiError {
    fn status(&self) -> StatusCode {
        match self {
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::NotFound => StatusCode::NOT_FOUND,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::InvalidArgument(_) => StatusCode::BAD_REQUEST,
            // 423 Locked is the right semantic — the resource exists
            // but can't currently be operated on. Clients reconnect
            // an unlocked session and retry.
            ApiError::VaultLocked => StatusCode::LOCKED,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn tag(&self) -> &'static str {
        match self {
            ApiError::Unauthorized => "unauthorized",
            ApiError::NotFound => "not_found",
            ApiError::Conflict(_) => "conflict",
            ApiError::InvalidArgument(_) => "invalid_argument",
            ApiError::VaultLocked => "vault_locked",
            ApiError::Internal(_) => "server_error",
        }
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    tag: &'static str,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // Internal errors are logged in full; the response stays opaque
        // so nothing about the server's state leaks to the caller.
        if let ApiError::Internal(err) = &self {
            tracing::error!(error = %err, "internal server error");
        }
        let status = self.status();
        let body = ErrorBody {
            error: ErrorDetail {
                tag: self.tag(),
                message: match &self {
                    ApiError::Unauthorized => "authentication required".into(),
                    ApiError::NotFound => "resource not found".into(),
                    ApiError::Conflict(m) => (*m).to_string(),
                    ApiError::InvalidArgument(m) => m.clone(),
                    ApiError::VaultLocked => {
                        "workspace content key is not currently live on the server; reconnect an unlocked client".into()
                    }
                    ApiError::Internal(_) => "internal server error".into(),
                },
            },
        };
        (status, Json(body)).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::Internal(Box::new(e))
    }
}
