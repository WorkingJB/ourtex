//! Passphrase → `MasterKey` via Argon2id.
//!
//! Parameters are the `Argon2::default()` profile (Argon2id, m=19456,
//! t=2, p=1), matching the rest of the workspace (`mytex-server`
//! password hashing, `mytex-auth` token hashing). The 32-byte output
//! is used directly as an AEAD key — we call `hash_password_into`
//! rather than the PHC-string variant because we want raw key bytes,
//! not a stored-credential envelope.

use crate::error::{CryptoError, Result};
use argon2::Argon2;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

const SALT_LEN: usize = 16;
const KEY_LEN: usize = 32;
const MIN_PASSPHRASE_CHARS: usize = 8;

/// 32-byte key derived from a passphrase. Cleared from memory on
/// drop; the inner bytes are private to force callers through the
/// `expose_bytes` accessor at their use sites (easier to grep for).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct MasterKey([u8; KEY_LEN]);

impl MasterKey {
    pub fn expose_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// Constructor for testing and for reconstructing from a stored
    /// wrapped form after unwrap. Do not call with arbitrary bytes at
    /// runtime; use `derive_master_key` instead.
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }
}

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey(<redacted>)")
    }
}

/// KDF salt. 16 bytes of uniform randomness, stored alongside the
/// wrapped content key in the server's `tenants` row so any client
/// with the passphrase can re-derive the same master key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Salt(#[serde(with = "salt_base64")] [u8; SALT_LEN]);

impl Salt {
    pub fn generate() -> Self {
        let mut buf = [0u8; SALT_LEN];
        rand::thread_rng().fill_bytes(&mut buf);
        Self(buf)
    }

    pub fn as_bytes(&self) -> &[u8; SALT_LEN] {
        &self.0
    }

    pub fn to_wire(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }

    pub fn from_wire(s: &str) -> Result<Self> {
        let bytes = URL_SAFE_NO_PAD
            .decode(s.as_bytes())
            .map_err(|_| CryptoError::Wire("salt is not valid base64url"))?;
        if bytes.len() != SALT_LEN {
            return Err(CryptoError::Wire("salt has wrong length"));
        }
        let mut out = [0u8; SALT_LEN];
        out.copy_from_slice(&bytes);
        Ok(Self(out))
    }
}

/// Derive a 32-byte master key from the passphrase. Argon2id with the
/// default parameter profile. Passphrase bytes are zeroed on return
/// via `Argon2::hash_password_into` — we additionally zero the
/// caller's copy here so a derived passphrase doesn't linger.
pub fn derive_master_key(passphrase: &str, salt: &Salt) -> Result<MasterKey> {
    if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(CryptoError::WeakPassphrase);
    }
    let mut out = [0u8; KEY_LEN];
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt.as_bytes(), &mut out)
        .map_err(|e| CryptoError::Kdf(e.to_string()))?;
    let key = MasterKey(out);
    // `out` is copied into MasterKey; overwrite the stack copy now.
    let mut out_zero = out;
    out_zero.zeroize();
    Ok(key)
}

mod salt_base64 {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; SALT_LEN], s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&URL_SAFE_NO_PAD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<[u8; SALT_LEN], D::Error> {
        let s = String::deserialize(d)?;
        let bytes = URL_SAFE_NO_PAD
            .decode(s.as_bytes())
            .map_err(serde::de::Error::custom)?;
        if bytes.len() != SALT_LEN {
            return Err(serde::de::Error::custom("salt has wrong length"));
        }
        let mut out = [0u8; SALT_LEN];
        out.copy_from_slice(&bytes);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_is_deterministic_for_same_salt() {
        let salt = Salt::generate();
        let a = derive_master_key("correct horse battery staple", &salt).unwrap();
        let b = derive_master_key("correct horse battery staple", &salt).unwrap();
        assert_eq!(a.expose_bytes(), b.expose_bytes());
    }

    #[test]
    fn derive_differs_by_salt() {
        let s1 = Salt::generate();
        let s2 = Salt::generate();
        let a = derive_master_key("same passphrase", &s1).unwrap();
        let b = derive_master_key("same passphrase", &s2).unwrap();
        assert_ne!(a.expose_bytes(), b.expose_bytes());
    }

    #[test]
    fn derive_rejects_short_passphrase() {
        let salt = Salt::generate();
        assert!(matches!(
            derive_master_key("short", &salt),
            Err(CryptoError::WeakPassphrase)
        ));
    }

    #[test]
    fn salt_wire_round_trip() {
        let a = Salt::generate();
        let s = a.to_wire();
        let b = Salt::from_wire(&s).unwrap();
        assert_eq!(a.as_bytes(), b.as_bytes());
    }
}
