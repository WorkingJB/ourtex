//! OAuth 2.1 + PKCE client helper.
//!
//! `acquire_token()` runs the full authorization-code-with-PKCE dance
//! against an orchext-server: it generates a verifier+challenge, binds
//! a loopback listener for the redirect, opens the user's browser at
//! the consent URL on the orchext web app, captures the auth code
//! returned via 302, and exchanges it at `/v1/oauth/token` for an
//! `ocx_*` bearer token row in `mcp_tokens`.
//!
//! Designed to be embedded in agent integrations (Claude Code, custom
//! MCP-speaking tools, etc.) as a one-shot token acquisition helper.
//! The matching CLI (`orchext-oauth`) wraps it for the common case
//! where an integration shells out to acquire credentials.
//!
//! Threading model: the helper binds a temporary listener on
//! `127.0.0.1:0`, accepts a single GET, sends a friendly HTML body
//! back to the browser, then closes. The whole flow is bounded by a
//! caller-supplied timeout — if the user never approves, the future
//! resolves with `Error::Timeout` and the listener is dropped.

#![forbid(unsafe_code)]

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;
use uuid::Uuid;

/// Default browser-wait timeout. Long enough that the user can sign in
/// from cold, short enough that a forgotten flow doesn't pin the
/// listener forever.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// All inputs `acquire_token()` needs. Server URL is the orchext-server
/// API base (e.g. `https://orchext.example.com`). The consent URL is
/// derived from `consent_base` if provided, else from `server_url` —
/// this lets self-hosters point the consent UI at a different origin
/// than the API (web app behind one domain, API behind another).
#[derive(Debug, Clone)]
pub struct AcquireRequest {
    /// API base. Used to POST `/v1/oauth/token`. Trailing slash is
    /// trimmed by the helper.
    pub server_url: String,
    /// Web-app base for the consent URL. Defaults to `server_url`.
    pub consent_base: Option<String>,
    pub tenant_id: Uuid,
    /// Display name shown to the user on the consent screen and saved
    /// to `mcp_tokens.label`.
    pub client_label: String,
    /// Visibility labels — `public`, `work`, `personal`, `private`,
    /// or any custom label registered server-side.
    pub scope: Vec<String>,
    /// `read` (default) or `read_propose`.
    pub mode: Option<String>,
    /// Token TTL in days; server clamps to [1, 365]. None means
    /// server default (90 days).
    pub ttl_days: Option<i64>,
    pub max_docs: Option<i32>,
    pub max_bytes: Option<i64>,
    /// Hard cap on the browser/loopback wait. Defaults to
    /// [`DEFAULT_TIMEOUT`].
    pub timeout: Option<Duration>,
}

/// What the caller gets back on success — the bearer plus the metadata
/// the server returned alongside it. The bearer is the only "secret"
/// here; everything else is observational.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcquiredToken {
    /// `ocx_…` bearer. Caller is responsible for storing it securely
    /// (the CLI prints it to stdout; library callers usually pipe it
    /// into a keychain).
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub scope: String,
    pub tenant_id: Uuid,
    pub token_id: String,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("listener bind failed: {0}")]
    Bind(#[source] std::io::Error),
    #[error("could not open browser: {0}")]
    OpenBrowser(String),
    #[error("user did not complete authorization within {0:?}")]
    Timeout(Duration),
    #[error("user denied authorization")]
    Denied,
    #[error("authorization server returned error: {0}")]
    AuthServerError(String),
    #[error("redirect did not include an authorization code")]
    MissingCode,
    #[error("state mismatch — possible CSRF, refusing token exchange")]
    StateMismatch,
    #[error("malformed callback request")]
    BadCallback,
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("server URL invalid: {0}")]
    BadUrl(#[from] url::ParseError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Run the full PKCE flow. Returns once the token has been exchanged
/// or the timeout elapses.
pub async fn acquire_token(req: AcquireRequest) -> Result<AcquiredToken, Error> {
    let timeout = req.timeout.unwrap_or(DEFAULT_TIMEOUT);

    let verifier = generate_verifier();
    let challenge = compute_challenge(&verifier);
    let state = generate_state();

    // Bind on `127.0.0.1:0` and read back the chosen port. Loopback
    // only — never expose the auth-code receiver to other interfaces.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(Error::Bind)?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/cb");

    let consent_base = req
        .consent_base
        .as_deref()
        .unwrap_or(&req.server_url)
        .trim_end_matches('/');
    let consent_url = build_consent_url(
        consent_base,
        &req,
        &challenge,
        &redirect_uri,
        &state,
    )?;

    eprintln!("Opening browser to authorize...");
    eprintln!("If it doesn't open, visit:\n  {consent_url}");
    if let Err(e) = open_browser(consent_url.as_str()) {
        eprintln!("warning: {e}");
    }

    // Loop until we accept a connection that carries a /cb request.
    // Browsers occasionally probe `/favicon.ico` or other paths first
    // — answer those with 404 and keep waiting. Bounded by the
    // outer timeout.
    let callback = tokio::time::timeout(timeout, accept_callback(&listener, &state))
        .await
        .map_err(|_| Error::Timeout(timeout))??;

    let token = exchange_code(
        &req.server_url,
        &callback.code,
        &verifier,
        &redirect_uri,
    )
    .await?;
    Ok(token)
}

/// Accepted browser callback decoded from `?code=…&state=…`.
#[derive(Debug)]
struct Callback {
    code: String,
}

async fn accept_callback(
    listener: &TcpListener,
    expected_state: &str,
) -> Result<Callback, Error> {
    loop {
        let (mut stream, _) = listener.accept().await?;

        // Read the request head. We only need the request line (and a
        // bit of the headers, for browsers that send connection: close).
        // 4 KiB is more than enough for the loopback redirect.
        let mut buf = vec![0u8; 4096];
        let mut read = 0;
        loop {
            let n = stream.read(&mut buf[read..]).await?;
            if n == 0 {
                break;
            }
            read += n;
            if read >= 4 || buf[..read].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
            if read == buf.len() {
                break;
            }
        }
        let req_str = std::str::from_utf8(&buf[..read]).unwrap_or("");
        let path = match parse_request_path(req_str) {
            Some(p) => p,
            None => {
                respond_404(&mut stream).await.ok();
                continue;
            }
        };

        if !path.starts_with("/cb") {
            respond_404(&mut stream).await.ok();
            continue;
        }

        // Parse `?code=...&state=...&error=...`. We URL-encode an
        // absolute base so the relative path parses as a Url.
        let parsed = Url::parse("http://127.0.0.1")
            .and_then(|b| b.join(path))
            .ok();
        let Some(parsed) = parsed else {
            respond_html(&mut stream, 400, ERR_HTML).await.ok();
            return Err(Error::BadCallback);
        };

        let mut code: Option<String> = None;
        let mut state: Option<String> = None;
        let mut error: Option<String> = None;
        for (k, v) in parsed.query_pairs() {
            match k.as_ref() {
                "code" => code = Some(v.into_owned()),
                "state" => state = Some(v.into_owned()),
                "error" => error = Some(v.into_owned()),
                _ => {}
            }
        }

        if let Some(err) = error {
            respond_html(&mut stream, 200, DENIED_HTML).await.ok();
            return Err(if err == "access_denied" {
                Error::Denied
            } else {
                Error::AuthServerError(err)
            });
        }

        let Some(state) = state else {
            respond_html(&mut stream, 400, ERR_HTML).await.ok();
            return Err(Error::StateMismatch);
        };
        if !ct_eq(state.as_bytes(), expected_state.as_bytes()) {
            respond_html(&mut stream, 400, ERR_HTML).await.ok();
            return Err(Error::StateMismatch);
        }

        let Some(code) = code else {
            respond_html(&mut stream, 400, ERR_HTML).await.ok();
            return Err(Error::MissingCode);
        };

        respond_html(&mut stream, 200, OK_HTML).await.ok();
        return Ok(Callback { code });
    }
}

async fn exchange_code(
    server_url: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<AcquiredToken, Error> {
    let url = format!("{}/v1/oauth/token", server_url.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&serde_json::json!({
            "grant_type": "authorization_code",
            "code": code,
            "code_verifier": verifier,
            "redirect_uri": redirect_uri,
        }))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::AuthServerError(format!(
            "/v1/oauth/token returned {status}: {body}"
        )));
    }
    let token: AcquiredToken = resp.json().await?;
    Ok(token)
}

fn build_consent_url(
    consent_base: &str,
    req: &AcquireRequest,
    challenge: &str,
    redirect_uri: &str,
    state: &str,
) -> Result<Url, Error> {
    let mut url = Url::parse(&format!("{consent_base}/oauth/authorize"))?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code");
        q.append_pair("tenant_id", &req.tenant_id.to_string());
        q.append_pair("client_label", &req.client_label);
        q.append_pair("redirect_uri", redirect_uri);
        q.append_pair("code_challenge", challenge);
        q.append_pair("code_challenge_method", "S256");
        q.append_pair("scope", &req.scope.join(" "));
        q.append_pair("state", state);
        if let Some(m) = req.mode.as_deref() {
            q.append_pair("mode", m);
        }
        if let Some(ttl) = req.ttl_days {
            q.append_pair("ttl_days", &ttl.to_string());
        }
        if let Some(d) = req.max_docs {
            q.append_pair("max_docs", &d.to_string());
        }
        if let Some(b) = req.max_bytes {
            q.append_pair("max_bytes", &b.to_string());
        }
    }
    Ok(url)
}

/// RFC 7636 §4.1: 64 chars from the unreserved set [A-Za-z0-9-._~].
/// We sample uniformly from that 66-char set; output length is fixed
/// at 64 (well within the 43..=128 window the spec allows).
fn generate_verifier() -> String {
    const CHARSET: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::thread_rng();
    let mut out = String::with_capacity(64);
    let mut bytes = [0u8; 64];
    rng.fill_bytes(&mut bytes);
    for b in bytes {
        out.push(CHARSET[(b as usize) % CHARSET.len()] as char);
    }
    out
}

fn compute_challenge(verifier: &str) -> String {
    let mut h = Sha256::new();
    h.update(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(h.finalize())
}

/// 32 bytes of entropy, base64url-encoded — used as the `state` CSRF
/// parameter the consent UI echoes back to the loopback redirect.
fn generate_state() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Pulls the request URI out of the first line of an HTTP/1.x request
/// (`GET /cb?... HTTP/1.1\r\n`). Returns None if the head doesn't look
/// like a request line.
fn parse_request_path(head: &str) -> Option<&str> {
    let line = head.split("\r\n").next()?;
    let mut parts = line.split(' ');
    let _method = parts.next()?;
    let path = parts.next()?;
    Some(path)
}

async fn respond_html(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    body: &str,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "OK",
    };
    let resp = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\
         \r\n{body}",
        len = body.len(),
    );
    stream.write_all(resp.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn respond_404(stream: &mut tokio::net::TcpStream) -> std::io::Result<()> {
    respond_html(stream, 404, "<h1>Not found</h1>").await
}

const OK_HTML: &str = r#"<!doctype html>
<html><head><title>Authorized</title>
<style>body{font:14px system-ui;margin:48px;color:#222}
.card{max-width:480px;border:1px solid #e5e5e5;border-radius:8px;padding:24px}
h1{margin:0 0 8px;font-size:18px}p{color:#555}</style></head>
<body><div class="card"><h1>Authorization complete</h1>
<p>You can close this tab and return to the terminal.</p></div></body></html>"#;

const DENIED_HTML: &str = r#"<!doctype html>
<html><head><title>Denied</title>
<style>body{font:14px system-ui;margin:48px;color:#222}
.card{max-width:480px;border:1px solid #e5e5e5;border-radius:8px;padding:24px}
h1{margin:0 0 8px;font-size:18px;color:#b00}p{color:#555}</style></head>
<body><div class="card"><h1>Authorization denied</h1>
<p>No token was issued. You can close this tab.</p></div></body></html>"#;

const ERR_HTML: &str = r#"<!doctype html>
<html><head><title>Error</title>
<style>body{font:14px system-ui;margin:48px;color:#222}
.card{max-width:480px;border:1px solid #e5e5e5;border-radius:8px;padding:24px}
h1{margin:0 0 8px;font-size:18px;color:#b00}p{color:#555}</style></head>
<body><div class="card"><h1>Could not complete authorization</h1>
<p>The redirect was malformed. See the terminal for details.</p></div></body></html>"#;

/// Cross-platform browser opener. We shell out to the standard
/// per-OS helper rather than pulling in a dedicated crate; that's
/// one fewer dependency and the helpers all do the right thing for
/// a regular `https://` URL.
fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let cmd = ("open", vec![url]);
    #[cfg(target_os = "windows")]
    let cmd = ("cmd", vec!["/C", "start", "", url]);
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = ("xdg-open", vec![url]);

    std::process::Command::new(cmd.0)
        .args(&cmd.1)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not invoke {}: {e}", cmd.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_in_unreserved_set() {
        let v = generate_verifier();
        assert_eq!(v.len(), 64);
        for c in v.chars() {
            assert!(
                c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~'),
                "verifier char {c:?} outside unreserved set"
            );
        }
    }

    #[test]
    fn challenge_matches_server_check() {
        // Mirror of crates/orchext-server/src/oauth.rs::pkce_matches —
        // round-trip that verifier → challenge produces what the
        // server expects.
        let v = "abcDEF123-._~xyzABCDEFGHIJKLMNOPQRSTUVWXYZ012345";
        let c = compute_challenge(v);
        let mut h = Sha256::new();
        h.update(v.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(h.finalize());
        assert_eq!(c, expected);
    }

    #[test]
    fn parses_request_line() {
        let head = "GET /cb?code=oac_x&state=abc HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(parse_request_path(head), Some("/cb?code=oac_x&state=abc"));
    }

    #[test]
    fn rejects_garbage_request_line() {
        assert_eq!(parse_request_path(""), None);
        assert_eq!(parse_request_path("GET\r\n"), None);
    }

    #[tokio::test]
    async fn loopback_accepts_callback_with_matching_state() {
        // End-to-end at the TCP layer: bind a listener, drive a fake
        // browser GET, assert the callback is parsed cleanly. Mirrors
        // exactly what the real flow does between "browser navigates
        // to loopback" and "exchange code at /v1/oauth/token."
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpStream;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = "test-state-token";

        let browser = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let mut s = TcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .unwrap();
            let req = format!(
                "GET /cb?code=oac_yes&state=test-state-token HTTP/1.1\r\n\
                 Host: 127.0.0.1\r\nConnection: close\r\n\r\n"
            );
            s.write_all(req.as_bytes()).await.unwrap();
            let mut buf = vec![0u8; 1024];
            let _ = s.read(&mut buf).await;
        });

        let cb = accept_callback(&listener, state).await.unwrap();
        assert_eq!(cb.code, "oac_yes");
        browser.await.unwrap();
    }

    #[tokio::test]
    async fn loopback_rejects_state_mismatch() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpStream;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let browser = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let mut s = TcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .unwrap();
            let req = "GET /cb?code=oac_x&state=WRONG HTTP/1.1\r\n\
                       Host: 127.0.0.1\r\nConnection: close\r\n\r\n";
            s.write_all(req.as_bytes()).await.unwrap();
            let mut buf = vec![0u8; 1024];
            let _ = s.read(&mut buf).await;
        });

        let err = accept_callback(&listener, "expected").await.unwrap_err();
        assert!(matches!(err, Error::StateMismatch));
        browser.await.unwrap();
    }

    #[tokio::test]
    async fn loopback_surfaces_user_denial() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpStream;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let browser = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let mut s = TcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .unwrap();
            let req = "GET /cb?error=access_denied&state=ignored HTTP/1.1\r\n\
                       Host: 127.0.0.1\r\nConnection: close\r\n\r\n";
            s.write_all(req.as_bytes()).await.unwrap();
            let mut buf = vec![0u8; 1024];
            let _ = s.read(&mut buf).await;
        });

        let err = accept_callback(&listener, "ignored").await.unwrap_err();
        assert!(matches!(err, Error::Denied));
        browser.await.unwrap();
    }

    #[tokio::test]
    async fn loopback_ignores_favicon_then_accepts() {
        // Browsers sometimes prefetch /favicon.ico before the redirect
        // arrives. The accept loop must reply 404 and keep waiting for
        // the real /cb request.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpStream;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let browser = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let mut s = TcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .unwrap();
            s.write_all(
                b"GET /favicon.ico HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
            let mut buf = vec![0u8; 1024];
            let _ = s.read(&mut buf).await;
            drop(s);

            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let mut s = TcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .unwrap();
            s.write_all(
                b"GET /cb?code=oac_real&state=s HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
            let _ = s.read(&mut buf).await;
        });

        let cb = accept_callback(&listener, "s").await.unwrap();
        assert_eq!(cb.code, "oac_real");
        browser.await.unwrap();
    }

    #[test]
    fn consent_url_has_required_params() {
        let req = AcquireRequest {
            server_url: "https://api.example.com".into(),
            consent_base: Some("https://app.example.com".into()),
            tenant_id: Uuid::nil(),
            client_label: "Test agent".into(),
            scope: vec!["work".into(), "public".into()],
            mode: Some("read".into()),
            ttl_days: Some(30),
            max_docs: Some(50),
            max_bytes: None,
            timeout: None,
        };
        let url = build_consent_url(
            "https://app.example.com",
            &req,
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrst123",
            "http://127.0.0.1:5555/cb",
            "state-token",
        )
        .unwrap();
        let pairs: std::collections::HashMap<_, _> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(pairs.get("response_type").map(String::as_str), Some("code"));
        assert_eq!(
            pairs.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(pairs.get("scope").map(String::as_str), Some("work public"));
        assert_eq!(pairs.get("ttl_days").map(String::as_str), Some("30"));
        assert!(pairs.get("max_bytes").is_none());
    }
}
