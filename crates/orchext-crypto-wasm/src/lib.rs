//! Browser-facing wrapper around `ourtex-crypto`.
//!
//! Keeps the JS-exposed surface minimal and stateless: four top-level
//! functions that take and return wire-form (base64url-nopad) strings.
//! The master key is derived inside each call and dropped before return
//! — nothing JS can hold is ever the raw passphrase-derived key.
//!
//! Wire-form outputs line up bit-for-bit with what `ourtex-crypto`
//! emits on the server, so wrapped blobs round-trip between browser
//! and Rust server without special-casing.

#![forbid(unsafe_code)]

use ourtex_crypto::{
    derive_master_key,
    unwrap_content_key as rust_unwrap,
    wrap_content_key as rust_wrap,
    ContentKey, CryptoError, Salt, SealedBlob,
};
use wasm_bindgen::prelude::*;

/// Fresh 16-byte KDF salt as base64url-nopad. Called once at
/// `init-crypto` time for a new workspace.
#[wasm_bindgen(js_name = generateSalt)]
pub fn generate_salt() -> String {
    Salt::generate().to_wire()
}

/// Fresh 32-byte content key as base64url-nopad. Called once at
/// `init-crypto` time; after that, the same key is unwrapped from
/// storage for the lifetime of the workspace (key rotation is future
/// work, see `phase-2b3-encryption.md`).
#[wasm_bindgen(js_name = generateContentKey)]
pub fn generate_content_key() -> String {
    ContentKey::generate().to_wire()
}

/// Derive a master key from `passphrase` + `salt_wire` (Argon2id) and
/// seal the content key under it. Output is the base64url-nopad
/// sealed blob that `POST /v1/t/:tid/vault/init-crypto` stores in
/// `tenants.wrapped_content_key`.
#[wasm_bindgen(js_name = wrapContentKey)]
pub fn wrap_content_key(
    content_wire: &str,
    passphrase: &str,
    salt_wire: &str,
) -> Result<String, JsError> {
    let salt = Salt::from_wire(salt_wire).map_err(to_js)?;
    let content = ContentKey::from_wire(content_wire).map_err(to_js)?;
    let master = derive_master_key(passphrase, &salt).map_err(to_js)?;
    let wrapped = rust_wrap(&content, &master).map_err(to_js)?;
    Ok(wrapped.to_wire())
}

/// Derive a master key from `passphrase` + `salt_wire` and unseal the
/// stored wrapped content key. Any failure — wrong passphrase, bad
/// wire, tampered blob — surfaces as the single `decryption failed`
/// error, matching the Rust crate's enumeration-resistance posture.
#[wasm_bindgen(js_name = unwrapContentKey)]
pub fn unwrap_content_key(
    wrapped_wire: &str,
    passphrase: &str,
    salt_wire: &str,
) -> Result<String, JsError> {
    let salt = Salt::from_wire(salt_wire).map_err(to_js)?;
    let wrapped = SealedBlob::from_wire(wrapped_wire).map_err(to_js)?;
    let master = derive_master_key(passphrase, &salt).map_err(to_js)?;
    let content = rust_unwrap(&wrapped, &master).map_err(to_js)?;
    Ok(content.to_wire())
}

fn to_js(err: CryptoError) -> JsError {
    JsError::new(&err.to_string())
}
