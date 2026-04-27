#![forbid(unsafe_code)]

use chrono::Duration;
use orchext_audit::AuditWriter;
use orchext_auth::{IssueRequest, Mode, Scope, TokenService};
use orchext_index::Index;
use orchext_mcp::rpc::{Id, Notification, Request, Response};
use orchext_mcp::{McpError, Server};
use orchext_vault::{PlainFileDriver, VaultDriver};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

// Seed type directories per `FORMAT.md` + `reconciled-v1-plan.md §D1`.
const SEED_TYPES: &[&str] = &[
    "identity",
    "roles",
    "goals",
    "relationships",
    "memories",
    "tools",
    "preferences",
    "domains",
    "decisions",
    "attachments",
];

enum Cmd {
    Serve { token: String, vault: PathBuf },
    Init(InitArgs),
    Help,
}

struct InitArgs {
    vault: PathBuf,
    label: String,
    scope: Vec<String>,
    ttl_days: Option<i64>,
}

fn parse_args() -> std::result::Result<Cmd, String> {
    let mut iter = std::env::args().skip(1);
    let sub = iter.next().unwrap_or_default();
    match sub.as_str() {
        "serve" => {
            let mut token: Option<String> = None;
            let mut vault: Option<PathBuf> = None;
            while let Some(arg) = iter.next() {
                match arg.as_str() {
                    "--token" => token = Some(iter.next().ok_or("--token requires a value")?),
                    "--vault" => {
                        vault = Some(PathBuf::from(iter.next().ok_or("--vault requires a value")?))
                    }
                    "-h" | "--help" => return Ok(Cmd::Help),
                    other => return Err(format!("unknown argument to serve: {other}")),
                }
            }
            Ok(Cmd::Serve {
                token: token.ok_or("serve requires --token")?,
                vault: vault.ok_or("serve requires --vault")?,
            })
        }
        "init" => {
            let mut vault: Option<PathBuf> = None;
            let mut label: String = "default".into();
            let mut scope: Vec<String> = vec!["work".into(), "public".into()];
            let mut ttl_days: Option<i64> = None;
            while let Some(arg) = iter.next() {
                match arg.as_str() {
                    "--vault" => {
                        vault = Some(PathBuf::from(iter.next().ok_or("--vault requires a value")?))
                    }
                    "--label" => label = iter.next().ok_or("--label requires a value")?,
                    "--scope" => {
                        scope = iter
                            .next()
                            .ok_or("--scope requires a value")?
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                    }
                    "--ttl-days" => {
                        ttl_days = Some(
                            iter.next()
                                .ok_or("--ttl-days requires a value")?
                                .parse()
                                .map_err(|e| format!("--ttl-days must be an integer: {e}"))?,
                        );
                    }
                    "-h" | "--help" => return Ok(Cmd::Help),
                    other => return Err(format!("unknown argument to init: {other}")),
                }
            }
            Ok(Cmd::Init(InitArgs {
                vault: vault.ok_or("init requires --vault")?,
                label,
                scope,
                ttl_days,
            }))
        }
        "-h" | "--help" | "" => Ok(Cmd::Help),
        other => Err(format!("unknown subcommand: {other}")),
    }
}

fn help_text() -> &'static str {
    "Usage:\n\
     \n\
     orchext-mcp serve --token <TOKEN> --vault <VAULT_DIR>\n\
         Run the JSON-RPC MCP server on stdio.\n\
     \n\
     orchext-mcp init --vault <VAULT_DIR> \\\n\
                    [--label <LABEL>] [--scope work,public] [--ttl-days 90]\n\
         Create the vault skeleton, issue an initial token, and print a\n\
         Claude Desktop mcpServers config entry. The token secret is\n\
         shown once and never again — save it.\n"
}

#[tokio::main]
async fn main() -> ExitCode {
    match parse_args() {
        Ok(Cmd::Help) => {
            println!("{}", help_text());
            ExitCode::SUCCESS
        }
        Ok(Cmd::Serve { token, vault }) => match run_serve(token, vault).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("orchext-mcp: {e}");
                ExitCode::from(1)
            }
        },
        Ok(Cmd::Init(args)) => match run_init(args).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("orchext-mcp: {e}");
                ExitCode::from(1)
            }
        },
        Err(e) => {
            eprintln!("orchext-mcp: {e}\n\n{}", help_text());
            ExitCode::from(2)
        }
    }
}

// ---------------- serve ----------------

async fn run_serve(token_secret: String, vault_root: PathBuf) -> std::result::Result<(), String> {
    // Canonicalize so the fs watcher sees the same absolute path the OS
    // uses when firing events — on macOS `/tmp/...` is a symlink to
    // `/private/tmp/...` and fsevent will report under the latter, which
    // means event paths would fail to match against a `/tmp/...` root.
    let vault_root = vault_root
        .canonicalize()
        .map_err(|e| format!("vault path {}: {e}", vault_root.display()))?;
    let vault: Arc<dyn VaultDriver> = Arc::new(PlainFileDriver::new(vault_root.clone()));
    let orchext_dir = vault_root.join(".orchext");
    let index = Arc::new(
        Index::open(orchext_dir.join("index.sqlite"))
            .await
            .map_err(|e| format!("open index: {e}"))?,
    );
    let auth = Arc::new(
        TokenService::open(orchext_dir.join("tokens.json"))
            .await
            .map_err(|e| format!("open token service: {e}"))?,
    );
    let audit = Arc::new(
        AuditWriter::open(orchext_dir.join("audit.jsonl"))
            .await
            .map_err(|e| format!("open audit log: {e}"))?,
    );
    let token = auth
        .authenticate(&token_secret)
        .await
        .map_err(|e| format!("authenticate: {e}"))?;

    // Catch up the index with whatever already exists on disk. The watcher
    // only fires on *changes* after it starts, so any docs written before
    // the server boots would otherwise be invisible to search/list until
    // touched. `reindex_from` is idempotent — it clears + rebuilds.
    index
        .reindex_from(&*vault)
        .await
        .map_err(|e| format!("reindex: {e}"))?;

    let (note_tx, note_rx) = tokio::sync::mpsc::unbounded_channel::<Notification>();
    let proposals_dir = orchext_dir.join("proposals");
    tokio::fs::create_dir_all(&proposals_dir)
        .await
        .map_err(|e| format!("create proposals dir: {e}"))?;
    let server = Arc::new(
        Server::new(vault.clone(), index.clone(), auth, audit, token)
            .with_notifier(note_tx)
            .with_proposals_dir(proposals_dir),
    );

    // Watcher holds notify's RecommendedWatcher alive via WatcherHandle.
    // Dropping `_watch` stops the watcher; we keep it scoped to this
    // function so it lives for the duration of the serve loop.
    let _watch = orchext_mcp::watch::spawn(vault_root.clone(), server.clone())
        .map_err(|e| format!("start fs watcher: {e}"))?;

    serve_stdio(server, note_rx).await.map_err(|e| e.to_string())
}

async fn serve_stdio(
    server: Arc<Server>,
    mut notifications: tokio::sync::mpsc::UnboundedReceiver<Notification>,
) -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin).lines();
    let mut writer = stdout;

    let mut stdin_open = true;
    loop {
        if stdin_open {
            tokio::select! {
                line = reader.next_line() => {
                    match line? {
                        None => stdin_open = false,
                        Some(line) if line.trim().is_empty() => continue,
                        Some(line) => handle_line(&server, &mut writer, &line).await?,
                    }
                }
                note = notifications.recv() => {
                    let Some(note) = note else { break };
                    write_message(&mut writer, &note).await?;
                }
            }
        } else {
            // Stdin EOF'd: drain remaining notifications with a short grace
            // so any in-flight fs-watcher event still reaches the client
            // that's disconnecting. This also prevents the server from
            // exiting mid-write on a race between EOF and a pending notify.
            match tokio::time::timeout(
                std::time::Duration::from_millis(250),
                notifications.recv(),
            )
            .await
            {
                Ok(Some(note)) => write_message(&mut writer, &note).await?,
                Ok(None) | Err(_) => break,
            }
        }
    }
    Ok(())
}

async fn handle_line<W: AsyncWriteExt + Unpin>(
    server: &Server,
    writer: &mut W,
    line: &str,
) -> io::Result<()> {
    let req: Request = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            let resp = Response::err(Id::Null, McpError::ParseError(e.to_string()).to_rpc());
            return write_message(writer, &resp).await;
        }
    };
    if let Some(resp) = server.handle(req).await {
        write_message(writer, &resp).await?;
    }
    Ok(())
}

async fn write_message<W: AsyncWriteExt + Unpin, T: serde::Serialize>(
    writer: &mut W,
    msg: &T,
) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(msg)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

// ---------------- init ----------------

async fn run_init(args: InitArgs) -> std::result::Result<(), String> {
    let vault = args.vault;
    tokio::fs::create_dir_all(&vault)
        .await
        .map_err(|e| format!("create vault dir: {e}"))?;
    for t in SEED_TYPES {
        tokio::fs::create_dir_all(vault.join(t))
            .await
            .map_err(|e| format!("create {t} dir: {e}"))?;
    }
    let orchext_dir = vault.join(".orchext");
    tokio::fs::create_dir_all(&orchext_dir)
        .await
        .map_err(|e| format!(".orchext dir: {e}"))?;

    let auth = TokenService::open(orchext_dir.join("tokens.json"))
        .await
        .map_err(|e| format!("open token service: {e}"))?;

    let scope = Scope::new(args.scope.iter().cloned()).map_err(|e| format!("scope: {e}"))?;
    let ttl = args.ttl_days.map(Duration::days);

    let issued = auth
        .issue(IssueRequest {
            label: args.label,
            scope,
            mode: Mode::Read,
            limits: Default::default(),
            ttl,
        })
        .await
        .map_err(|e| format!("issue: {e}"))?;

    // Pre-create the index + audit log so a first `serve` invocation
    // doesn't have to — catches permission issues at init time.
    let _ = Index::open(orchext_dir.join("index.sqlite"))
        .await
        .map_err(|e| format!("init index: {e}"))?;
    let _ = AuditWriter::open(orchext_dir.join("audit.jsonl"))
        .await
        .map_err(|e| format!("init audit: {e}"))?;

    let secret = issued.secret.expose();
    print_init_summary(&vault, secret, &issued.info);
    Ok(())
}

fn print_init_summary(vault: &Path, secret: &str, info: &orchext_auth::PublicTokenInfo) {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "orchext-mcp".into());
    let vault_str = vault.to_string_lossy();
    let scope_list = info.scope.join(", ");

    println!("Vault initialized at {vault_str}");
    println!();
    println!("Token ({id}, scope: {scope_list}, expires {expires}):", id = info.id, expires = info.expires_at.format("%Y-%m-%d"));
    println!("  {secret}");
    println!();
    println!("Save the secret now — it is shown once and cannot be recovered.");
    println!();
    println!("Launch the server:");
    println!("  {exe} serve --vault {vault_str} --token {secret}");
    println!();
    println!("Claude Desktop mcpServers entry (add to claude_desktop_config.json):");
    let config = serde_json::json!({
        "mcpServers": {
            "orchext": {
                "command": exe,
                "args": ["serve", "--vault", vault_str, "--token", secret]
            }
        }
    });
    println!("{}", serde_json::to_string_pretty(&config).unwrap());
}
