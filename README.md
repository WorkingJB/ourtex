# Orchext

> API and AI documentation but for you — and your team.

Orchext is a local-first context store for the AI era. You keep a
directory of plain-markdown files describing who you are, what you're
working on, who you work with, and which tools you use; any AI agent
that speaks MCP can read from that directory with a scoped token, and
nothing writes without your review. An optional encrypted cloud tier
lets context follow you across devices and shares it with teammates
and organizations without handing the provider your keys.

## Why this exists

**Every AI product rebuilds a lossy profile of you from scratch.** You
introduce yourself to ChatGPT. You re-explain your project to Claude.
You teach Cursor your preferences again. Each tool stores a small
fragment, none of it leaves, and none of it follows you. Switching
models — let alone bringing a teammate up to speed — means starting
over.

**Memory features are a vendor moat.** "Provider memory" looks like a
user feature; it's a lock-in mechanism. The richer your context grows
inside one assistant, the higher the cost of trying another. That
gradient compounds in the vendor's favor, not yours.

**Your context shouldn't live in someone else's database.** Notes
about your manager, your goals, your team's roadmap — these are some
of the most sensitive bytes you produce. The current default is to
hand them to whichever AI happens to be the friction-free option this
quarter. That's a bad default.

Orchext inverts the model:

- **One vault you own, many agents you grant read access to.** Your
  context lives in plain markdown on your disk (and optionally,
  encrypted, on a server you control). Agents authenticate with
  short-lived, scoped, revocable tokens — never your full credentials,
  never blanket access.
- **Plain files, not a database.** Markdown + YAML frontmatter.
  Grep-able, diff-able, git-able, editable in any text editor. If you
  ever walk away from the desktop app, your data walks with you.
- **Read-only by default; writes are reviewed.** External agents can
  search, list, and read. Proposed writes land in a queue you approve
  before they touch the vault. A misbehaving agent can't quietly
  rewrite who you are.
- **Standard MCP, no protocol of our own.** Any AI client that speaks
  the Model Context Protocol can read your vault. We don't shape the
  responses for any particular model.
- **End-to-end encrypted on the server.** When you opt into the cloud
  tier for sync or sharing, the server holds ciphertext only. Keys
  derive from a passphrase that never leaves your device. Even an
  operator with full DB access sees nothing.

## Common use cases

- **Personal context for AI coding / writing assistants.** Keep your
  preferences, current projects, and tool inventory as markdown in
  `~/Orchext/`. Point Claude Desktop, Cursor, or any MCP-speaking
  client at the local MCP server (stdio) or a remote orchext-server
  over HTTP. The agent reads scoped context on demand; you review and
  accept any proposed writes.
- **Shared team context.** A tenant workspace on the cloud tier
  holds goals, decisions, and conventions the whole team should
  share. Members connect their desktop or web client with their own
  credentials; the server only ever sees encrypted blobs.
- **Portable identity across AI providers.** Orchext vaults are
  directories of markdown. Switching from one assistant to another
  means pointing the new one at the same vault — not re-teaching it
  who you are.
- **Agent sandboxing.** External agents get read-only MCP access
  scoped to a subset of your visibility labels (`work`, `public`,
  `personal`, `private`). The `private` floor is a hard label — only
  tokens that explicitly include `private` can read documents marked
  private. Writes are mediated by an approval queue.

## Repo layout

```
apps/
  desktop/             Tauri app (React UI + Rust core)
  web/                 React web client against orchext-server
crates/
  orchext-vault/        VaultDriver trait + PlainFileDriver
  orchext-index/        SQLite + FTS5 search / graph index (local)
  orchext-auth/         Scoped opaque-token auth (ocx_* secrets)
  orchext-audit/        Hash-chained append-only audit log
  orchext-crypto/       Argon2id KDF + XChaCha20-Poly1305 AEAD
  orchext-crypto-wasm/  wasm-bindgen surface for the browser
  orchext-mcp/          MCP server (JSON-RPC stdio transport)
  orchext-server/       Cloud control-plane (axum + Postgres) —
                        also hosts the MCP HTTP transport + OAuth
  orchext-sync/         Remote vault driver + crypto control calls
  orchext-oauth-client/ Agent-side PKCE helper + `orchext-oauth` CLI
docs/
  ARCHITECTURE.md      System design and key decisions
  FORMAT.md            Vault file format (markdown + frontmatter)
  MCP.md               MCP surface exposed to external agents
  implementation-status.md   Running build status
  phases/              Per-phase implementation plans
```

## Status

Orchext is in active build-out. The local stdio MCP server, the
desktop app (multi-vault + remote workspaces + browser unlock), the
cloud server (auth + vault + index + tokens + audit + crypto + OAuth
2.1 PKCE issuance + MCP HTTP transport), the web client, and an
agent-side OAuth CLI are all shipped. The remaining Phase 2b.5
slice is `context.propose` — read-only is the v1 default; writes go
through a review queue once that lands.

See [`docs/implementation-status.md`](docs/implementation-status.md)
for what's shipped, what's in flight, and which phase doc to open
next.

## Getting started

Prerequisites: Rust 1.75+, Node 20+, and (for the server crate)
Postgres reachable via `DATABASE_URL`.

```bash
# Build the workspace
cargo check --workspace

# Run the desktop app
cd apps/desktop
npm install
npm run tauri dev

# Run the web app (requires wasm-pack on PATH)
cd apps/web
npm install
npm run dev            # http://localhost:1430

# Run the cloud server locally (requires Postgres)
cd crates/orchext-server
cp .env.example .env   # edit DATABASE_URL + ORCHEXT_BIND
cargo run

# Acquire an MCP bearer for an external agent (PKCE OAuth)
cargo run -p orchext-oauth-client -- \
    --server https://your-orchext-server \
    --tenant <uuid-from-tenant-picker> \
    --label "Claude Code" \
    --scope "work public"
```

## License

Apache-2.0.
