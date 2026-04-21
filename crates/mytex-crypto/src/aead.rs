//! XChaCha20-Poly1305 seal / open.
//!
//! XChaCha is chosen over plain ChaCha20-Poly1305 because its 192-bit
//! nonce is long enough to safely generate at random per call — no
//! counter or DB-backed nonce tracking. The 128-bit Poly1305 tag is
//! appended inside the ciphertext (aead crate convention).
//!
//! Wire format for a `SealedBlob` is `<24 byte nonce><ciphertext+tag>`
//! base64url-no-pad encoded. The nonce is not secret; bundling it
//! with the ciphertext keeps clients + server from having to ship
//! nonces out-of-band.

use crate::error::{CryptoError, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305, XNonce,
};
use rand::RngCore;

pub const NONCE_LEN: usize = 24;
pub const KEY_LEN: usize = 32;

#[derive(Debug, Clone)]
pub struct SealedBlob(Vec<u8>);

impl SealedBlob {
    pub fn to_wire(&self) -> String {
        URL_SAFE_NO_PAD.encode(&self.0)
    }

    pub fn from_wire(s: &str) -> Result<Self> {
        let bytes = URL_SAFE_NO_PAD
            .decode(s.as_bytes())
            .map_err(|_| CryptoError::Wire("sealed blob is not valid base64url"))?;
        if bytes.len() < NONCE_LEN + 16 {
            // nonce + at least the poly1305 tag
            return Err(CryptoError::Wire("sealed blob is too short"));
        }
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Encrypt `plaintext` under a 32-byte key, returning a sealed blob
/// that bundles a freshly-generated random nonce with the ciphertext
/// and authentication tag.
pub fn seal(plaintext: &[u8], key: &[u8; KEY_LEN]) -> Result<SealedBlob> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| CryptoError::Seal)?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(SealedBlob(out))
}

/// Decrypt a sealed blob. All failure modes (wrong key, tampered
/// ciphertext, truncated nonce) collapse to `CryptoError::Open`.
pub fn open(blob: &SealedBlob, key: &[u8; KEY_LEN]) -> Result<Vec<u8>> {
    if blob.0.len() < NONCE_LEN + 16 {
        return Err(CryptoError::Open);
    }
    let cipher = XChaCha20Poly1305::new(key.into());
    let (nonce_bytes, ct) = blob.0.split_at(NONCE_LEN);
    let nonce = XNonce::from_slice(nonce_bytes);
    cipher.decrypt(nonce, ct).map_err(|_| CryptoError::Open)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; KEY_LEN] {
        [0x42; KEY_LEN]
    }

    #[test]
    fn seal_open_round_trip() {
        let key = test_key();
        let msg = b"hello world";
        let blob = seal(msg, &key).unwrap();
        let out = open(&blob, &key).unwrap();
        assert_eq!(out, msg);
    }

    #[test]
    fn wrong_key_rejected() {
        let blob = seal(b"secret", &test_key()).unwrap();
        let mut other = test_key();
        other[0] ^= 1;
        assert!(matches!(open(&blob, &other), Err(CryptoError::Open)));
    }

    #[test]
    fn tampered_ciphertext_rejected() {
        let key = test_key();
        let mut blob = seal(b"important", &key).unwrap();
        // Flip a byte in the ciphertext (after the 24-byte nonce).
        blob.0[NONCE_LEN + 1] ^= 0x01;
        assert!(matches!(open(&blob, &key), Err(CryptoError::Open)));
    }

    #[test]
    fn two_seals_use_distinct_nonces() {
        let key = test_key();
        let a = seal(b"x", &key).unwrap();
        let b = seal(b"x", &key).unwrap();
        // Same plaintext + same key must produce different ciphertext
        // because the random nonce changes. If this ever fires, the
        // RNG is broken.
        assert_ne!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn wire_round_trip() {
        let key = test_key();
        let blob = seal(b"payload", &key).unwrap();
        let s = blob.to_wire();
        let back = SealedBlob::from_wire(&s).unwrap();
        assert_eq!(back.as_bytes(), blob.as_bytes());
        let out = open(&back, &key).unwrap();
        assert_eq!(out, b"payload");
    }

    #[test]
    fn from_wire_rejects_short_blob() {
        // Too short to hold nonce + tag.
        assert!(SealedBlob::from_wire(&URL_SAFE_NO_PAD.encode([0u8; 10])).is_err());
    }
}
