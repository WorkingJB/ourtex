use thiserror::Error;

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("vault error: {0}")]
    Vault(String),

    #[error("join error: {0}")]
    Join(String),

    #[error("invalid document: {0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, IndexError>;
