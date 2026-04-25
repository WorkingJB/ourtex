use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("argon2 error: {0}")]
    Argon2(String),

    #[error("token not recognized")]
    UnknownToken,

    #[error("token revoked")]
    Revoked,

    #[error("token expired")]
    Expired,

    #[error("invalid token secret format")]
    InvalidSecret,

    #[error("scope must be non-empty")]
    EmptyScope,

    #[error("token not found: {0}")]
    NotFound(String),

    #[error("invalid scope label: {0:?}")]
    InvalidScope(String),
}

pub type Result<T> = std::result::Result<T, AuthError>;
