use thiserror::Error;

/// Errors from the sync client. Callers typically surface these to the
/// user after translation; `Unauthorized` and `VersionConflict` are the
/// only two that clients regularly want to branch on.
#[derive(Debug, Error)]
pub enum SyncError {
    #[error("server error {status}: {tag} — {message}")]
    Server {
        status: u16,
        tag: String,
        message: String,
    },

    #[error("unauthorized")]
    Unauthorized,

    #[error("not found")]
    NotFound,

    #[error("version conflict")]
    VersionConflict,

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("url parse error: {0}")]
    Url(#[from] url::ParseError),

    #[error("document parse error: {0}")]
    Document(String),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, SyncError>;
