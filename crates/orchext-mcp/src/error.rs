use crate::rpc::RpcError;
use serde_json::json;
use thiserror::Error;

/// Ourtex MCP error codes, per `MCP.md` §7.
///
/// `not_authorized` is deliberately ambiguous: out-of-scope, missing, and
/// revoked-direct-access all map to it so the error itself cannot be used
/// to enumerate vault contents.
#[derive(Debug, Clone, Error)]
pub enum McpError {
    #[error("unexpected server error: {0}")]
    Server(String),

    #[error("token revoked")]
    TokenRevoked,

    #[error("not authorized")]
    NotAuthorized,

    #[error("version conflict")]
    VersionConflict,

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("rate limited")]
    RateLimited { retry_after_ms: u64 },

    #[error("vault locked")]
    VaultLocked,

    #[error("proposals disabled")]
    ProposalsDisabled,

    /// Standard JSON-RPC parse / method-not-found errors, used before the
    /// Ourtex-specific code range applies.
    #[error("method not found: {0}")]
    MethodNotFound(String),

    #[error("parse error: {0}")]
    ParseError(String),
}

impl McpError {
    pub fn code(&self) -> i32 {
        match self {
            Self::ParseError(_) => -32700,
            Self::MethodNotFound(_) => -32601,
            Self::Server(_) => -32000,
            Self::TokenRevoked => -32001,
            Self::NotAuthorized => -32002,
            Self::VersionConflict => -32003,
            Self::InvalidArgument(_) => -32004,
            Self::RateLimited { .. } => -32005,
            Self::VaultLocked => -32006,
            Self::ProposalsDisabled => -32007,
        }
    }

    pub fn tag(&self) -> &'static str {
        match self {
            Self::ParseError(_) => "parse_error",
            Self::MethodNotFound(_) => "method_not_found",
            Self::Server(_) => "server_error",
            Self::TokenRevoked => "token_revoked",
            Self::NotAuthorized => "not_authorized",
            Self::VersionConflict => "version_conflict",
            Self::InvalidArgument(_) => "invalid_argument",
            Self::RateLimited { .. } => "rate_limited",
            Self::VaultLocked => "vault_locked",
            Self::ProposalsDisabled => "proposals_disabled",
        }
    }

    pub fn to_rpc(&self) -> RpcError {
        let mut data = json!({ "tag": self.tag() });
        if let Self::RateLimited { retry_after_ms } = self {
            data["retry_after_ms"] = json!(retry_after_ms);
        }
        RpcError::new(self.code(), self.to_string()).with_data(data)
    }
}

pub type Result<T> = std::result::Result<T, McpError>;
