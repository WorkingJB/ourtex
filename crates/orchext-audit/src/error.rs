use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("malformed entry at line {line}: {reason}")]
    Malformed { line: u64, reason: String },

    #[error("chain broken at seq {seq}: {reason}")]
    ChainBroken { seq: u64, reason: String },

    #[error("invalid actor: {0:?}")]
    InvalidActor(String),
}

pub type Result<T> = std::result::Result<T, AuditError>;
