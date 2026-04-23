# Ourtex — Implementation Status

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

**Last updated:** 2026-04-22

**Toolchain:** Rust 1.95.0 stable (rustup) + Node 20+ for the web /
desktop frontends. wasm-pack 0.14 drives the browser crypto build.
Workspace at repo root.

**Test totals:** 148/148 passing with `DATABASE_URL` set; 128/128
without the DB-required suite (Rust only — `apps/web` has no JS test
suite yet).

**Rebrand 2026-04-21:** product renamed `mytex` → `ourtex`. All
crates, bundle identifiers, env vars (`MYTEX_*` → `OURTEX_*`), vault
directory (`.mytex` → `.ourtex`), and token prefix (`mtx_` → `otx_`)
renamed in place. No backwards-compat shims — existing installs and
databases must be rebuilt.

**Rebrand planned 2026-04-22 (executes in Phase 3a):** product will
rename `ourtex` → `orchext` (orchestration + context) once Phase
2b.4 wraps. Same playbook: `OURTEX_*` → `ORCHEXT_*`, `.ourtex` →
`.orchext`, `otx_*` → `ocx_*`, GitHub org/repo rename. Executes as
the kickoff of Phase 3a alongside the `type: task` / `type: skill`
seed types, because Phase 3 also absorbs the scope expansion into
task aggregation + agent orchestration. Plan detail in
[`phases/phase-3a-rebrand-tasks.md`](phases/phase-3a-rebrand-tasks.md).

| Crate          | Status        | Unit | Integration | Notes                                  |
|----------------|---------------|-----:|------------:|----------------------------------------|
| `ourtex-vault`  | ✅ shipped     | 12   | 6           | Format parser + `PlainFileDriver`      |
| `ourtex-audit`  | ✅ shipped     | 2    | 5           | Hash-chained JSONL log                 |
| `ourtex-auth`   | ✅ shipped     | 11   | 9           | Opaque tokens + Argon2id + scopes      |
| `ourtex-index`  | ✅ shipped     | 4    | 6           | SQLite + FTS5; search / graph / filter |
| `ourtex-mcp`    | ✅ shipped     | 11   | 22          | JSON-RPC + stdio; rate limit + fs watcher |
| `ourtex-desktop`| ✅ 2a + 2b.2 + 2b.3 | 7 | —           | Multi-vault + remote connect + unlock/lock |
| `ourtex-server` | ✅ Phase 2b.3 | 20   | 20          | Auth + vault + index + tokens + audit + crypto |
| `ourtex-sync`   | ✅ 2b.2 + 2b.3 | 0   | —           | `RemoteVaultDriver` + crypto control calls |
| `ourtex-crypto` | ✅ 2b.3 + wasm32 | 13 | —           | Argon2id KDF + XChaCha20-Poly1305 AEAD; browser build clean |
| `ourtex-crypto-wasm` | ✅ 2b.4 | —  | —               | wasm-bindgen surface; 4 ops: generateSalt/ContentKey, wrap/unwrap |
| `ourtex-web`    | 🚧 2b.4 unlock   | —  | —           | Vite + React + Tailwind; login + tenant picker + unlock + read-only docs |

**In flight:** Phase 2b.4 — `apps/web` web client + WASM crypto.
Opened 2026-04-22. `ourtex-crypto` builds clean for
`wasm32-unknown-unknown`; new `ourtex-crypto-wasm` wrapper crate
exposes four wasm-bindgen functions (generateSalt, generateContentKey,
wrapContentKey, unwrapContentKey) consumed by `apps/web` via wasm-pack.
Browser unlock flow wired: `UnlockView` handles both seed-fresh and
unwrap-seeded paths, publishes the content key, and a 4-minute
heartbeat keeps the server-side TTL alive. Still to wire: writes
(`docWrite`, `docDelete`), tokens/audit views, onboarding parity with
desktop, and hardening the session token off `localStorage`. Details
in [`phases/phase-2b4-web.md`](phases/phase-2b4-web.md); forward plan
in [`phases/phase-2-plan.md`](phases/phase-2-plan.md).

---

## Phase docs

### Shipped (frozen)

- [`phases/phase-1-core.md`](phases/phase-1-core.md) — Core v1:
  vault, audit, auth, index, mcp, desktop (incl. Phase 2a
  multi-vault).
- [`phases/phase-2b1-server.md`](phases/phase-2b1-server.md) —
  Server skeleton + auth (axum, Postgres, sessions).
- [`phases/phase-2b2-remote-vault.md`](phases/phase-2b2-remote-vault.md) —
  Tenant-scoped vault/index/token/audit HTTP endpoints + `ourtex-sync`
  client + desktop remote workspaces.
- [`phases/phase-2b3-encryption.md`](phases/phase-2b3-encryption.md) —
  `ourtex-crypto` + session-bound decryption; encrypted
  `body_ciphertext`; desktop unlock/lock + heartbeat.

### In flight

- [`phases/phase-2b4-web.md`](phases/phase-2b4-web.md) — `apps/web` +
  `ourtex-crypto-wasm`; login, tenant picker, unlock, read-only docs
  shipped 2026-04-22. Writes / tokens / audit still to wire.

### Planned

- [`phases/phase-2-plan.md`](phases/phase-2-plan.md) — Phase 2 goals,
  decisions D7–D17, remaining sub-milestones (2b.4 web client in
  flight, 2b.5 MCP HTTP/OAuth/`context.propose`, 2c teams), scope
  cuts, open questions.
- [`phases/phase-3a-rebrand-tasks.md`](phases/phase-3a-rebrand-tasks.md) —
  Rebrand `ourtex` → `orchext` + vault-native `type: task` and
  `type: skill` seed types (FORMAT v0.2). Kicks off Phase 3.
- [`phases/phase-3b-integrations.md`](phases/phase-3b-integrations.md) —
  First external task integration (Todoist) + visibility-driven
  storage tier (`task_projection`) + server-held integration
  credentials. Introduces decisions D18, D22–D26.
- [`phases/phase-3c-task-expansion.md`](phases/phase-3c-task-expansion.md) —
  Linear / Jira / Asana / MS To Do adapters + team-inbox aggregation
  (depends on Phase 2c teams). Decisions D27–D30.
- [`phases/phase-3d-agent-observer.md`](phases/phase-3d-agent-observer.md) —
  Agent sessions observer-only: `orchext-agents` crate, heartbeat
  protocol, client-encrypted transcripts, activity panes. Decisions
  D31–D35.
- [`phases/phase-3e-orchestration.md`](phases/phase-3e-orchestration.md) —
  Full orchestration surface: atomic task checkout, HITL approval
  gates, runtime skill injection, shared team agents, goal
  ancestry. Decisions D36–D42.
- [`phases/phase-4-installers.md`](phases/phase-4-installers.md) —
  Desktop distribution & installers (signed macOS DMG, Windows MSI,
  Linux, auto-updater). Renumbered from Phase 3 on 2026-04-22.

---

## Out of scope / deferred

- Cloud sync + session-bound decryption — shipped, see
  [`phases/phase-2b3-encryption.md`](phases/phase-2b3-encryption.md).
- `context.propose` write-back flow — planned for Phase 2b.5.
- HTTP API — shipped, see
  [`phases/phase-2b2-remote-vault.md`](phases/phase-2b2-remote-vault.md).
- Desktop installers / signed builds — planned for Phase 4
  (formerly Phase 3; renumbered 2026-04-22).

---

## Repo layout

```
ourtex/
├─ Cargo.toml                 workspace root, Apache-2.0, MSRV 1.75
├─ crates/
│  ├─ ourtex-vault/            ✅ shipped
│  ├─ ourtex-audit/            ✅ shipped
│  ├─ ourtex-auth/             ✅ shipped
│  ├─ ourtex-index/            ✅ shipped
│  ├─ ourtex-mcp/              ✅ shipped
│  ├─ ourtex-server/           ✅ Phase 2b.3
│  │  ├─ src/                 lib + bin (axum HTTP API)
│  │  ├─ migrations/          sqlx migrations (Postgres)
│  │  ├─ tests/               auth_flow.rs + vault_flow.rs + crypto_flow.rs (need live Postgres)
│  │  ├─ Dockerfile           multi-stage, debian-slim runtime
│  │  ├─ docker-compose.yml   postgres + server; dev profile
│  │  └─ .env.example         reference env vars for compose
│  ├─ ourtex-sync/             ✅ 2b.2 + 2b.3 — RemoteVaultDriver + crypto control
│  ├─ ourtex-crypto/           ✅ 2b.3 + wasm32 — Argon2id KDF + XChaCha20-Poly1305 AEAD
│  └─ ourtex-crypto-wasm/      ✅ 2b.4 — wasm-bindgen surface for the browser
├─ apps/
│  ├─ desktop/                ✅ Phase 2a
│  │  ├─ src-tauri/           Rust (ourtex-desktop crate)
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
# Without Postgres: 109 tests pass (ourtex-server integration tests skip).
cargo test --workspace

# With Postgres: 118 tests pass. Spin up a throwaway container:
docker run --rm -d --name ourtex-test-pg \
  -e POSTGRES_USER=ourtex -e POSTGRES_PASSWORD=testpw -e POSTGRES_DB=ourtex_test \
  -p 5555:5432 postgres:16-alpine

DATABASE_URL="postgres://ourtex:testpw@localhost:5555/ourtex_test" \
  cargo test --workspace

docker stop ourtex-test-pg
```

`sqlx::test` creates a fresh database per test function, so there is
no state bleed between tests. The throwaway container is for dev
ergonomics only; CI will want a persistent Postgres service.

### Running ourtex-server locally

```bash
# From crates/ourtex-server/:
cp .env.example .env
docker compose up            # postgres + server on localhost:8080
curl http://localhost:8080/healthz

# Or for a hot-reload dev loop on the server:
docker compose up -d postgres
DATABASE_URL="postgres://ourtex:ourtex-dev-password@localhost/ourtex" \
  cargo run -p ourtex-server
```

### Running the desktop app

```bash
cd apps/desktop
npm install
npm run tauri dev
```

First run shows the vault picker; registers the chosen directory as
a workspace in `~/.ourtex/workspaces.json`. Subsequent launches
auto-open the active workspace.

### Running the web app

```bash
# Requires wasm-pack on PATH (cargo install wasm-pack).
cd apps/web
npm install
npm run dev                  # http://localhost:1430
```

`predev` and `prebuild` hooks run `wasm-pack build` against
`ourtex-crypto-wasm` so the WASM module is always fresh. Set
`OURTEX_SERVER_URL` to override the proxy target
(default `http://localhost:8080`).
