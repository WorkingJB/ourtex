//! `orchext-oauth` — CLI wrapper around `orchext_oauth_client::acquire_token`.
//!
//! Designed to be the canonical "acquire a token" helper for any
//! external integration that needs to talk to orchext-server's MCP
//! surface. Typical use:
//!
//! ```text
//! orchext-oauth \
//!     --server https://orchext.example.com \
//!     --tenant 8d4f7a40-... \
//!     --label "Claude Code" \
//!     --scope "work public"
//! ```
//!
//! Args are parsed manually (no `clap` dep) so the binary stays tiny.
//! Token output goes to stdout as JSON by default; `--bearer-only`
//! prints just the secret so the caller can pipe it.

#![forbid(unsafe_code)]

use orchext_oauth_client::{acquire_token, AcquireRequest, AcquiredToken, Error};
use std::process::ExitCode;
use std::time::Duration;
use uuid::Uuid;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return ExitCode::SUCCESS;
    }

    let parsed = match parse_args(&args) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("error: {msg}");
            eprintln!();
            print_help();
            return ExitCode::from(2);
        }
    };

    match acquire_token(parsed.req).await {
        Ok(t) => {
            print_token(&t, parsed.bearer_only);
            ExitCode::SUCCESS
        }
        Err(Error::Denied) => {
            eprintln!("authorization denied by user");
            ExitCode::from(3)
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

struct Parsed {
    req: AcquireRequest,
    bearer_only: bool,
}

fn parse_args(raw: &[String]) -> Result<Parsed, String> {
    let mut server: Option<String> = None;
    let mut consent: Option<String> = None;
    let mut tenant: Option<String> = None;
    let mut label: Option<String> = None;
    let mut scope: Option<String> = None;
    let mut mode: Option<String> = None;
    let mut ttl_days: Option<i64> = None;
    let mut max_docs: Option<i32> = None;
    let mut max_bytes: Option<i64> = None;
    let mut timeout_secs: Option<u64> = None;
    let mut bearer_only = false;

    let mut iter = raw.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--server" => server = Some(next(&mut iter, "--server")?),
            "--consent-base" => consent = Some(next(&mut iter, "--consent-base")?),
            "--tenant" => tenant = Some(next(&mut iter, "--tenant")?),
            "--label" => label = Some(next(&mut iter, "--label")?),
            "--scope" => scope = Some(next(&mut iter, "--scope")?),
            "--mode" => mode = Some(next(&mut iter, "--mode")?),
            "--ttl-days" => {
                ttl_days = Some(
                    next(&mut iter, "--ttl-days")?
                        .parse()
                        .map_err(|_| "--ttl-days must be an integer".to_string())?,
                )
            }
            "--max-docs" => {
                max_docs = Some(
                    next(&mut iter, "--max-docs")?
                        .parse()
                        .map_err(|_| "--max-docs must be an integer".to_string())?,
                )
            }
            "--max-bytes" => {
                max_bytes = Some(
                    next(&mut iter, "--max-bytes")?
                        .parse()
                        .map_err(|_| "--max-bytes must be an integer".to_string())?,
                )
            }
            "--timeout-secs" => {
                timeout_secs = Some(
                    next(&mut iter, "--timeout-secs")?
                        .parse()
                        .map_err(|_| "--timeout-secs must be an integer".to_string())?,
                )
            }
            "--bearer-only" => bearer_only = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let server = server.ok_or("--server is required".to_string())?;
    let tenant = tenant.ok_or("--tenant is required".to_string())?;
    let label = label.ok_or("--label is required".to_string())?;
    let scope = scope.ok_or("--scope is required".to_string())?;
    let tenant_id = Uuid::parse_str(&tenant).map_err(|_| "--tenant must be a UUID".to_string())?;
    let scope: Vec<String> = scope
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if scope.is_empty() {
        return Err("--scope must list at least one visibility label".into());
    }

    Ok(Parsed {
        req: AcquireRequest {
            server_url: server,
            consent_base: consent,
            tenant_id,
            client_label: label,
            scope,
            mode,
            ttl_days,
            max_docs,
            max_bytes,
            timeout: timeout_secs.map(Duration::from_secs),
        },
        bearer_only,
    })
}

fn next<'a>(
    iter: &mut std::slice::Iter<'a, String>,
    flag: &str,
) -> Result<String, String> {
    iter.next()
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn print_token(t: &AcquiredToken, bearer_only: bool) {
    if bearer_only {
        println!("{}", t.access_token);
        return;
    }
    let json = serde_json::to_string_pretty(t).unwrap_or_else(|_| t.access_token.clone());
    println!("{json}");
}

fn print_help() {
    eprintln!(
        r#"orchext-oauth — acquire an orchext-server agent token via OAuth 2.1 + PKCE.

Usage:
  orchext-oauth --server <URL> --tenant <UUID> --label <NAME> --scope <"a b c">
                [--mode read|read_propose] [--ttl-days N] [--max-docs N]
                [--max-bytes N] [--timeout-secs N] [--consent-base URL]
                [--bearer-only]

Required:
  --server URL           orchext-server API base, e.g. https://api.example.com
  --tenant UUID          tenant id the token will operate against
  --label NAME           display label saved to mcp_tokens.label
  --scope "a b c"        space-separated visibility labels

Optional:
  --mode MODE            "read" (default) or "read_propose"
  --ttl-days N           token TTL (server clamps to [1,365])
  --max-docs N           per-token retrieval doc cap
  --max-bytes N          per-token retrieval byte cap
  --timeout-secs N       browser-wait timeout (default 300)
  --consent-base URL     web app base if it differs from --server
  --bearer-only          print only the access_token (default: full JSON)
"#
    );
}
