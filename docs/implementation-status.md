# Mytex — Implementation Status

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

**Last updated:** 2026-04-21

**Toolchain:** Rust 1.95.0 stable (rustup). Workspace at repo root.

**Test totals:** 148/148 passing with `DATABASE_URL` set; 128/128
without the DB-required suite.

| Crate          | Status        | Unit | Integration | Notes                                  |
|----------------|---------------|-----:|------------:|----------------------------------------|
| `mytex-vault`  | ✅ shipped     | 12   | 6           | Format parser + `PlainFileDriver`      |
| `mytex-audit`  | ✅ shipped     | 2    | 5           | Hash-chained JSONL log                 |
| `mytex-auth`   | ✅ shipped     | 11   | 9           | Opaque tokens + Argon2id + scopes      |
| `mytex-index`  | ✅ shipped     | 4    | 6           | SQLite + FTS5; search / graph / filter |
| `mytex-mcp`    | ✅ shipped     | 11   | 22          | JSON-RPC + stdio; rate limit + fs watcher |
| `mytex-desktop`| ✅ 2a + 2b.2 + 2b.3 | 7 | —           | Multi-vault + remote connect + unlock/lock |
| `mytex-server` | ✅ Phase 2b.3 | 20   | 20          | Auth + vault + index + tokens + audit + crypto |
| `mytex-sync`   | ✅ 2b.2 + 2b.3 | 0   | —           | `RemoteVaultDriver` + crypto control calls |
| `mytex-crypto` | ✅ Phase 2b.3 | 13   | —           | Argon2id KDF + XChaCha20-Poly1305 AEAD |

**Next up:** Phase 2b.4 — `apps/web` web client + WASM crypto. Pulled
ahead of MCP HTTP/SSE so a shareable URL lands sooner (no OAuth
dependency; web client uses the same session-token flow desktop uses
today). See [`phases/phase-2-plan.md`](phases/phase-2-plan.md).

---

## Phase docs

### Shipped (frozen)

- [`phases/phase-1-core.md`](phases/phase-1-core.md) — Core v1:
  vault, audit, auth, index, mcp, desktop (incl. Phase 2a
  multi-vault).
- [`phases/phase-2b1-server.md`](phases/phase-2b1-server.md) —
  Server skeleton + auth (axum, Postgres, sessions).
- [`phases/phase-2b2-remote-vault.md`](phases/phase-2b2-remote-vault.md) —
  Tenant-scoped vault/index/token/audit HTTP endpoints + `mytex-sync`
  client + desktop remote workspaces.
- [`phases/phase-2b3-encryption.md`](phases/phase-2b3-encryption.md) —
  `mytex-crypto` + session-bound decryption; encrypted
  `body_ciphertext`; desktop unlock/lock + heartbeat.

### Planned

- [`phases/phase-2-plan.md`](phases/phase-2-plan.md) — Phase 2 goals,
  decisions D7–D17, remaining sub-milestones (2b.4 web client,
  2b.5 MCP HTTP/OAuth/`context.propose`, 2c teams), scope cuts,
  open questions.
- [`phases/phase-3-plan.md`](phases/phase-3-plan.md) — Desktop
  distribution & installers (signed macOS DMG, Windows MSI, Linux,
  auto-updater).

---

## Out of scope / deferred

- Cloud sync + session-bound decryption — shipped, see
  [`phases/phase-2b3-encryption.md`](phases/phase-2b3-encryption.md).
- `context.propose` write-back flow — planned for Phase 2b.5.
- HTTP API — shipped, see
  [`phases/phase-2b2-remote-vault.md`](phases/phase-2b2-remote-vault.md).
- Desktop installers / signed builds — planned for Phase 3.

---

## Repo layout

```
mytex/
├─ Cargo.toml                 workspace root, Apache-2.0, MSRV 1.75
├─ crates/
│  ├─ mytex-vault/            ✅ shipped
│  ├─ mytex-audit/            ✅ shipped
│  ├─ mytex-auth/             ✅ shipped
│  ├─ mytex-index/            ✅ shipped
│  ├─ mytex-mcp/              ✅ shipped
│  ├─ mytex-server/           ✅ Phase 2b.3
│  │  ├─ src/                 lib + bin (axum HTTP API)
│  │  ├─ migrations/          sqlx migrations (Postgres)
│  │  ├─ tests/               auth_flow.rs + vault_flow.rs + crypto_flow.rs (need live Postgres)
│  │  ├─ Dockerfile           multi-stage, debian-slim runtime
│  │  ├─ docker-compose.yml   postgres + server; dev profile
│  │  └─ .env.example         reference env vars for compose
│  ├─ mytex-sync/             ✅ 2b.2 + 2b.3 — RemoteVaultDriver + crypto control
│  └─ mytex-crypto/           ✅ Phase 2b.3 — Argon2id KDF + XChaCha20-Poly1305 AEAD
├─ apps/
│  └─ desktop/                ✅ Phase 2a
│     ├─ src-tauri/           Rust (mytex-desktop crate)
│     └─ src/                 React + Vite + TS + Tailwind
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
      ├─ phase-2-plan.md
      └─ phase-3-plan.md
```

---

## Development quick-reference

### Running the full test suite

```bash
# Without Postgres: 109 tests pass (mytex-server integration tests skip).
cargo test --workspace

# With Postgres: 118 tests pass. Spin up a throwaway container:
docker run --rm -d --name mytex-test-pg \
  -e POSTGRES_USER=mytex -e POSTGRES_PASSWORD=testpw -e POSTGRES_DB=mytex_test \
  -p 5555:5432 postgres:16-alpine

DATABASE_URL="postgres://mytex:testpw@localhost:5555/mytex_test" \
  cargo test --workspace

docker stop mytex-test-pg
```

`sqlx::test` creates a fresh database per test function, so there is
no state bleed between tests. The throwaway container is for dev
ergonomics only; CI will want a persistent Postgres service.

### Running mytex-server locally

```bash
# From crates/mytex-server/:
cp .env.example .env
docker compose up            # postgres + server on localhost:8080
curl http://localhost:8080/healthz

# Or for a hot-reload dev loop on the server:
docker compose up -d postgres
DATABASE_URL="postgres://mytex:mytex-dev-password@localhost/mytex" \
  cargo run -p mytex-server
```

### Running the desktop app

```bash
cd apps/desktop
npm install
npm run tauri dev
```

First run shows the vault picker; registers the chosen directory as
a workspace in `~/.mytex/workspaces.json`. Subsequent launches
auto-open the active workspace.
