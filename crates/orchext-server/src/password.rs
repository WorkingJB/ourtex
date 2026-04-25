//! Argon2id password hashing.
//!
//! Matches `ourtex-auth`'s approach for token secrets (D15): Argon2id
//! with per-entry random salt, storing only the encoded hash. Verify
//! is constant-time inside `argon2`'s implementation.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, PasswordVerifier, SaltString},
    Argon2, PasswordHash,
};

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("password hashing failed: {0}")]
    Hash(String),
    #[error("stored hash is malformed")]
    MalformedHash,
}

/// Hash a password with Argon2id + a fresh random salt. Returns the
/// encoded PHC string suitable for storage.
pub fn hash(password: &str) -> Result<String, PasswordError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| PasswordError::Hash(e.to_string()))
}

/// Verify a candidate password against a stored PHC string. Any
/// mismatch — malformed hash, wrong password, whatever — returns
/// `Ok(false)`. A malformed hash is treated distinctly so callers
/// can log it rather than silently count a real password attempt.
pub fn verify(candidate: &str, stored: &str) -> Result<bool, PasswordError> {
    let parsed = PasswordHash::new(stored).map_err(|_| PasswordError::MalformedHash)?;
    Ok(Argon2::default()
        .verify_password(candidate.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_ok() {
        let h = hash("correct horse battery staple").unwrap();
        assert!(verify("correct horse battery staple", &h).unwrap());
    }

    #[test]
    fn wrong_password_verifies_false() {
        let h = hash("password1").unwrap();
        assert!(!verify("password2", &h).unwrap());
    }

    #[test]
    fn malformed_hash_errors() {
        let err = verify("anything", "not-a-phc-string").unwrap_err();
        assert!(matches!(err, PasswordError::MalformedHash));
    }

    #[test]
    fn same_password_produces_distinct_hashes() {
        let a = hash("same").unwrap();
        let b = hash("same").unwrap();
        assert_ne!(a, b, "salt must differ per hash");
    }
}
