//! Content key: the 32-byte AEAD key that actually encrypts document
//! bodies. Lives in two forms:
//!
//! - **Plain**: the raw `[u8; 32]`, kept in client memory after
//!   unwrap + on the server while a client has an active session.
//! - **Wrapped**: the plain key encrypted under the
//!   passphrase-derived `MasterKey`, stored in the server's
//!   `tenants.wrapped_content_key` column. Any client with the
//!   passphrase (and access to the stored salt) can unwrap.
//!
//! In the v1.1 scope this is also the "session key" from `ARCH.md`
//! §3.4 — there's only one key per workspace today. Per-doc keys
//! and key rotation are 2b.3+ follow-ups.

use crate::{
    aead::{open, seal, SealedBlob, KEY_LEN},
    error::{CryptoError, Result},
    kdf::MasterKey,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::{rngs::OsRng, RngCore};
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct ContentKey([u8; KEY_LEN]);

impl ContentKey {
    /// Fresh random key. Called once per workspace at
    /// `init-crypto` time.
    pub fn generate() -> Self {
        let mut buf = [0u8; KEY_LEN];
        OsRng.fill_bytes(&mut buf);
        Self(buf)
    }

    pub fn expose_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// The compact wire form used on `/v1/t/:tid/session-key` —
    /// base64url-nopad of the 32 raw key bytes. The server stores
    /// the bytes in memory only, never on disk in this form.
    pub fn to_wire(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }

    pub fn from_wire(s: &str) -> Result<Self> {
        let bytes = URL_SAFE_NO_PAD
            .decode(s.as_bytes())
            .map_err(|_| CryptoError::Wire("content key is not valid base64url"))?;
        if bytes.len() != KEY_LEN {
            return Err(CryptoError::Wire("content key has wrong length"));
        }
        let mut out = [0u8; KEY_LEN];
        out.copy_from_slice(&bytes);
        Ok(Self(out))
    }
}

impl std::fmt::Debug for ContentKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ContentKey(<redacted>)")
    }
}

/// Encrypt the content key under the passphrase-derived master key.
/// Output is a `SealedBlob` the server can hold as `TEXT` alongside
/// the salt — rehydrating a client only requires the passphrase.
pub fn wrap_content_key(content: &ContentKey, master: &MasterKey) -> Result<SealedBlob> {
    seal(content.expose_bytes(), master.expose_bytes())
}

pub fn unwrap_content_key(blob: &SealedBlob, master: &MasterKey) -> Result<ContentKey> {
    let bytes = open(blob, master.expose_bytes())?;
    if bytes.len() != KEY_LEN {
        return Err(CryptoError::Open);
    }
    let mut out = [0u8; KEY_LEN];
    out.copy_from_slice(&bytes);
    Ok(ContentKey(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kdf::{derive_master_key, Salt};

    #[test]
    fn wrap_unwrap_round_trip() {
        let salt = Salt::generate();
        let master = derive_master_key("correct horse battery staple", &salt).unwrap();
        let content = ContentKey::generate();
        let wrapped = wrap_content_key(&content, &master).unwrap();
        let back = unwrap_content_key(&wrapped, &master).unwrap();
        assert_eq!(back.expose_bytes(), content.expose_bytes());
    }

    #[test]
    fn wrong_passphrase_fails_to_unwrap() {
        let salt = Salt::generate();
        let good = derive_master_key("correct horse battery staple", &salt).unwrap();
        let bad = derive_master_key("wrong horse battery staple", &salt).unwrap();
        let content = ContentKey::generate();
        let wrapped = wrap_content_key(&content, &good).unwrap();
        assert!(unwrap_content_key(&wrapped, &bad).is_err());
    }

    #[test]
    fn wire_round_trip() {
        let c = ContentKey::generate();
        let s = c.to_wire();
        let back = ContentKey::from_wire(&s).unwrap();
        assert_eq!(back.expose_bytes(), c.expose_bytes());
    }
}
