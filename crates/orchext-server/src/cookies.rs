//! Cookie helpers for session + CSRF tokens.
//!
//! Two cookies, both `Path=/` and `SameSite=Lax`:
//!   * `ourtex_session` — opaque session secret, `HttpOnly` so JS
//!     cannot read it. The browser attaches it to every same-origin
//!     request automatically.
//!   * `ourtex_csrf` — random token, **not** `HttpOnly` so the web
//!     client can read `document.cookie`, copy the value, and send it
//!     back as `X-Ourtex-CSRF` on state-changing requests
//!     (double-submit pattern).
//!
//! `Secure` is gated on `AppState::secure_cookies`. Browsers refuse to
//! store `Secure` cookies over plain HTTP, so local dev with HTTP needs
//! `OURTEX_SECURE_COOKIES=0`.
//!
//! `SameSite=Lax` (not `Strict`) is the SPA default. `Strict` blocks
//! cookies on top-level cross-site navigations which would break, e.g.,
//! a user clicking a magic link or a deep link from email. `Lax` still
//! prevents form-submit CSRF, and the double-submit token covers the
//! POST/PUT/DELETE case anyway.

use axum::http::{HeaderMap, HeaderValue, header};
use std::collections::HashMap;

pub const SESSION_COOKIE: &str = "ourtex_session";
pub const CSRF_COOKIE: &str = "ourtex_csrf";

/// Parse all `Cookie:` headers into a name→value map. Last-wins on
/// duplicates, which matches RFC 6265 sloppily but is fine for our
/// use case (we only read two well-known names).
pub fn parse(headers: &HeaderMap) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for raw in headers.get_all(header::COOKIE).iter() {
        let Ok(s) = raw.to_str() else { continue };
        for part in s.split(';') {
            let part = part.trim();
            if let Some((k, v)) = part.split_once('=') {
                out.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    out
}

/// Build the `Set-Cookie` value for a session cookie. `max_age_secs`
/// controls expiry; pass `0` (and call `clear_session`) to remove.
pub fn build_session(value: &str, max_age_secs: i64, secure: bool) -> HeaderValue {
    let secure_flag = if secure { "; Secure" } else { "" };
    let v = format!(
        "{SESSION_COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}{secure_flag}"
    );
    HeaderValue::from_str(&v).expect("session cookie value is ascii")
}

/// CSRF cookie — readable from JS so the SPA can mirror it back into
/// the `X-Ourtex-CSRF` header.
pub fn build_csrf(value: &str, max_age_secs: i64, secure: bool) -> HeaderValue {
    let secure_flag = if secure { "; Secure" } else { "" };
    let v = format!(
        "{CSRF_COOKIE}={value}; Path=/; SameSite=Lax; Max-Age={max_age_secs}{secure_flag}"
    );
    HeaderValue::from_str(&v).expect("csrf cookie value is ascii")
}

pub fn clear_session(secure: bool) -> HeaderValue {
    build_session("", 0, secure)
}

pub fn clear_csrf(secure: bool) -> HeaderValue {
    build_csrf("", 0, secure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[test]
    fn parse_single_cookie() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, "ourtex_session=abc123".parse().unwrap());
        let cookies = parse(&h);
        assert_eq!(cookies.get("ourtex_session").map(String::as_str), Some("abc123"));
    }

    #[test]
    fn parse_multiple_in_one_header() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            "ourtex_session=abc; ourtex_csrf=xyz".parse().unwrap(),
        );
        let cookies = parse(&h);
        assert_eq!(cookies.get("ourtex_session").map(String::as_str), Some("abc"));
        assert_eq!(cookies.get("ourtex_csrf").map(String::as_str), Some("xyz"));
    }

    #[test]
    fn parse_handles_no_cookie_header() {
        let h = HeaderMap::new();
        assert!(parse(&h).is_empty());
    }

    #[test]
    fn build_session_includes_httponly() {
        let v = build_session("tok", 3600, true);
        let s = v.to_str().unwrap();
        assert!(s.contains("HttpOnly"));
        assert!(s.contains("Secure"));
        assert!(s.contains("SameSite=Lax"));
    }

    #[test]
    fn build_csrf_omits_httponly() {
        let v = build_csrf("tok", 3600, false);
        let s = v.to_str().unwrap();
        assert!(!s.contains("HttpOnly"));
        assert!(!s.contains("Secure"));
    }
}
