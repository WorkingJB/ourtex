use orchext_server::{config::Config, router, AppState};
use sqlx::postgres::PgPoolOptions;
use std::io::{self, BufRead, Write};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("wipe") => {
            let skip_confirm = args.iter().any(|a| a == "--yes" || a == "-y");
            return run_wipe(skip_confirm).await;
        }
        Some(other) if other.starts_with('-') => {
            // Fall through to server startup; flags belong to that
            // path historically (none today, but reserved).
        }
        Some(other) => {
            eprintln!(
                "unknown subcommand: {other}\n\
                 \n\
                 usage:\n\
                 \torchext-server          run the HTTP server\n\
                 \torchext-server wipe     TRUNCATE all user-facing tables (destructive)\n\
                 \torchext-server wipe --yes  same, skip the typed confirmation"
            );
            std::process::exit(2);
        }
        None => {}
    }
    run_server().await
}

async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "orchext_server=info,axum=info,sqlx=warn".into()),
        )
        .compact()
        .init();

    let config = Config::from_env()?;
    tracing::info!(bind = %config.bind, "starting orchext-server");

    let db = PgPoolOptions::new()
        .max_connections(config.db_max_connections)
        .connect(&config.database_url)
        .await?;

    orchext_server::migrate(&db).await?;
    tracing::info!("migrations applied");

    let state = AppState::new(db)
        .with_secure_cookies(config.secure_cookies)
        .with_deployment_mode(config.deployment_mode);
    let mut app = router(state);
    if let Some(cors) = orchext_server::cors_layer(&config.cors_allow_origins) {
        tracing::info!(
            origins = ?config.cors_allow_origins,
            "CORS enabled for explicit origins"
        );
        app = app.layer(cors);
    }

    // Outermost layer: every request is wrapped in a `request` span
    // carrying a fresh UUID. Any `tracing::info!`/`warn!`/etc. inside
    // a handler inherits the span's fields, so a single request's log
    // lines are greppable by `id=…` in Fly's log feed. The id is
    // generated server-side rather than honoured from `X-Request-Id`
    // so callers can't pollute or impersonate request IDs in our logs.
    app = app.layer(
        TraceLayer::new_for_http().make_span_with(|req: &axum::http::Request<_>| {
            tracing::info_span!(
                "request",
                id = %uuid::Uuid::new_v4(),
                method = %req.method(),
                uri = %req.uri(),
            )
        }),
    );

    let addr: SocketAddr = config.bind.parse()?;
    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "listening");
    // `into_make_service_with_connect_info::<SocketAddr>` attaches
    // axum's `ConnectInfo` extension to every request, which the
    // `SmartIpKeyExtractor` in the auth-rate-limit layer falls back to
    // when no proxy headers are set. Without this, direct (non-Fly)
    // deploys would 500 on signup/login.
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// `orchext-server wipe` — TRUNCATE all user-facing tables. Used between
/// testing rounds to clear stale state (accounts that pre-date a schema
/// change, half-broken signups, etc.). Cascades through every dependent
/// table thanks to ON DELETE CASCADE on the FKs:
///
///   accounts → sessions, memberships, mcp_tokens, pending_signups,
///              oauth_authorization_codes
///   tenants  → organizations, documents (→ doc_links, doc_tags),
///              audit_entries, proposals, mcp_tokens,
///              oauth_authorization_codes
///
/// Reads `DATABASE_URL` from the environment (same as the server). By
/// default, prompts the operator to retype the database name to confirm —
/// fat-fingering test instead of prod, etc. `--yes` skips the prompt for
/// scripted use.
async fn run_wipe(skip_confirm: bool) -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn".into()),
        )
        .compact()
        .init();

    let config = Config::from_env()?;
    let target = redact_password(&config.database_url);
    let db_name = db_name_from_url(&config.database_url);

    eprintln!("Wipe target: {target}");
    if let Some(name) = db_name.as_deref() {
        eprintln!("Database name: {name}");
    }

    let db = PgPoolOptions::new()
        .max_connections(2)
        .connect(&config.database_url)
        .await?;

    // Pre-flight row counts so the operator sees the blast radius.
    // `accounts` and `tenants` are the cascade roots — every other row
    // is reachable from one of them. The `organizations` count is
    // best-effort: a partially-migrated DB (no Slice 1 migration yet)
    // still has accounts + tenants but not organizations, and we
    // shouldn't refuse the wipe over a missing optional table.
    let core_counts: Result<(i64, i64), sqlx::Error> = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM accounts), (SELECT COUNT(*) FROM tenants)",
    )
    .fetch_one(&db)
    .await;
    let (accts, tens) = match core_counts {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!(
                "Could not read row counts from accounts/tenants ({e}). \
                 The DB looks uninitialized — run the server once first to \
                 apply migrations, then retry the wipe."
            );
            return Err(Box::new(e));
        }
    };
    let orgs: Option<i64> = sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM organizations")
        .fetch_one(&db)
        .await
        .ok()
        .map(|(c,)| c);
    match orgs {
        Some(c) => eprintln!(
            "Will TRUNCATE: {accts} account(s), {tens} tenant(s), {c} org(s) \
             (cascading to all dependents)."
        ),
        None => eprintln!(
            "Will TRUNCATE: {accts} account(s), {tens} tenant(s) \
             (organizations table absent — pre-Slice-1 schema; cascading to all dependents)."
        ),
    }

    if !skip_confirm {
        match db_name {
            Some(expected) => {
                eprint!("\nType the database name ({expected}) to confirm: ");
                io::stderr().flush().ok();
                let mut line = String::new();
                io::stdin().lock().read_line(&mut line)?;
                if line.trim() != expected {
                    eprintln!("Confirmation didn't match. Aborting.");
                    return Ok(());
                }
            }
            None => {
                // Couldn't parse a db name — fall back to typing WIPE.
                eprint!("\nType WIPE to confirm: ");
                io::stderr().flush().ok();
                let mut line = String::new();
                io::stdin().lock().read_line(&mut line)?;
                if line.trim() != "WIPE" {
                    eprintln!("Confirmation didn't match. Aborting.");
                    return Ok(());
                }
            }
        }
    }

    sqlx::query("TRUNCATE TABLE accounts, tenants RESTART IDENTITY CASCADE")
        .execute(&db)
        .await?;

    eprintln!("Done. All user-facing tables truncated.");
    Ok(())
}

/// Replace `:password@` with `:***@` so the printed target doesn't
/// leak credentials in logs or scrollback.
fn redact_password(url: &str) -> String {
    // Find `://`, then the next `@` between scheme end and the next
    // `/`. Replace anything between `:` and `@` in that span with `***`.
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let after_scheme = scheme_end + 3;
    let rest = &url[after_scheme..];
    let Some(at) = rest.find('@') else {
        return url.to_string();
    };
    let userinfo = &rest[..at];
    let Some(colon) = userinfo.find(':') else {
        return url.to_string();
    };
    let mut out = String::with_capacity(url.len());
    out.push_str(&url[..after_scheme + colon + 1]);
    out.push_str("***");
    out.push_str(&rest[at..]);
    out
}

/// Pull the database name out of a `postgres://user:pass@host:port/dbname?params`
/// string. Best-effort string slicing — returns `None` if the shape doesn't
/// match.
fn db_name_from_url(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://")?.1;
    let after_authority = after_scheme.split_once('/')?.1;
    let path = after_authority.split('?').next()?;
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_password_from_postgres_url() {
        assert_eq!(
            redact_password("postgres://user:secret@host:5432/db"),
            "postgres://user:***@host:5432/db"
        );
    }

    #[test]
    fn redact_passes_through_when_no_userinfo() {
        // No password to redact — return unchanged rather than corrupt
        // the URL.
        assert_eq!(
            redact_password("postgres://host:5432/db"),
            "postgres://host:5432/db"
        );
    }

    #[test]
    fn extracts_db_name_from_postgres_url() {
        assert_eq!(
            db_name_from_url("postgres://user:pw@host:5432/orchext_test").as_deref(),
            Some("orchext_test")
        );
        assert_eq!(
            db_name_from_url("postgres://user:pw@host:5432/db?sslmode=require").as_deref(),
            Some("db")
        );
        assert_eq!(db_name_from_url("postgres://user:pw@host:5432"), None);
        assert_eq!(db_name_from_url("not a url"), None);
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install ctrl+c handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
