use crate::error::{AuthError, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;

pub const SECRET_PREFIX: &str = "otx_";
const SECRET_RANDOM_BYTES: usize = 32;

/// A freshly minted or presented token secret. Never logged. Never persisted
/// in plaintext; only the Argon2id hash touches disk.
pub struct TokenSecret(String);

impl TokenSecret {
    pub fn generate() -> Self {
        let mut bytes = [0u8; SECRET_RANDOM_BYTES];
        rand::thread_rng().fill_bytes(&mut bytes);
        let encoded = URL_SAFE_NO_PAD.encode(bytes);
        Self(format!("{SECRET_PREFIX}{encoded}"))
    }

    pub fn from_str(s: &str) -> Result<Self> {
        if !s.starts_with(SECRET_PREFIX) {
            return Err(AuthError::InvalidSecret);
        }
        let payload = &s[SECRET_PREFIX.len()..];
        if payload.is_empty() {
            return Err(AuthError::InvalidSecret);
        }
        Ok(Self(s.to_string()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for TokenSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TokenSecret").field(&"<redacted>").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_prefixed_secrets() {
        let s = TokenSecret::generate();
        assert!(s.expose().starts_with(SECRET_PREFIX));
        assert!(s.expose().len() > SECRET_PREFIX.len() + 10);
    }

    #[test]
    fn parses_well_formed_secrets() {
        assert!(TokenSecret::from_str("otx_abc123").is_ok());
    }

    #[test]
    fn rejects_malformed_secrets() {
        assert!(TokenSecret::from_str("").is_err());
        assert!(TokenSecret::from_str("abc").is_err());
        assert!(TokenSecret::from_str("otx_").is_err());
    }

    #[test]
    fn debug_does_not_leak() {
        let s = TokenSecret::generate();
        let debug_output = format!("{s:?}");
        assert!(!debug_output.contains(s.expose()));
        assert!(debug_output.contains("redacted"));
    }
}
