# Orchext — Implementation Status

Master index. Running status of the v1 build, updated after each
crate or significant milestone. Other docs describe *intent*
(`ARCHITECTURE.md`, `FORMAT.md`, `MCP.md`, `reconciled-v1-plan.md`);
this one describes *state*. Per-phase detail lives in
[`phases/`](phases/) so any one doc stays readable; a new session
should be able to open this file and the phase it's working in, and
know exactly where things stand.

**Per-phase docs:** keep each under ~500 lines. If a phase is pushing
that limit, consolidate scope or break out a sub-phase.

---

## Snapshot

**Last updated:** 2026-04-27

**Toolchain:** Rust 1.95.0 stable (rustup) + Node 20+ for the web /
desktop frontends. wasm-pack 0.14 drives the browser crypto build.
Workspace at repo root.

**Test totals:** 224/224 passing with `DATABASE_URL` set; 162/162
without the DB-required suite (Rust only — `apps/web` has no JS test
suite yet). +52 across the OAuth + MCP HTTP rounds, +4 in the
initial deployment-hardening pass (1 `/readyz` happy-path, 3 CORS
layer tests), +1 in the post-launch hardening pass (auth rate-limit
XFF regression — pins the signup/login fix below), +10 in slice 4
(`context.propose`): +6 server integration tests covering the
propose → list → approve / reject paths and `version_conflict` /
`proposals_disabled` gates, +4 stdio MCP unit tests for the
`.orchext/proposals/` spool path.

**Scope shuffle 2026-04-25:** four scope changes folded in one pass.
(1) **Graph view dropped.** Desktop's `GraphView.tsx` +
`react-force-graph-2d` removed; web never adopted it. The view didn't
earn its UI weight against the documents list. (2) **2b.4 closed**
without onboarding parity — desktop's Anthropic-keyed onboarding chat
needs a server-mediated route the browser can call, deferred to
Phase 3 platform. (3) **2b.5 narrowed and started.** Begins with web
auth hardening (httpOnly session cookie + double-submit CSRF),
followed by OAuth 2.1 + PKCE for agent tokens, then MCP HTTP/SSE,
then `context.propose`. (4) **Phase 2c absorbed into Phase 3 platform**
alongside web onboarding chat and OS keychain — see
[`phases/phase-3-platform.md`](phases/phase-3-platform.md). Phase 3a
rebrand still kicks off the post-platform work.

**Rebrand 2026-04-21:** product renamed `mytex` → `orchext`. All
crates, bundle identifiers, env vars (`MYTEX_*` → `ORCHEXT_*`), vault
directory (`.mytex` → `.orchext`), and token prefix (`mtx_` → `ocx_`)
renamed in place. No backwards-compat shims — existing installs and
databases must be rebuilt.

**Rebrand planned 2026-04-22 (executes in Phase 3a):** product will
rename `orchext` → `orchext` (orchestration + context) once Phase
2b.4 wraps. Same playbook: `ORCHEXT_*` → `ORCHEXT_*`, `.orchext` →
`.orchext`, `ocx_*` → `ocx_*`, GitHub org/repo rename. Executes as
the kickoff of Phase 3a alongside the `type: task` / `type: skill`
seed types, because Phase 3 also absorbs the scope expansion into
task aggregation + agent orchestration. Plan detail in
[`phases/phase-3a-rebrand-tasks.md`](phases/phase-3a-rebrand-tasks.md).

| Crate          | Status        | Unit | Integration | Notes                                  |
|----------------|---------------|-----:|------------:|----------------------------------------|
| `orchext-vault`  | ✅ shipped     | 12   | 6           | Format parser + `PlainFileDriver`      |
| `orchext-audit`  | ✅ shipped     | 2    | 5           | Hash-chained JSONL log                 |
| `orchext-auth`   | ✅ shipped     | 11   | 9           | Opaque tokens + Argon2id + scopes      |
| `orchext-index`  | ✅ shipped     | 4    | 6           | SQLite + FTS5; search / graph / filter |
| `orchext-mcp`    | ✅ shipped     | 11   | 26          | JSON-RPC + stdio; rate limit + fs watcher; `context_propose` spool |
| `orchext-desktop`| ✅ 2a + 2b.2 + 2b.3 | 7 | —           | Multi-vault + remote connect + unlock/lock |
| `orchext-server` | ✅ 2b.3 + 2b.5 | 45 | 50          | Auth + vault + index + tokens + audit + crypto + OAuth + MCP HTTP + proposals + readiness + CORS |
| `orchext-sync`   | ✅ 2b.2 + 2b.3 | 0   | —           | `RemoteVaultDriver` + crypto control calls |
| `orchext-oauth-client` | ✅ 2b.5 | 9 | —           | PKCE agent helper + `orchext-oauth` CLI    |
| `orchext-crypto` | ✅ 2b.3 + wasm32 | 13 | —           | Argon2id KDF + XChaCha20-Poly1305 AEAD; browser build clean |
| `orchext-crypto-wasm` | ✅ 2b.4 | —  | —               | wasm-bindgen surface; 4 ops: generateSalt/ContentKey, wrap/unwrap |
| `orchext-web`    | ✅ 2b.4 + 2b.5 | — | —            | Login + tenant picker + unlock + doc CRUD + tokens + audit + OAuth consent |

**Production hardening 2026-04-26 (post first deploy to
`app.orchext.ai` + `test-app.orchext.ai`):** three fixes landed after
runtime probes against the live deployment. (1) **Auth rate-limiter
500.** `POST /v1/auth/{signup,login}` (and `/native/*` twins) returned
500 "Unable To Extract Key" — `tower_governor`'s default
`PeerIpKeyExtractor` reads peer IP from axum's `ConnectInfo` extension,
which the binary wasn't attaching, and behind Fly the peer is the
proxy anyway. Fix: `SmartIpKeyExtractor` (XFF first, ConnectInfo
fallback) + `into_make_service_with_connect_info::<SocketAddr>` in
`main.rs`. New regression test pins the XFF path with
`rate_limit_auth: true`. (2) **SPA security headers.** Added CSP
(`default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; …;
frame-ancestors 'none'; upgrade-insecure-requests`),
`X-Content-Type-Options: nosniff`, `Referrer-Policy:
strict-origin-when-cross-origin`, `Permissions-Policy` lockdown, and
HSTS upgraded to `max-age=63072000; includeSubDomains; preload` —
all via `apps/web/vercel.json` headers. (3) **CI workflows green for
the first time.** `server.yml` was failing on every push because
`cargo clippy --workspace` pulled the Tauri desktop crate, which
needs GTK system libs absent on `ubuntu-latest` (exit 101); excluded
the desktop crate from server CI. `web.yml` was failing on a
"committed wasm matches sources" diff — wasm-pack output isn't
byte-identical across host platforms, so the check was unfixable;
removed it (Vercel's no-Rust prebuild path remains the actual gate
for stale wasm).

**In flight:** Phase 2b.5 — auth hardening + agent surface. Web auth
hardening **closed 2026-04-25** *([Notion](https://www.notion.so/34d47fdae49a81d4add7cfd2b7151ca8))*:
server emits an httpOnly `orchext_session` cookie alongside a readable
`orchext_csrf` cookie on login/signup, and accepts either bearer
(desktop) or cookie (web) on authenticated routes. State-changing
cookie-authed requests must double-submit CSRF via `X-Orchext-CSRF`
header. Web client dropped its `localStorage` token entirely and
probes `/v1/auth/me` on load to classify session state.
**OAuth 2.1 + PKCE — shipped 2026-04-26**
*([Notion](https://www.notion.so/34b47fdae49a80f8bf91d7f85aa1590c))*. Three pieces:
(1) **Server surface** — `POST /v1/oauth/authorize` (session-authed)
issues a single-use 10-min `oac_*` code under PKCE S256 + redirect URI
validation; `POST /v1/oauth/token` exchanges `(code, verifier, redirect_uri)`
for an audience-bound `ocx_*` bearer in `mcp_tokens`. Migration
`0005_oauth.sql`. (2) **Web consent UI** — `apps/web` route
`/oauth/authorize` parses agent-supplied params from the URL,
gates on session auth, resolves tenant membership, renders an
approve/deny prompt with a private-scope warning, and 302s back to
`redirect_uri?code=…&state=…` on approve (or `error=access_denied&…`
on deny per RFC 6749 §4.1.2.1). (3) **Agent client** — new
`crates/orchext-oauth-client` library + `orchext-oauth` CLI binary
that runs the full flow: PKCE generation, `127.0.0.1:0` loopback
listener, browser opener, callback parsing (state mismatch +
favicon-prefetch handled), code exchange. **Desktop consent UI
deferred** until installer slice (Phase 4) — needs a
`tauri-plugin-deep-link` integration + per-OS `orchext://` scheme
registration that's much cheaper to land alongside packaged builds.
**MCP HTTP transport — shipped 2026-04-26**
*([Notion](https://www.notion.so/34b47fdae49a80cfaf2deabe4f71c339))*:
`POST /v1/mcp` exposes the JSON-RPC surface (initialize, ping,
tools/{list,call}, resources/{list,read}) authenticated against the
`mcp_tokens` table — closing the loop with the OAuth flow above so
agent-acquired bearers actually have somewhere to be used. Wire
format reuses orchext-mcp's rpc envelope + error codes + tool
definitions, so HTTP and stdio agents see byte-identical JSON.
SSE (`GET /v1/mcp/events`) + `notifications/*` deferred until a
real remote MCP client appears (every current MCP client uses
stdio); `resources/subscribe` rides with that.
**`context.propose` — shipped 2026-04-27**
*([Notion](https://www.notion.so/34b47fdae49a8090a361ca985f9ebd6c))*.
Four surfaces in one slice. (1) **Server schema + tool** — migration
`0006_proposals.sql` adds the `proposals` table; `context_propose`
exists on both stdio and HTTP MCP, gated on `mode = read_propose`,
with a best-effort base-version check at propose time and the
authoritative re-check inside the approve transaction.
(2) **Server review endpoints** — admin-gated `GET /v1/t/:tid/proposals`
(filterable by status), `GET /v1/t/:tid/proposals/:id`,
`POST /v1/t/:tid/proposals/:id/approve` (applies the patch,
re-encrypts under the live session key when the row was encrypted,
bumps `documents.version`, audit-logs `proposal.approve`),
`POST /v1/t/:tid/proposals/:id/reject` (status flip + audit). Patch
merge is shallow on frontmatter (`null` clears) and exactly-zero-or-
one body op (`body_replace` / `body_append`).
(3) **Web UI** — new `/proposals` pane in `apps/web` with
pending / approved / rejected / all filters, frontmatter + body diff
preview, approve / reject buttons that surface `version_conflict`
inline. (4) **Desktop UI** — same pane wired into `apps/desktop`,
unified DTO across local + remote backends so the React side
renders identically. Local workspaces back the pane by reading
`.orchext/proposals/<id>.json` files dropped by stdio `orchext-mcp`;
remote workspaces hit the new server endpoints via a
`crates/orchext-sync` `proposals` module. Phase 2b.5 closes with
this slice. Forward plan in [`phases/phase-2-plan.md`](phases/phase-2-plan.md).

**Just shipped:** Phase 2b.4 closed 2026-04-25. Web client gained
login + signup, tenant picker, browser unlock with WASM-side
KDF/AEAD, 4-minute content-key heartbeat, doc CRUD with
base-version optimistic concurrency, tokens admin, and audit list.
Graph view dropped from both clients; onboarding chat moved to
Phase 3 platform.

---

## Phase docs

### Shipped (frozen)

Each phase entry below cross-references its tracked Notion backlog
items. The full per-item index lives in
[`memory/notion_backlog_index.md`](../../.claude/projects/-Users-jonathanbutler-Documents-Development-orchext/memory/notion_backlog_index.md)
(local memory, not in-repo) and inline next to each item in the
phase docs themselves.

- [`phases/phase-1-core.md`](phases/phase-1-core.md) — Core v1:
  vault, audit, auth, index, mcp, desktop (incl. Phase 2a
  multi-vault).
  *(Notion: [vault](https://www.notion.so/34b47fdae49a8031b92bda39b62584a3) ·
  [audit](https://www.notion.so/34b47fdae49a80af81fdd485c4df22ad) ·
  [auth/tokens](https://www.notion.so/34b47fdae49a803b98f4eb9aed1e9e87) ·
  [index](https://www.notion.so/34b47fdae49a8046909ce0aa7d968984) ·
  [mcp](https://www.notion.so/34b47fdae49a8091904cd4790ea31aad) ·
  [desktop](https://www.notion.so/34b47fdae49a80fc9b5cf59683c43a1d) ·
  [Phase 2a multi-vault](https://www.notion.so/34b47fdae49a80428509dd81db41891a))*
- [`phases/phase-2b1-server.md`](phases/phase-2b1-server.md) —
  Server skeleton + auth (axum, Postgres, sessions).
  *([Notion](https://www.notion.so/34b47fdae49a80d7a07aca2c31db3cba))*
- [`phases/phase-2b2-remote-vault.md`](phases/phase-2b2-remote-vault.md) —
  Tenant-scoped vault/index/token/audit HTTP endpoints + `orchext-sync`
  client + desktop remote workspaces.
  *(Notion: [endpoints](https://www.notion.so/34b47fdae49a8007b10ecec54458f25e) ·
  [orchext-sync](https://www.notion.so/34b47fdae49a8054bd86c7de49c7dd7e) ·
  [desktop](https://www.notion.so/34d47fdae49a81718f80f6a184b3c3fc))*
- [`phases/phase-2b3-encryption.md`](phases/phase-2b3-encryption.md) —
  `orchext-crypto` + session-bound decryption; encrypted
  `body_ciphertext`; desktop unlock/lock + heartbeat.
  *(Notion: [crypto](https://www.notion.so/34b47fdae49a80adb0fac091491f0d60) ·
  [server session-key](https://www.notion.so/34b47fdae49a80fabe34da9df833c33e) ·
  [desktop unlock/heartbeat](https://www.notion.so/34b47fdae49a808988f3f14d8b846e9a))*
- [`phases/phase-2b4-web.md`](phases/phase-2b4-web.md) — `apps/web` +
  `orchext-crypto-wasm`; login, tenant picker, unlock, doc CRUD,
  tokens, audit. Closed 2026-04-25 without graph or onboarding chat.
  *(Notion: [web client](https://www.notion.so/34b47fdae49a806b8e86fcfb24fcdc8d) ·
  [WASM crypto](https://www.notion.so/34d47fdae49a810f8e65f18bb9667e21) ·
  [doc CRUD](https://www.notion.so/34d47fdae49a8109a1c2f5728d76bfca) ·
  [tokens + audit](https://www.notion.so/34d47fdae49a81a9af78cb30a33c225b))*

### In flight

_(none — Phase 2b.5 closed 2026-04-27 with `context.propose`.)_

### Planned

- [`phases/phase-2-plan.md`](phases/phase-2-plan.md) — Phase 2 goals,
  decisions D7–D17, remaining 2b.5 slices, scope cuts, open
  questions.
- [`phases/phase-3-platform.md`](phases/phase-3-platform.md) —
  Teams + invites (formerly Phase 2c), web onboarding chat, OS
  keychain. Bundles the work pushed out of 2b.4 and 2b.5 narrowing.
  *(Notion: [teams](https://www.notion.so/34b47fdae49a80a09100d7e9ec10afe8) ·
  [org/ seed type](https://www.notion.so/34b47fdae49a80f3aa60c780298ebe07) ·
  [team UI](https://www.notion.so/34b47fdae49a8033bec2e5f0a2eeaf33) ·
  [onboarding chat](https://www.notion.so/34d47fdae49a81d6a012e90cbbcb0d0b) ·
  [OS keychain](https://www.notion.so/34d47fdae49a819c8ce9dd6511989596))*
- [`phases/phase-3a-rebrand-tasks.md`](phases/phase-3a-rebrand-tasks.md) —
  Rebrand `orchext` → `orchext` + vault-native `type: task` and
  `type: skill` seed types (FORMAT v0.2). Kicks off after Phase 3
  platform wraps.
  *(Notion: [rebrand sweep](https://www.notion.so/34d47fdae49a811fb29af81c1e4e503a) ·
  [FORMAT v0.2](https://www.notion.so/34d47fdae49a812aa86cd06ccd5994de) ·
  [vault-native task docs](https://www.notion.so/34d47fdae49a816daa7ce593c1156a83) ·
  [orchext-tasks crate](https://www.notion.so/34d47fdae49a8197bf6aee1abc7f6b42) ·
  [index views](https://www.notion.so/34d47fdae49a81209ef3c925818eb982) ·
  [MCP task tools](https://www.notion.so/34d47fdae49a818ab89be2b10c2c8245) ·
  [Tasks pane](https://www.notion.so/34d47fdae49a81ed80e3de9472e96e5f) ·
  [Skills pane](https://www.notion.so/34d47fdae49a8131a033dfd0eae46506))*
- [`phases/phase-3b-integrations.md`](phases/phase-3b-integrations.md) —
  First external task integration (Todoist) + visibility-driven
  storage tier (`task_projection`) + server-held integration
  credentials. Introduces decisions D18, D22–D26.
  *(Notion 3b.1: [SECURITY.md carve-outs](https://www.notion.so/34d47fdae49a81ffa240dd96098a0b08) ·
  [task_projection](https://www.notion.so/34d47fdae49a8160b910d8da2c5101e5) ·
  [visibility flipping](https://www.notion.so/34d47fdae49a815083a4f99501186ee2) ·
  [GET /tasks](https://www.notion.so/34d47fdae49a81d29c14c35c9c5f337c).
  3b.2: [orchext-integrations + trait](https://www.notion.so/34d47fdae49a81779ee4c1a5d23f15cf) ·
  [Todoist OAuth](https://www.notion.so/34d47fdae49a81ae9be4d49aeee15ec2) ·
  [external_task_cache](https://www.notion.so/34d47fdae49a81e4a2e0c7133f73d1dc) ·
  [incoming tray UI](https://www.notion.so/34d47fdae49a81138124fdfa0a781adf) ·
  [promote-to-vault](https://www.notion.so/34d47fdae49a81709855ea142dbf1178))*
- [`phases/phase-3c-task-expansion.md`](phases/phase-3c-task-expansion.md) —
  Linear / Jira / Asana / MS To Do adapters + team-inbox aggregation
  (depends on the team workspaces shipped in Phase 3 platform).
  Decisions D27–D30.
  *(Notion: [Linear adapter + endpoint](https://www.notion.so/34d47fdae49a81129046c74a8ed72bd2) ·
  [team-inbox view](https://www.notion.so/34d47fdae49a81f9ac99daba62ea6d9d) ·
  [Jira/Asana/MS To Do adapters](https://www.notion.so/34d47fdae49a8105bdfaea3286db0c69) ·
  [status push-back](https://www.notion.so/34d47fdae49a81b1a665f4db9d32340c))*
- [`phases/phase-3d-agent-observer.md`](phases/phase-3d-agent-observer.md) —
  Agent sessions observer-only: `orchext-agents` crate, heartbeat
  protocol, client-encrypted transcripts, activity panes. Decisions
  D31–D35.
  *(Notion: [orchext-agents + trait](https://www.notion.so/34d47fdae49a81d4bdd4d941d0d974fa) ·
  [Claude Code adapter](https://www.notion.so/34d47fdae49a819baa8ff7668383d52a) ·
  [agent_sessions table](https://www.notion.so/34d47fdae49a8173b814cf78f7acce37) ·
  [server routes + heartbeat](https://www.notion.so/34d47fdae49a81c68b0cd656981f27a9) ·
  [transcript encryption](https://www.notion.so/34d47fdae49a8152b061eeb25afff98a) ·
  [cost ledger](https://www.notion.so/34d47fdae49a8171ba6cd316afb62753) ·
  [Activity pane](https://www.notion.so/34d47fdae49a81a0a466f0995a7e20f6))*
- [`phases/phase-3e-orchestration.md`](phases/phase-3e-orchestration.md) —
  Full orchestration surface: atomic task checkout, HITL approval
  gates, runtime skill injection, shared team agents, goal
  ancestry. Decisions D36–D42.
  *(Notion 3e.1: [orchestrator + checkout](https://www.notion.so/34d47fdae49a818abb69d3f52f6a2a3d) ·
  [goal ancestry](https://www.notion.so/34d47fdae49a81b48113e6d85ab5c4ea) ·
  [claimed-task push-back](https://www.notion.so/34d47fdae49a811995f5f1c28bceb2b1).
  3e.2: [HITL approvals](https://www.notion.so/34d47fdae49a8104aa50dbb69b0d458b) ·
  [Approvals queue UI](https://www.notion.so/34d47fdae49a817b8c66fa97a3a7ad9f) ·
  [skill injection](https://www.notion.so/34d47fdae49a81c69d1cc8d72149b209) ·
  [skill_list MCP tool](https://www.notion.so/34d47fdae49a81938f7cc5e15adb6816).
  3e.3: [shared sessions](https://www.notion.so/34d47fdae49a81c9afbee67244881eb1) ·
  [team session key slot](https://www.notion.so/34d47fdae49a81919048d52fecd383b6) ·
  [orchestrator:manage](https://www.notion.so/34d47fdae49a81368f2af140cd9141aa) ·
  [Board oversight](https://www.notion.so/34d47fdae49a81b8a1f0c7ed7a0f8400))*
- [`phases/phase-4-installers.md`](phases/phase-4-installers.md) —
  Desktop distribution & installers (signed macOS DMG, Windows MSI,
  Linux, auto-updater). Renumbered from Phase 3 on 2026-04-22.
  *(Notion: [4.1 macOS DMG](https://www.notion.so/34d47fdae49a81ac90a2e402e46dda59) ·
  [4.2 Windows MSI](https://www.notion.so/34d47fdae49a8156870fcccef4cbb2ae) ·
  [4.3 Linux packages](https://www.notion.so/34d47fdae49a811393f1eaa509d25d45) ·
  [4.4 auto-updater](https://www.notion.so/34d47fdae49a8171b5e6e1dec05fee4b) ·
  [4.5 download landing](https://www.notion.so/34d47fdae49a81f89104fe82a3890b46))*

---

## Out of scope / deferred

- Cloud sync + session-bound decryption — shipped, see
  [`phases/phase-2b3-encryption.md`](phases/phase-2b3-encryption.md).
- `context.propose` write-back flow — shipped 2026-04-27 (Phase 2b.5).
- HTTP API — shipped, see
  [`phases/phase-2b2-remote-vault.md`](phases/phase-2b2-remote-vault.md).
- Desktop installers / signed builds — planned for Phase 4
  (formerly Phase 3; renumbered 2026-04-22).

---

## Repo layout

```
orchext/
├─ Cargo.toml                 workspace root, Apache-2.0, MSRV 1.75
├─ crates/
│  ├─ orchext-vault/            ✅ shipped
│  ├─ orchext-audit/            ✅ shipped
│  ├─ orchext-auth/             ✅ shipped
│  ├─ orchext-index/            ✅ shipped
│  ├─ orchext-mcp/              ✅ shipped
│  ├─ orchext-server/           ✅ Phase 2b.3
│  │  ├─ src/                 lib + bin (axum HTTP API)
│  │  ├─ migrations/          sqlx migrations (Postgres)
│  │  ├─ tests/               auth_flow.rs + vault_flow.rs + crypto_flow.rs (need live Postgres)
│  │  ├─ Dockerfile           multi-stage, debian-slim runtime
│  │  ├─ docker-compose.yml   postgres + server; dev profile
│  │  └─ .env.example         reference env vars for compose
│  ├─ orchext-sync/             ✅ 2b.2 + 2b.3 — RemoteVaultDriver + crypto control
│  ├─ orchext-crypto/           ✅ 2b.3 + wasm32 — Argon2id KDF + XChaCha20-Poly1305 AEAD
│  └─ orchext-crypto-wasm/      ✅ 2b.4 — wasm-bindgen surface for the browser
├─ apps/
│  ├─ desktop/                ✅ Phase 2a
│  │  ├─ src-tauri/           Rust (orchext-desktop crate)
│  │  └─ src/                 React + Vite + TS + Tailwind
│  └─ web/                    🚧 Phase 2b.4 — in flight
│     ├─ src/                 React + Vite + TS + Tailwind (no Tauri)
│     └─ src/wasm/            wasm-pack output (generated, gitignored)
└─ docs/
   ├─ ARCHITECTURE.md         v1 contract + Phase 2 overview
   ├─ FORMAT.md               vault format spec + Phase 2 planned additions
   ├─ MCP.md                  MCP surface spec + Phase 2 roadmap
   ├─ reconciled-v1-plan.md   v1 decisions (D1–D6)
   ├─ comparison-architecture.md  alternate proposal (input only; superseded)
   ├─ implementation-status.md   this file — master index
   └─ phases/                 per-phase status docs (shipped + planned)
      ├─ phase-1-core.md
      ├─ phase-2b1-server.md
      ├─ phase-2b2-remote-vault.md
      ├─ phase-2b3-encryption.md
      ├─ phase-2b4-web.md
      ├─ phase-2-plan.md
      ├─ phase-3a-rebrand-tasks.md
      ├─ phase-3b-integrations.md
      ├─ phase-3c-task-expansion.md
      ├─ phase-3d-agent-observer.md
      ├─ phase-3e-orchestration.md
      └─ phase-4-installers.md
```

---

## Development quick-reference

### Running the full test suite

```bash
# Without Postgres: 109 tests pass (orchext-server integration tests skip).
cargo test --workspace

# With Postgres: 118 tests pass. Spin up a throwaway container:
docker run --rm -d --name orchext-test-pg \
  -e POSTGRES_USER=orchext -e POSTGRES_PASSWORD=testpw -e POSTGRES_DB=orchext_test \
  -p 5555:5432 postgres:16-alpine

DATABASE_URL="postgres://orchext:testpw@localhost:5555/orchext_test" \
  cargo test --workspace

docker stop orchext-test-pg
```

`sqlx::test` creates a fresh database per test function, so there is
no state bleed between tests. The throwaway container is for dev
ergonomics only; CI will want a persistent Postgres service.

### Running orchext-server locally

```bash
# From crates/orchext-server/:
cp .env.example .env
docker compose up            # postgres + server on localhost:8080
curl http://localhost:8080/healthz

# Or for a hot-reload dev loop on the server:
docker compose up -d postgres
DATABASE_URL="postgres://orchext:orchext-dev-password@localhost/orchext" \
  cargo run -p orchext-server
```

### Running the desktop app

```bash
cd apps/desktop
npm install
npm run tauri dev
```

First run shows the vault picker; registers the chosen directory as
a workspace in `~/.orchext/workspaces.json`. Subsequent launches
auto-open the active workspace.

### Running the web app

```bash
# Requires wasm-pack on PATH (cargo install wasm-pack).
cd apps/web
npm install
npm run dev                  # http://localhost:1430
```

`predev` and `prebuild` hooks run `wasm-pack build` against
`orchext-crypto-wasm` so the WASM module is always fresh. Set
`ORCHEXT_SERVER_URL` to override the proxy target
(default `http://localhost:8080`).
