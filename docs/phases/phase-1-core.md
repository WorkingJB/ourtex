# Phase 1 — Core v1 (shipped)

Shipped crate details for the initial v1 build: vault format, audit
log, tokens/auth, index, MCP server, and desktop app (including the
Phase 2a multi-vault follow-up, since that work landed inside
`orchext-desktop`). Each section is frozen at the date shown; live
status lives in [`../implementation-status.md`](../implementation-status.md).

---

### `orchext-vault` — 2026-04-18
*([Notion: vault format + storage driver](https://www.notion.so/34b47fdae49a8031b92bda39b62584a3))*

The vault format parser and storage driver abstraction.

**Public API:**

- `Document` — parse / serialize / version (SHA-256)
- `Frontmatter` — all seed fields + `extras` (BTreeMap) preserves unknown/x-* fields round-trip
- `DocumentId` — newtype validated per `FORMAT.md` §3.3
- `Visibility` — `Public | Work | Personal | Private | Custom(String)`; `is_private()` only true for the built-in `Private`
- `VaultDriver` — async trait: `list`, `read`, `write`, `delete`
- `PlainFileDriver` — disk-backed impl, skips `.orchext/` and dot-dirs
- `VaultError` — `thiserror` enum

**Notable tests:**

- Round-trip preserves `x-*` extensions (FORMAT.md §3.4 commitment)
- `private` hard floor: built-in `Private` reports `is_private()` true; `Custom("semi-private")` does not
- `PlainFileDriver` rejects `write(id, doc)` when `id` doesn't match `doc.frontmatter.id`
- `.orchext/` directory is skipped by `list()`

**Decisions recorded here:** none — matches spec.

### `orchext-audit` — 2026-04-18
*([Notion: append-only audit log](https://www.notion.so/34b47fdae49a80af81fdd485c4df22ad))*

Append-only, hash-chained JSONL audit log. Matches `ARCHITECTURE.md` §5.7 and `MCP.md` §9.

**Public API:**

- `AuditWriter::open(path)` — recovers chain state (seq, last hash) from existing file
- `AuditWriter::append(AuditRecord) -> AuditEntry` — atomic append (O_APPEND + flush), rotates state
- `verify(path) -> VerifyReport` — rehashes every entry, fails at the exact `seq` where the chain breaks
- `Iter` — stream entries from disk
- `Actor::{Owner, Token(String)}` — serializes as `"owner"` or `"tok:<id>"` (literal string, not JSON object)
- `Outcome::{Ok, Denied, Error}`

**Decisions recorded here:**

- **JSONL not SQLite.** Log file is newline-delimited JSON; chosen over a SQLite table for append simplicity, grep-ability, and so the log survives even if SQLite schemas drift. The indexer (below) is what uses SQLite.
- **Hash input is compact JSON of a fixed-field struct.** Deterministic because field order is declaration order in a struct (not a map).
- **Canonical hash excludes the `hash` field** of the entry itself (chicken-and-egg), but includes `prev_hash`, so tampering with any field is detected.

**Notable tests:**

- Reopen preserves chain: writer close + reopen + append continues at the right seq with the right `prev_hash`
- Tamper detection identifies the specific seq where the chain broke
- Empty log verifies cleanly (0 entries, no last seq/hash)

### `orchext-auth` — 2026-04-18
*([Notion: token service](https://www.notion.so/34b47fdae49a803b98f4eb9aed1e9e87))*

Token service: issuance, Argon2id hashing, scope eval including the `private` hard floor, revocation, expiry, retrieval limits.

**Public API:**

- `TokenService::open(path)` — loads `tokens.json` or starts empty
- `TokenService::issue(IssueRequest) -> IssuedToken` — returns secret + public info
- `TokenService::authenticate(&str) -> AuthenticatedToken` — constant-time-ish verify via Argon2id
- `TokenService::revoke(id)`, `mark_used(id, ts)`, `list()`
- `Scope` — `BTreeSet<String>` wrapper with `allows_label`, `allows(&Visibility)`, `includes_private`, `narrow_to(&[String])`
- `Mode::{Read, ReadPropose}`
- `Limits { max_docs: u32, max_bytes: u64 }` — default 20 docs / 64 KiB per `MCP.md` §3.1
- `TokenSecret` — Debug-redacted newtype (never prints the raw value)
- `IssueRequest`, `IssuedToken`, `AuthenticatedToken`, `PublicTokenInfo`

**Decisions recorded here:**

- **Secret format: `ocx_` + base64url-no-pad of 32 random bytes.** Matches `MCP.md` §3.1 intent; 43-char payload, url-safe for stdio copy-paste.
- **Token ID: `tok_` + base64url-no-pad of 12 random bytes.** Separate from the secret, goes in audit logs, never leaks secret bits.
- **Atomic persistence via write-temp + rename.** Prevents torn JSON files under crash.
- **`Scope::narrow_to` is intersection-only.** Can never widen — matches `MCP.md` §3.2.
- **Private hard-floor is enforced by construction.** `Scope::allows_label` is a literal-string match against the scope set; no implicit promotion anywhere. Tests cover: token without `"private"` can't read `Private` docs; custom `semi-private` label doesn't accidentally grant `Private` access.

**Notable tests:**

- Issue → authenticate roundtrip
- Wrong secret / malformed secret / revoked / expired all reject with distinct errors
- `PublicTokenInfo` serialization never emits the hash
- Persists across reopen (tokens file survives service drop)
- Private floor enforced both ways (denies without `private`, allows with `private`)

---

### `orchext-index` — 2026-04-18
*([Notion: SQLite + FTS5 indexer + graph queries](https://www.notion.so/34b47fdae49a8046909ce0aa7d968984))*

Full-text search + tag/type filter + link graph over the vault. Backed
by SQLite with FTS5.

**Public API:**

- `Index::open(path)` — opens or creates `index.sqlite` at the given path; applies schema idempotently
- `Index::reindex_from(&dyn VaultDriver) -> IndexStats` — full rebuild from a vault; the contract that makes `index.sqlite` safely deletable (FORMAT.md §7)
- `Index::upsert(type_, &Document)` — insert or replace a document plus its tags, links, and FTS row
- `Index::remove(&DocumentId)` — drops from all tables including FTS
- `Index::search(SearchQuery) -> Vec<SearchHit>` — FTS5 bm25-scored, filtered by type/tag/visibility/updated_since, with snippet
- `Index::list(ListFilter) -> Vec<ListItem>` — enumerate, same filters, no body
- `Index::backlinks(id)` / `outbound_links(id)` — graph queries

**Decisions recorded here:**

- **rusqlite with `bundled` feature.** No system SQLite dependency; binary is self-contained. FTS5 is compiled in.
- **Async wrapper via `tokio::task::spawn_blocking`.** rusqlite is synchronous; `Arc<Mutex<Connection>>` (std mutex, since we're in blocking context) serializes access within a process.
- **Contentful FTS5 table, not external-content.** Slight storage duplication (body is in both `documents` and `search`); huge simplicity win — no triggers, straightforward upsert.
- **`documents` + `tags` + `links` normalized.** `ON DELETE CASCADE` drops tags and links when a document is removed; FTS row is dropped explicitly.
- **Scope filtering is an `IN` clause on `visibility`.** Passing `allowed_visibility` is how callers apply the `private` hard floor: if `"private"` isn't in the set, no `private` documents surface. Consistent with how `orchext-auth` thinks about scope.
- **Title extraction is `# Heading` → first non-empty H1, fallback to `id`.** Matches MCP.md §5.1.
- **`WAL` journal mode enabled.** Better concurrency (the desktop UI might read while MCP writes), negligible cost.

**Notable tests:**

- `search_respects_scope_filter_and_private_floor`: proves a scope without `"private"` cannot surface `Visibility::Private` documents, even when the query matches the body.
- `remove_drops_from_all_tables_including_fts`: after remove, search misses, backlinks/outbound disappear, list excludes it.
- `upsert_replaces_tags_and_links`: re-upserting a document replaces (not unions) its tag and link sets.
- `reindex_from_vault_and_search`: reindex produces correct `IndexStats`, subsequent search returns hits.

### `orchext-mcp` — 2026-04-19
*([Notion: JSON-RPC 2.0 MCP server](https://www.notion.so/34b47fdae49a8091904cd4790ea31aad))*

JSON-RPC 2.0 MCP server over stdio. Wires the four backing services
(`vault`, `index`, `auth`, `audit`) behind the v1 surface defined by
`MCP.md`.

**Public API (library):**

- `Server::new(vault, index, auth, audit, token)` — one server per
  connection; `token` is an `AuthenticatedToken` already verified.
- `Server::handle(Request) -> Option<Response>` — dispatches one
  JSON-RPC message. Returns `None` for notifications.
- `McpError` / `McpError::to_rpc()` — the code/tag mapping from
  `MCP.md` §7 (`-32000..-32007`).
- `rpc::{Request, Response, Notification, RpcError, Id}` — wire
  envelope types.

**Binary:** `orchext-mcp --token <TOKEN> --vault <VAULT_DIR>`. Reads
line-delimited JSON from stdin, writes line-delimited JSON to stdout.

**Implemented methods:** `initialize`, `initialized` (notification),
`ping`, `tools/list`, `tools/call`, `resources/list`, `resources/read`,
`resources/subscribe`, `resources/unsubscribe`.

**Tools:** `context.search`, `context.get`, `context.list` under
the `context.` namespace (D3). Results include provenance
(`visibility`, `updated`, `source` when set).

**Decisions recorded here:**

- **Token pre-authenticated at startup.** `main.rs` calls
  `TokenService::authenticate` before reading a single byte of
  JSON-RPC input. An invalid token exits non-zero immediately;
  every JSON-RPC message after that is implicitly authorized as
  the pre-verified principal. This matches MCP.md §2.1 (stdio
  launch) where the token arrives via `--token` and is bound to
  the process lifetime.
- **Index is rebuilt from the vault on every `serve` start.**
  `reindex_from` is idempotent and cheap at v1 vault sizes. This
  guarantees the index matches disk at T0 — important because the
  fs watcher only fires on changes *after* it starts, so any docs
  added while the server was down would otherwise be invisible
  until touched.
- **Rate limit: 60 requests / 10-second sliding window per token.**
  Applies to `tools/*`, `resources/*`. `initialize`, `ping`, and
  notifications are exempt — the limiter protects the indexer
  and fs, not handshakes. When saturated returns `-32005 /
  rate_limited` with `error.data.retry_after_ms` set to the wait
  until the oldest in-window request ages out.
- **`not_authorized` is deliberately ambiguous.** Out-of-scope,
  nonexistent, and private-without-private-scope documents all
  return `-32002 / not_authorized` from `context.get` and
  `resources/read`. A test (`get_nonexistent_is_indistinguishable_from_out_of_scope`)
  pins this so it cannot regress.
- **Private hard floor is re-checked defensively in `context.get`.**
  The index layer already enforces it via `allowed_visibility`, but
  `get` reads from the vault (not the index) and re-checks
  `visibility.is_private() && !scope.includes_private()` so a
  future refactor of `Scope::allows` cannot silently widen access.
- **`scope` request argument narrows only, never widens.**
  `Scope::narrow_to` is intersection; a `scope: ["private"]`
  argument on a token without `"private"` errors out rather than
  granting access. Returned as `-32004 / invalid_argument`.
- **Provenance-only, no sanitization (D5).** Results carry the
  frontmatter `source` when set. The server does not scrub,
  relabel, or reinterpret body text. For search hits `source`
  costs one extra `vault.read` per hit — acceptable at the
  bounded limits (≤100 docs); re-evaluate if needed by promoting
  `source` into the index schema.
- **Retrieval limits enforced in order `hard cap → token cap →
  request`.** `limit` is clamped to 100 (hard), then to
  `token.limits.max_docs`, then to what the caller asked for.
  For search, a running `max_bytes` counter over snippet bytes
  can truncate early and set `truncated: true`. For `context.get`,
  `max_bytes` is not applied — a single-document fetch that the
  caller asked for by ID should not be silently truncated.
- **`resources/subscribe` emits updates via an fs watcher.** The
  `notify` crate watches the vault root recursively (fsevent backend
  on macOS; default elsewhere). On Create/Modify/Remove the watcher
  thread classifies the path as `(type, id)`, upserts or removes the
  doc from the index, then calls `Server::emit_resource_updated`
  which fires `notifications/resources/updated` if the URI matches
  a subscription (exact, type-prefix, or root). The vault root is
  canonicalized at startup so fsevent's absolute paths line up with
  the driver root.
- **Audit on every dispatched call.** Every
  `context.*` / `resources.read` call appends one JSONL entry
  with actor = `tok:<id>`, outcome `ok` or `denied`, and the
  scope in effect. `auth.mark_used` is touched on every attempt
  (including denials) so revoked tokens still leave a trail.
  Audit-write failure is logged via `tracing::warn` but never
  fails the caller — the user's read must succeed even if the
  audit sink is wedged.
- **`tools/call` returns both `content` (text) and
  `structuredContent` (typed JSON).** MCP clients that only look
  at `content` get the tool result as a stringified JSON block;
  strict clients read `structuredContent` directly without a
  second parse.
- **Tool input validation is hand-rolled (serde + explicit
  length checks).** No JSON-schema validator dep. `tools/list`
  still advertises schemas so agents can self-validate before
  calling.

**Notable tests:**

- `search_private_floor_requires_explicit_private`: a token
  without `private` cannot surface a private diary entry even
  when the query body matches; with `private` in scope, it does.
- `search_rejects_widening_scope_argument`: a `scope: ["private"]`
  request on a work-only token returns `-32004 / invalid_argument`,
  not a widened result set.
- `get_nonexistent_is_indistinguishable_from_out_of_scope`:
  both map to `-32002 / not_authorized` (enumeration defence).
- `resources_list_filters_by_scope`: resource listings omit
  URIs the token can't read; direct `resources/read` to those
  URIs returns `-32002`.
- `audit_log_grows_per_call`: both an ok `context.list` and a
  denied `context.get` append chained JSONL entries that
  `orchext_audit::verify` accepts.

**Binary subcommands:**

- `orchext-mcp init --vault <DIR> [--label <L>] [--scope work,public]
  [--ttl-days N]` — creates the vault skeleton (seed type dirs +
  `.orchext/`), issues an initial token, and prints (a) the token
  secret (shown once), (b) the launch command, (c) a
  ready-to-paste Claude Desktop `mcpServers` entry.
- `orchext-mcp serve --vault <DIR> --token <TOKEN>` — the JSON-RPC
  server itself. Reindexes at startup, spawns the fs watcher,
  then enters a `tokio::select!` loop over `(stdin lines,
  notification channel)`. On stdin EOF it drains any in-flight
  notifications for up to 250 ms before exiting, so an fs event
  racing a disconnect still reaches the client.

**Known gaps (not in v1 surface):**

- `context.propose` returns method-not-found; intentionally
  deferred to v1.1 per MCP.md §5.4 and reconciled-v1-plan D6 (it
  depends on the desktop review UI).
- FSEvents coalesces bursts; a single `echo >> file.md` can emit
  2–3 `notifications/resources/updated` for one logical write.
  Clients dedupe by URI; this is a minor politeness issue, not a
  correctness one. Debouncing would require `notify-debouncer-mini`
  and is deferred.

---

### `orchext-desktop` — 2026-04-19
*(Notion: [Desktop Tauri app + React UI](https://www.notion.so/34b47fdae49a80fc9b5cf59683c43a1d) · [Phase 2a multi-vault switcher](https://www.notion.so/34b47fdae49a80428509dd81db41891a))*

Tauri 2 desktop app (Rust backend + React/Vite/TS/Tailwind frontend).
Lives at `apps/desktop/`; the Rust side is `apps/desktop/src-tauri/`
(workspace member `orchext-desktop`) and the frontend at
`apps/desktop/src/`.

**Screens:**

- **Vault picker** (first run or "Switch vault"): directory dialog via
  `tauri-plugin-dialog`; `vault_open` creates the seed type dirs +
  `.orchext/`, opens the persistent stores, runs a full `reindex_from`,
  and returns a `VaultInfo` snapshot.
- **Documents**: three-pane layout — types sidebar, document list,
  detail editor. New / save / delete with frontmatter fields (id,
  type, visibility, tags, source) and a markdown body textarea.
  Every save goes through `vault.write` then `index.upsert` so search
  stays consistent.
- **Tokens**: list active + revoked tokens; issue form (label, scope
  checkboxes with a distinct `private` warning, TTL days); the secret
  is shown exactly once in a dismissable panel, then only the
  redacted `PublicTokenInfo` remains on screen.
- **Audit**: reverse-chronological table of `AuditEntry` rows with a
  "chain verified" / "chain broken" badge backed by
  `orchext_audit::verify`.

**Commands (Tauri backend):** `vault_open`, `vault_info`, `doc_list`,
`doc_read`, `doc_write`, `doc_delete`, `token_list`, `token_issue`,
`token_revoke`, `audit_list`. All are `async` and call the existing
crates directly — no subprocess to `orchext-mcp`.

**Decisions recorded here:**

- **Services managed as `tokio::sync::RwLock<Option<OpenVault>>`** in
  Tauri state. Commands `clone` out a `Services` snapshot of `Arc`s
  under a short read lock, then do their work without holding the
  lock, so concurrent requests don't serialize behind a slow command.
- **Frontend calls crates through Tauri commands, not an in-process
  MCP server.** An alternative was to embed `orchext-mcp` and talk to
  it over stdio internally. Direct calls are simpler, keep the MCP
  surface authoritative for agents (who are external by definition),
  and avoid re-serializing every payload through JSON-RPC twice.
- **Secret is shown once, then only `PublicTokenInfo`.** The
  `token_issue` command returns `{ info, secret }`; the UI renders
  the secret in a yellow dismissable panel with a copy button.
  After dismiss, `token_list` no longer has access to the secret
  (it was never stored in plaintext — Argon2id hash only).
- **Reindex on vault open.** Same argument as orchext-mcp: watcher
  (not yet wired in the desktop — see below) only fires on changes
  *after* it starts, so any docs edited outside the app need a
  ground-truth rebuild to surface in list/search.
- **Markdown body is a `<textarea>`, not a rich editor.** Scope cut.
  CodeMirror / rich preview is worth adding later but would have
  tripled the UI work for little gain at this stage.
- **Tailwind 3.4 + hand-rolled components** over shadcn/MUI/etc.
  One style system, no transitive design-system churn; easy to
  rip out if we pick a component library later.
- **Icon is a generated placeholder.** `icons/icon.png` is a 32x32
  solid-purple PNG produced from a Python one-liner; replace before
  any distribution build.

**Binary workflows:**

- **Dev:** `cd apps/desktop && npm run tauri dev` — vite on
  `localhost:1420`, Rust hot-reload from `src-tauri/`. Requires
  `~/.cargo/bin` on PATH (Tauri invokes `cargo metadata` at startup).
- **Build:** `npm run tauri build` — full `.app` / `.dmg` bundle.
  Not exercised yet; icon needs replacement first.

**Follow-ons shipped since MVP (2026-04-19):**

- **Fs watcher wired** — `src-tauri/src/watch.rs` mirrors the
  `orchext-mcp` pattern: notify watcher owns path→(type,id), calls
  `index.upsert`/`remove`, emits Tauri event `vault://changed`.
  `DocumentsView` and `GraphView` subscribe and auto-refresh. No
  debouncing; bursts may trigger several events per logical write.
- **Save indicator** — `DocEditor` flashes a transient "Saved ✓"
  pill for ~1.8s on success and persists a red error banner on
  failure. `role="status" aria-live="polite"` for assistive tech.
- **Graph view** (reconciled-v1-plan §v1 item 1) — new `Graph`
  nav tab. Backend: `graph_snapshot` Tauri command + a new
  `Index::all_edges()` that pulls every `(source, target)` link
  row in one SQL trip. Frontend: `react-force-graph-2d` canvas,
  click-to-jump to Documents. Orphan edges (target not in vault)
  are filtered out — this is a v1 simplification, not a bug.
- **In-app onboarding agent** (§v1 item 6) — first-run screen
  (auto-opened when `document_count == 0`, also a nav tab).
  Chat UI backed by `onboarding_chat` / `onboarding_finalize`
  Tauri commands that POST directly to Anthropic's `/v1/messages`
  endpoint via `reqwest` (no Rust SDK exists). Model pinned to
  `claude-haiku-4-5` for cost. Scope cuts: no streaming, no tool
  use (agent returns a JSON array of seed docs in a finalize turn),
  single-session only. API key stored in `.orchext/settings.json`
  alongside `tokens.json` — plaintext at rest, same threat model
  as the existing token file, move to OS keychain in a follow-up.

**Known gaps remaining:**

- **Obsidian import** (§v1 item 5) — explicitly cut from the MVP;
  not started.
- **API key in plaintext** — `.orchext/settings.json` is not
  encrypted. Fine for local dev, but should move to
  `tauri-plugin-stronghold` / OS keychain before any distribution
  build.
- **Fs watcher burst coalescing** — a single `echo >> file.md`
  can emit 2–3 `vault://changed` events. Harmless (React just
  re-fetches), but noisy; `notify-debouncer-mini` would smooth it.

**Phase 2a shipped (2026-04-19): Multi-vault + workspace switcher**

The desktop app now tracks N registered vaults and switches between
them from the header. Unblocks use case 5 locally (personal ↔ any
other local vault).

- **Registry at `~/.orchext/workspaces.json`** — per-install client
  state (not part of the vault format; see `FORMAT.md` §11.1). JSON
  with `{version, active, workspaces:[{id, name, kind, path,
  added_at}]}`. Atomic write via temp + rename. Workspace IDs are
  `ws_` + base64url of 8 random bytes (matches `tok_` pattern).
- **New Rust module:** `apps/desktop/src-tauri/src/workspaces.rs`
  (Registry + WorkspaceEntry + helpers) with 4 unit tests
  (empty-load, roundtrip, path-dedup, active-promotion on remove).
- **State model:** `AppState { registry_path, registry: RwLock<Registry>,
  open: RwLock<Option<OpenVault>> }`. Only the active workspace is
  open at a time; switching drops the old `OpenVault` (and its
  watcher) before opening the new one. Deliberate simplification:
  keeping N warm would require N watchers + coordinating the fs-event
  channel, and v1 vault sizes don't need it.
- **New commands:** `workspace_list`, `workspace_add(path, name?)`,
  `workspace_activate(id)`, `workspace_remove(id)`,
  `workspace_rename(id, name)`. `vault_open` is gone; frontend
  calls `workspace_add` instead.
- **`vault_info()` now auto-opens** the active registered workspace
  if present but not loaded. Returns `null` only on first run
  (registry empty). Existing `doc_*` / `token_*` / `audit_*`
  commands route through `active_services()`, which returns a clear
  "no workspace open" error if called before any workspace is
  registered.
- **`VaultInfo` grew** `workspace_id` and `name` fields so the
  frontend can key React children off the active workspace.
- **UI:** new `WorkspaceSwitcher.tsx` dropdown in the header showing
  active + list + "Add workspace…" + per-row Rename / Remove.
  Remove on the last remaining workspace is refused at the UI layer
  (the backend would simply leave an empty registry with no active).
- **Re-mount on switch:** `Layout.tsx`'s `<main>` carries
  `key={vault.workspace_id}`, so all child views (Documents, Graph,
  Tokens, Audit, Onboarding) unmount + remount on switch and re-
  fetch cleanly. Avoided threading a workspace prop through every
  child; React keying is the lighter touch.
- **Workspace isolation** is path-based (same as v1): each vault's
  `.orchext/` holds its own tokens, audit, index, proposals, settings.
  No cross-workspace data paths added.

**Decisions recorded here:**

- **Single-open, not multi-open.** As above; revisit only if
  workspace count grows past ~10 or users ask for cross-vault search.
- **Registry outside the vault, not inside.** Vault portability
  wins. A vault dropped onto another machine registers as a new
  workspace on that machine without rewriting anything inside it.
- **No React Router.** Workspace is React state in `App.tsx`, not
  a URL path segment. URL-based routing (`/w/:id/...`) was in the
  Phase 2a plan but was cut — it adds a dependency and deep-link
  semantics we don't yet need.
- **Rename is admin-free.** Users can rename any registered
  workspace from the switcher; no confirmation. Revisit if a
  workspace name ever appears in audit logs or tokens (it doesn't
  yet).

**Known gaps after Phase 2a:**

- **UI not exercised in a browser.** Code type-checks and
  `vite build` succeeds; interactive smoke-test deferred to the
  user or next session (Tauri dev needs the native shell).
- **Fs watcher thrash on rapid switches.** Switching workspaces
  tears down and recreates the watcher. Heavy clicking could
  produce brief gaps where file changes on the previous workspace
  would be missed; that workspace isn't active, so no user-visible
  effect. The next reindex on reactivation catches up.
- **No keyboard shortcut** for workspace switching yet.
