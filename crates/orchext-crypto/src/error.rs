use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("passphrase is too short (need ≥8 chars)")]
    WeakPassphrase,

    #[error("kdf failed: {0}")]
    Kdf(String),

    #[error("seal failed")]
    Seal,

    /// Used for every decryption failure: wrong key, tampered
    /// ciphertext, truncated nonce, bad base64. Collapsing these
    /// makes timing / oracle attacks less useful.
    #[error("decryption failed")]
    Open,

    #[error("wire format invalid: {0}")]
    Wire(&'static str),
}

pub type Result<T> = std::result::Result<T, CryptoError>;
