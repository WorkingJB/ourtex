# Phase 2b.2 — Remote vault + sync client (shipped)

Shipped 2026-04-19. Extends the 2b.1 server with tenant-scoped vault,
index, token, and audit endpoints; introduces the `orchext-sync`
client crate; desktop gains remote workspaces. Still plaintext at
rest (encryption retrofits in 2b.3). Forward-looking plan context in
[`phase-2-plan.md`](phase-2-plan.md); live status in
[`../implementation-status.md`](../implementation-status.md).

---

### `orchext-server` — 2026-04-19 (Phase 2b.2 delta)
*([Notion: tenant-scoped vault + index endpoints](https://www.notion.so/34b47fdae49a8007b10ecec54458f25e))*

Extends the 2b.1 surface with vault + index + tokens + audit endpoints
scoped to a tenant, plus a `/v1/tenants` membership listing. Still
plaintext at rest; encryption is 2b.3.

**New routes (all require `Authorization: Bearer <session>`; all
tenant-scoped routes additionally require membership):**

| Method | Path                                         | Purpose                                      |
| ---    | ---                                          | ---                                          |
| GET    | `/v1/tenants`                                | Memberships for the logged-in account        |
| GET    | `/v1/t/:tid/vault/docs`                      | List documents (`?type=…`)                   |
| GET    | `/v1/t/:tid/vault/doc-count`                 | Cheap count for the desktop header           |
| GET    | `/v1/t/:tid/vault/docs/:id`                  | Canonical source + metadata                  |
| PUT    | `/v1/t/:tid/vault/docs/:id`                  | Upsert; optional `base_version` precondition |
| DELETE | `/v1/t/:tid/vault/docs/:id`                  | Remove; optional `?base_version=…`           |
| GET    | `/v1/t/:tid/index/search`                    | FTS over documents + filters                 |
| GET    | `/v1/t/:tid/index/list`                      | Filtered listing (no bodies)                 |
| GET    | `/v1/t/:tid/index/graph`                     | `{ nodes, edges }` with orphan edges filtered |
| GET    | `/v1/t/:tid/index/backlinks/:id`             | Backlinks to a doc                           |
| GET    | `/v1/t/:tid/index/outbound/:id`              | Outbound links from a doc                    |
| GET    | `/v1/t/:tid/tokens`                          | List MCP tokens                              |
| POST   | `/v1/t/:tid/tokens`                          | Issue an MCP token (secret shown once)       |
| DELETE | `/v1/t/:tid/tokens/:token_id`                | Revoke an MCP token                          |
| GET    | `/v1/t/:tid/audit`                           | Paginated per-tenant audit chain             |

**New schema (`migrations/0002_vault.sql`):** `documents` +
`doc_tags` + `doc_links` (with a generated `tsvector` column + GIN
index for FTS); `audit_entries` (per-tenant hash-chained log, shape
identical to `orchext-audit`'s JSONL wire format); `mcp_tokens` (mirrors
`orchext-auth::StoredToken` — `ocx_*` secret, Argon2id-hashed at rest,
scope + mode + limits).

**New server modules:**

- `tenants.rs` — `/v1/tenants` endpoint + `tenant_auth` middleware
  that extracts `:tid`, joins `memberships`, attaches a
  `TenantContext` to the request. A non-member hitting a tenant URL
  gets `404 not_found` — enumeration-safe against tenant-id probing.
- `documents.rs` — vault CRUD. Wire format is the canonical orchext-vault
  source (YAML frontmatter + markdown body) as a single `source` field,
  so the version hash computed server-side matches bit-for-bit whatever
  the client computes locally.
- `idx.rs` — search / list / graph / backlinks. FTS via
  `websearch_to_tsquery` against a stored `tsvector` column;
  `ts_rank_cd(...)::float8` cast so the real-typed rank deserializes
  into Rust's `f64` (the default sqlx mapping of `real` is `f32`).
- `tokens.rs` — per-tenant MCP tokens. Revoke-idempotent-or-404: the
  second revoke of a token returns `404 not_found` since the
  `UPDATE ... WHERE revoked_at IS NULL` clause matches zero rows.
- `audit.rs` — `append(tx, tenant_id, record)` called from inside
  the writer's transaction so a rolled-back mutation cannot leave
  an "it happened" entry. Hash input struct is field-for-field
  identical to `orchext-audit`'s, so a future export-to-JSONL job
  emits records that `orchext_audit::verify` accepts unchanged.

**Decisions recorded here:**

- **Wire format for documents is canonical source, not a DTO.**
  Sending the exact YAML+markdown bytes that `orchext-vault::Document`
  produces on disk keeps the content hash identical whether computed
  client-side or server-side. The frontmatter is *also* stored as
  JSONB in the `documents` table for structured queries (type_,
  visibility filters, `updated` date lookups in the FTS query), but
  the wire + hash authoritative representation is the YAML text.
- **Reads are audited; denied reads aren't a thing at this layer.**
  The tenant guard runs first, so a non-member never reaches a
  document handler. Inside the tenant, a logged-in user has full
  access to every document — per-document scope enforcement is an
  MCP-token concern (tokens have scopes; user sessions don't yet,
  and `private` hard-floor enforcement at the HTTP layer lands when
  the web client does in 2b.4). Session reads append one
  `vault.read` audit entry per call with outcome `ok`.
- **Audit append takes the table lock per tenant, not globally.**
  `SELECT ... ORDER BY seq DESC LIMIT 1 FOR UPDATE` serializes
  concurrent appends for the same tenant (next_seq + prev_hash race
  is impossible). Other tenants are unaffected; at v1 scale the
  per-tenant contention is negligible.
- **Version precondition via `base_version` field, not `If-Match`.**
  Keeps the wire shape JSON-only; no header plumbing through the
  reqwest layer. A future MCP HTTP transport may add header support
  for the HTTP-agent-RPC flavor.
- **`sqlx::query_as` at runtime, not the `query!` macro** — matches
  2b.1's decision. Migrate once CI has Postgres + `cargo sqlx
  prepare`.
- **Orphan edges filtered at the server.** `/index/graph` only emits
  edges whose target is also a document in the tenant, so the
  desktop graph view renders without dangling nodes. Same behavior
  the local view already applied client-side.
- **Tenant-scope isolation is path-based.** Every tenant-scoped
  query includes `WHERE tenant_id = $1` — no cross-tenant joins
  anywhere. A future row-level security policy would add defense in
  depth; for v1 the query discipline is sufficient.

**Integration tests (`tests/vault_flow.rs`):**

- `vault_write_read_roundtrip` — PUT + GET of the same doc;
  canonical source round-trips through `orchext-vault::Document::parse`.
- `vault_version_conflict` — second write with a wrong
  `base_version` returns `409` with `message = "version_conflict"`.
- `vault_cross_tenant_is_not_found` — user B hitting user A's
  tenant URL gets `404`, proving the tenant guard's enumeration
  resistance.
- `index_search_finds_content` — pins the private hard floor for
  search: a doc with `visibility: private` must not surface when
  `visibility=work,public` is passed; must surface when `private` is
  in the visibility list.
- `audit_chain_records_writes` — one PUT + one GET produces two
  chained audit entries (`vault.write` at seq 0, `vault.read` at
  seq 1 with `prev_hash == entry[0].hash`); `head_hash` in the
  response equals the last entry's hash.
- `tokens_issue_and_revoke` — issue returns the secret + public
  info; list includes the new token; revoke returns 204; re-revoke
  returns 404.

### `orchext-sync` — 2026-04-19 (new, Phase 2b.2)
*([Notion: remote vault sync client](https://www.notion.so/34b47fdae49a8054bd86c7de49c7dd7e))*

Client-side library that turns a running `orchext-server` into a
`VaultDriver` the existing local stack can use unchanged. Every caller
of the trait — `orchext-index::Index::reindex_from`, the desktop's
Tauri commands — works against a remote workspace without code
changes downstream.

**Public API:**

- `RemoteConfig { server_url, tenant_id, session_token }`
- `RemoteClient::new(config)` — shared `reqwest::Client` with bearer
  auth preset and a structured error translation layer
  (`unauthorized`, `not_found`, `conflict:version_conflict` promoted
  to typed `SyncError` variants; other tags pass through as
  `Server { status, tag, message }`).
- `RemoteVaultDriver::new(client)` — `#[async_trait] impl
  VaultDriver` over HTTP. Plus `write_versioned(id, doc, base_version)`
  / `delete_versioned(id, base_version)` for callers that want the
  version precondition.
- `login(server_url, &LoginInput)` / `list_tenants(server_url, secret)`
  — standalone helpers for the first-connection flow (the caller
  doesn't have a tenant_id yet at login time, so these don't go
  through `RemoteClient`).

**Decisions recorded here:**

- **No client-side cache layer inside `orchext-sync`.** Callers
  construct a local `orchext-index::Index` at a cache path and call
  `reindex_from(&remote_driver)` on open. Lists/searches then go
  through the local Index (SQLite + FTS5), writes go through the
  RemoteVaultDriver + mirror into the local Index. The alternative
  — adding a TTL cache on top of every `VaultDriver::list` call —
  would duplicate the Index's job and muddy the trait contract.
  Revisit if the "reindex once at open" assumption stops holding.
- **Synthetic `Entry.path` for remote listings.** `VaultDriver::list`
  returns `Entry { id, type_, path }`; the path is only used
  downstream by the FS watcher (local-only). Remote entries set
  `path = "remote://<type>/<id>.md"` which is never dereferenced but
  keeps the type contract.
- **Error collapsing through `VaultDriver`.** The trait's error enum
  (`VaultError`) is narrow — `NotFound`, `InvalidId`, etc. Network
  errors and server-tagged errors collapse to `NotFound` with the
  reason preserved in the message. Callers who need structured
  access use `write_versioned` / `delete_versioned` directly and
  get the full `SyncError` enum back.

**Deps added to workspace:** `reqwest` (0.12, rustls-tls),
`url` (2). Both already in the desktop crate; the workspace pin
lets `orchext-sync` share them.

### `orchext-desktop` — 2026-04-19 (Phase 2b.2 delta)
*([Notion: desktop remote workspace registration](https://www.notion.so/34d47fdae49a81718f80f6a184b3c3fc))*

Opens remote workspaces in the same UI shell as local ones. First-run
users still get "Add workspace…" for local; a new `workspace_connect_remote`
command handles the login → pick-tenant → register flow for remote.
No URL routing change: the existing `WorkspaceSwitcher` lists both
kinds.

**Registry changes (`workspaces.rs`):**

- `WorkspaceEntry` gains optional `server_url`, `tenant_id`,
  `account_email`, `session_token`, `session_expires_at`.
- `Registry::add_remote(name, cache_root, server_url, tenant_id,
  email, session_token, expires_at)` — dedupes on
  `(server_url, tenant_id)`; re-registration refreshes the stored
  session token. No duplicate workspaces across (url, tenant) pairs.

**State changes (`state.rs`):**

- `OpenVault::auth` / `audit` become `Option<Arc<...>>` — remote
  workspaces skip the local-only `TokenService` and `AuditWriter`
  (the server owns both for remote tenants).
- `open_workspace` dispatches on `entry.kind`:
  - `"local"` → existing `PlainFileDriver` + local Index + tokens
    + audit + (on activate) fs watcher.
  - `"remote"` → `RemoteVaultDriver` + local Index at
    `~/.orchext/remote/<workspace_id>/index.sqlite`, reindex_from the
    server at open. Auth/audit/watcher are `None`.
- `Services::require_local(feature)` — helper that gives a clear
  error for remote workspaces hitting a local-only command.

**Command changes (`commands.rs`):**

- New `workspace_connect_remote(server_url, email, password, name?,
  tenant_id?)` — logs in, lists tenants, picks the personal tenant
  (or the one the caller specified), persists the session token, and
  activates the resulting workspace. Cache root is
  `~/.orchext/remote/<workspace_id>/`.
- `token_*`, `audit_list`: call `require_local` and surface a clear
  "not yet wired through the server" error for remote. Full wiring
  (issue tokens via `POST /v1/t/:tid/tokens`, read audit via
  `GET /v1/t/:tid/audit`) is a 2b.2 follow-up.
- `doc_*`, `graph_snapshot`, `vault_info`: unchanged — they only
  touch `vault` + `index`, both of which work against either driver.
- `activate_inner` skips `watch::spawn` for remote workspaces.

**Known gaps after Phase 2b.2:**

- **No frontend "Connect to server" button yet.** The backend
  command is wired; the React UI still only shows "Add workspace…"
  (folder picker). Hooking up a modal that takes server URL +
  email + password and calls `workspace_connect_remote` is a
  5-minute follow-up — deliberately deferred from this session.
- **Token + audit UIs are local-only for remote workspaces.** The
  existing `TokensView` / `AuditView` show a "not supported on
  remote" message. Routing them through `/v1/t/:tid/tokens` and
  `/v1/t/:tid/audit` is a follow-up.
- **Session token stored in plaintext in `workspaces.json`.** Same
  threat model as the `.orchext/settings.json` Anthropic key; move
  to `tauri-plugin-stronghold` / OS keychain in 2b.3 with the
  unlock flow.
- **No periodic re-sync.** Reindex runs once on open; a concurrent
  edit from another client goes unseen until the next open. SSE /
  polling is the plan (deferred this session); polling is the
  cheaper first cut.
- **Onboarding + `orchext-mcp` stdio still local-only.** Running the
  local MCP server against a `RemoteVaultDriver` would let
  stdio-launched agents read remote context; punted so we can ship
  the HTTP endpoints first.
