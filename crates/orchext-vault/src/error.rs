use thiserror::Error;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yml::Error),

    #[error("missing frontmatter: document must begin with a `---` delimiter")]
    MissingFrontmatter,

    #[error("unterminated frontmatter: no closing `---` delimiter")]
    UnterminatedFrontmatter,

    #[error("invalid id: {0:?}")]
    InvalidId(String),

    #[error("invalid visibility label: {0:?}")]
    InvalidVisibility(String),

    #[error("document not found: {0}")]
    NotFound(String),

    #[error("vault version {found} is newer than supported ({supported})")]
    VersionTooNew { found: String, supported: String },
}

pub type Result<T> = std::result::Result<T, VaultError>;
