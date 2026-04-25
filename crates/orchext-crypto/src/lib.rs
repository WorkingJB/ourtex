//! Passphrase KDF + AEAD primitives powering Ourtex's at-rest
//! encryption (Phase 2b.3). Key hierarchy:
//!
//! ```text
//! passphrase  ──(Argon2id + salt)──▶  MasterKey
//! MasterKey   ──(XChaCha20-Poly1305)──▶ wraps a random ContentKey
//! ContentKey  ──(XChaCha20-Poly1305)──▶ seals document bodies
//! ```
//!
//! The salt + wrapped content key live on the server
//! (`tenants.wrapped_content_key`). Any client with the passphrase
//! can derive the master key, unwrap, and publish the content key
//! to the server's short-TTL in-memory store
//! (`POST /v1/t/:tid/session-key`). While a content key is live on
//! the server, writes are encrypted and reads are decrypted
//! server-side — the "session-bound decryption" posture from
//! `ARCH.md` §3.4.
//!
//! Scope cuts for this pass (see `docs/implementation-status.md`):
//!   - one content key per workspace (no per-doc keys)
//!   - no key rotation / versioning beyond a static `key_version = 1`
//!   - no OS keychain (master key held in client memory only)
//!
//! Browser builds: `cargo build --target wasm32-unknown-unknown` works
//! out of the box — `OsRng` routes through `getrandom`'s `js` backend
//! (activated by the wasm32-gated dep in `Cargo.toml`) to reach
//! `crypto.getRandomValues`. No `wasm` feature flag needed.

#![forbid(unsafe_code)]

pub mod aead;
pub mod content_key;
pub mod error;
pub mod kdf;

pub use aead::{open, seal, SealedBlob, KEY_LEN, NONCE_LEN};
pub use content_key::{unwrap_content_key, wrap_content_key, ContentKey};
pub use error::{CryptoError, Result};
pub use kdf::{derive_master_key, MasterKey, Salt};
