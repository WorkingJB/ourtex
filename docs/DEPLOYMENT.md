# Orchext Deployment Architecture

> How the SaaS-hosted version of Orchext is deployed, configured, and operated — and how it relates to the self-hostable artifact in this repo.

This document is the durable reference for the hosted deployment of
Orchext at `app.orchext.ai` (production) and `test-app.orchext.ai`
(staging). It describes what runs where, why, how it's configured, and
what changes when traffic, cost, or risk grow.

It is the deployment counterpart to [`ARCHITECTURE.md`](./ARCHITECTURE.md).
Where the architecture doc says *what the system is*, this doc says
*how the SaaS instance of it lives in the world*.

---

## 1. Guiding principles

The deployment is evaluated against five principles, in order:

1. **No SaaS-only forks of the code.** Anything we change to support
   hosting must be configurable and must not regress the self-host
   path. The `Dockerfile` and `docker-compose.yml` in
   `crates/orchext-server/` remain the canonical self-host artifact.
2. **The user's claim that "your context isn't on someone else's
   server" survives the SaaS deployment.** Encryption-at-rest stays
   client-controlled; the host operator (us) cannot read user
   documents without a published session key.
3. **Boring, replaceable infrastructure.** Every chosen vendor must be
   exitable with `pg_dump`, a Docker image, and DNS — no bespoke
   runtimes, no proprietary protocols, no platform-locked features
   that we'd have to rewrite to leave.
4. **Lowest-mental-tax operations.** We are a small team. We pay a
   small premium for managed Postgres, managed TLS, managed
   deploys. We do not run our own database server.
5. **Cheap to start, predictable to scale.** First-month bill should
   be under $50; growth costs should rise sub-linearly with users.

When principles conflict, earlier ones win.

---

## 2. Topology

```
                         ┌──────────────────────────────────┐
                         │             Vercel               │
   user browser ───TLS───►  app.orchext.ai (prod SPA)       │
                         │  test-app.orchext.ai (test SPA)  │
                         └──────────────┬───────────────────┘
                                        │
                          /v1/*  rewrite (same-origin to browser)
                                        │
                                        ▼
                         ┌──────────────────────────────────┐
                         │             Fly.io               │
                         │  orchext-prod.fly.dev   (Rust)   │
                         │  orchext-test.fly.dev   (Rust)   │
                         └──────────────┬───────────────────┘
                                        │
                                        │  DATABASE_URL (TLS)
                                        ▼
                         ┌──────────────────────────────────┐
                         │              Neon                │
                         │  orchext-prod  Postgres project  │
                         │  orchext-test  Postgres project  │
                         └──────────────────────────────────┘
```

Three vendors, three concerns:

- **Vercel** serves the static React/Vite bundle and proxies API
  requests so the browser sees one origin.
- **Fly.io** runs the Rust `orchext-server` binary from the existing
  `Dockerfile`. Two apps — one per environment.
- **Neon** provides Postgres. Two projects — one per environment.

DNS for `orchext.ai` lives at the registrar (e.g. Cloudflare DNS or
Route 53 — see §6).

---

## 3. Component: Web (Vercel)

### 3.1 What runs there

The static build of `apps/web` (Vite + React + the
`orchext-tauri-shim` WASM bundle). Two Vercel projects, one per
environment, both deployed from the same Git repo.

| | Production | Test |
|---|---|---|
| Domain | `app.orchext.ai` | `test-app.orchext.ai` |
| Vercel project | `orchext-web-prod` | `orchext-web-test` |
| Branch deployed | `main` | `develop` (or `main` until a develop branch exists) |
| Build command | `npm run build` (in `apps/web`) | same |
| Output dir | `apps/web/dist` | same |

### 3.2 Routing — same-origin via rewrites

The browser must see one origin so the existing cookie model
(`HttpOnly` + `Secure` + `SameSite=Lax`) keeps working without any
server change. The committed [`apps/web/vercel.json`](../apps/web/vercel.json)
configures rewrites with host-conditional `has` clauses:

| Source path | Match | Destination |
|---|---|---|
| `/v1/:path*` | host = `app.orchext.ai` | `https://orchext-prod.fly.dev/v1/:path*` |
| `/healthz`   | host = `app.orchext.ai` | `https://orchext-prod.fly.dev/healthz` |
| `/v1/:path*` | host = `test-app.orchext.ai` | `https://orchext-test.fly.dev/v1/:path*` |
| `/healthz`   | host = `test-app.orchext.ai` | `https://orchext-test.fly.dev/healthz` |
| `/v1/:path*` | (fallback) | `https://orchext-test.fly.dev/v1/:path*` |
| `/healthz`   | (fallback) | `https://orchext-test.fly.dev/healthz` |

One `vercel.json` works for both projects because Vercel evaluates
rewrites in order and matches each rule's `has` clause against the
inbound request host. Preview deploys (`*.vercel.app` URLs) hit the
fallback rules and route to test, so previews never touch
production data. The browser only ever sees `app.orchext.ai` — no
CORS, no `SameSite=None`, no cross-origin cookie surface.

### 3.3 What changes in the codebase

- No code change in `apps/web/src/api.ts` — relative paths already
  work.
- No code change in `crates/orchext-server`.
- `apps/web/vercel.json` carries host-conditional rewrites; both
  projects deploy from the same file (see
  [`deploy/vercel/README.md`](../deploy/vercel/README.md)).

---

## 4. Component: API (Fly.io)

### 4.1 What runs there

A single binary, `orchext-server`, built from the existing
`crates/orchext-server/Dockerfile`. One Fly app per environment.

| | Production | Test |
|---|---|---|
| Fly app | `orchext-prod` | `orchext-test` |
| Public hostname | `orchext-prod.fly.dev` | `orchext-test.fly.dev` |
| Region | `sjc` (us-west) — pinned to match Neon's `aws-us-west-2` | same |
| VM size | `shared-cpu-1x`, 1024 MB | `shared-cpu-1x`, 256 MB |
| Min instances | 1 (always-on) | 0 (auto-stop when idle) |
| Healthcheck | `GET /healthz` — must return 200 | same |

### 4.2 Configuration

Provided as `fly.toml` in `deploy/fly/orchext-prod.toml` and
`deploy/fly/orchext-test.toml`. Both reference the same Dockerfile:

```toml
# deploy/fly/orchext-prod.toml
app = "orchext-prod"
primary_region = "iad"

[build]
  dockerfile = "../../crates/orchext-server/Dockerfile"

[env]
  ORCHEXT_BIND = "0.0.0.0:8080"
  ORCHEXT_SECURE_COOKIES = "1"
  RUST_LOG = "orchext_server=info,axum=info,sqlx=warn"

[http_service]
  internal_port = 8080
  force_https = true
  auto_stop_machines = false
  auto_start_machines = true
  min_machines_running = 1

  [[http_service.checks]]
    method = "GET"
    path = "/healthz"
    interval = "15s"
    timeout = "5s"
    grace_period = "10s"
```

Test env differs in `auto_stop_machines = true`, `min_machines_running = 0`,
smaller VM size.

### 4.3 Secrets

Set per Fly app via `fly secrets set`:

| Secret | Source | Notes |
|---|---|---|
| `DATABASE_URL` | Neon project — *direct* (unpooled) connection string | sqlx 0.8 has its own pool; do not use Neon's pgbouncer endpoint. See §5.3. |
| `ORCHEXT_COOKIE_KEY` (future) | locally generated 32 random bytes (base64) | Currently inferred at startup; pin once cookie signing is added. |
| `ORCHEXT_OAUTH_*` (future) | per-IdP credentials when desktop OAuth lands | Not used in current OAuth slice (in-app PKCE only). |

Secrets are encrypted at rest by Fly and injected as env vars at
runtime. They never appear in `fly.toml`, never in Git.

### 4.4 What changes in the codebase

- Add `deploy/fly/orchext-prod.toml` and `deploy/fly/orchext-test.toml`.
- Add `/healthz` endpoint in `crates/orchext-server/src/router.rs` if
  not present — must return 200 with no DB access (kubernetes-style
  liveness).
- Optional: a deeper `/readyz` that does a `SELECT 1` against the
  pool, used by Fly's checks.
- No change to the Dockerfile beyond §8 hardening.

---

## 5. Component: Database (Neon)

### 5.1 What runs there

Two Neon projects, one per environment. Each project hosts a single
database (`orchext`) that Fly's `orchext-server` connects to.

| | Production | Test |
|---|---|---|
| Neon project | `orchext-prod` | `orchext-test` |
| Plan | Launch ($19/mo) | Free tier or branch off prod |
| Region | AWS `us-west-2` (must match Fly region) | same |
| Postgres version | 17 | 17 |
| Storage cap (start) | 10 GB | 0.5 GB |
| PITR window | 7 days | none on free tier |
| Compute auto-suspend | disabled or ≥1 hr idle | default (5 min) |

### 5.2 Why Neon, not Fly Postgres

| | Neon | Fly Managed Postgres |
|---|---|---|
| Free tier | 0.5 GB | none |
| Paid entry | $19/mo (10 GB, autoscale) | ~$30–50/mo (1 vCPU, 50 GB) |
| Branching | git-like DB branches | none |
| Cold start | 500ms–1s after idle | none |
| Storage durability | continuously replicated to S3 | snapshot-based |
| Wire protocol | plain Postgres | plain Postgres |

For our scale (≤5000 users until further notice) the cost gap is
material: Neon's $19 vs Fly Postgres' ~$30–50 saves enough monthly
to fund the test environment outright. Cold-start latency is
acceptable for an interactive web app where the DB is hot whenever
anyone has used the app in the last hour.

We **do not use Supabase**, Railway DB, or RDS, even though all are
plain Postgres. The decision is reversible by changing one
connection string.

### 5.3 Connection string discipline

Neon exposes two endpoints per branch:

- **Direct (unpooled)** — host like `ep-xxx.us-west-2.aws.neon.tech`.
  Standard Postgres, session-mode, supports prepared statements.
- **Pooled (pgbouncer transaction mode)** — host like
  `ep-xxx-pooler.us-west-2.aws.neon.tech`.

`orchext-server` uses sqlx 0.8 which has its own connection pool
(`PgPoolOptions::new().max_connections(10)`). Long-running Rust
processes don't need Neon's pooler, and transaction-mode pooling
breaks prepared-statement caching. **Always use the direct endpoint
in `DATABASE_URL`.**

### 5.4 Migrations

Migrations live in `crates/orchext-server/migrations/` and are run on
startup by `orchext_server::migrate(&db)`. No separate migration step
in the deploy pipeline — the binary does it itself.

Risk this introduces: on first boot of a new release, the binary may
be unable to start until migrations succeed. Acceptable for our
scale. If a migration is large or slow, switch to a pre-deploy
`fly ssh console` migration step ahead of swapping traffic.

---

## 6. DNS

DNS is the layer that makes everything else replaceable. Records
should live at a registrar that supports CNAME at apex (Cloudflare
DNS or Route 53), not be entangled with any compute vendor.

| Record | Type | Target | Owned by |
|---|---|---|---|
| `app.orchext.ai` | CNAME | `cname.vercel-dns.com` | Vercel project |
| `test-app.orchext.ai` | CNAME | `cname.vercel-dns.com` | Vercel project |
| `orchext.ai` apex | A / ALIAS | marketing site target | (separate concern) |

Fly hostnames (`orchext-prod.fly.dev`) are intentionally **not**
custom-mapped. The browser never sees them — Vercel does. If the API
ever needs a public hostname (third-party MCP clients, CLI users),
add `api.orchext.ai` then.

TLS is handled by Vercel for the SPA hostnames and by Fly for the API
hostname. Both auto-renew. We do not manage certificates.

---

## 7. Environment configuration

A single source of truth for what each environment expects.

### 7.1 Web (Vercel)

| Variable | Prod | Test | Notes |
|---|---|---|---|
| `VITE_ENV_NAME` | `production` | `test` | Optional, for diagnostics banner. |
| `VITE_BUILD_SHA` | injected by Vercel | injected | For UI footer / Sentry tags. |

The web app has no API base URL config — it relies on Vercel
rewrites (§3.2).

### 7.2 API (Fly.io)

| Variable | Prod | Test | Source |
|---|---|---|---|
| `DATABASE_URL` | Neon prod direct URL | Neon test direct URL | `fly secrets` |
| `ORCHEXT_BIND` | `0.0.0.0:8080` | same | `fly.toml` |
| `ORCHEXT_SECURE_COOKIES` | `1` | `1` | `fly.toml` |
| `ORCHEXT_DB_MAX_CONNECTIONS` | `10` | `5` | `fly.toml` |
| `RUST_LOG` | `orchext_server=info,axum=info,sqlx=warn` | same | `fly.toml` |

Anything secret goes through `fly secrets`. Anything non-secret stays
in version-controlled `fly.toml`.

---

## 8. Hardening checklist

Work to land before we cut over to real users. Each item is small;
the checklist exists so nothing is forgotten.

### 8.1 Server (`crates/orchext-server`)

- [x] **`/healthz` endpoint** — returns 200, no DB access.
- [x] **`/readyz` endpoint** — returns 200 if DB pool has a live
      connection (`SELECT 1`); 503 otherwise.
- [x] **CORS layer (configurable, default deny)** — `cors_layer()`
      in `lib.rs` driven by `ORCHEXT_CORS_ALLOW_ORIGINS` (empty =
      no layer mounted). Wired in `main.rs` after the router.
- [x] **`SameSite=Lax`** on session and CSRF cookies confirmed at
      `cookies.rs:50,60`. `Strict` would break OAuth redirect flows.
- [x] **Request IDs in tracing** — `TraceLayer` in `main.rs`
      generates a fresh UUID per request and inserts it into the
      request span; every `tracing::*!` inside a handler inherits it.
- [x] **Auth rate-limit IP key extractor** — `SmartIpKeyExtractor`
      (XFF first, `ConnectInfo` fallback) wired in `auth.rs`, paired
      with `into_make_service_with_connect_info::<SocketAddr>` in
      `main.rs`. Caught after first prod deploy: signup/login 500'd
      with "Unable To Extract Key" because the default extractor
      needed `ConnectInfo` the binary wasn't attaching, and behind
      Fly the peer is the proxy anyway. Regression test
      `signup_succeeds_with_rate_limiter_enabled_and_xff` pins it.

### 8.2 Dockerfile (`crates/orchext-server/Dockerfile`)

- [x] **Cargo dependency caching** — `cargo-chef` planner+builder
      split. Workspace deps are baked into a layer that survives
      code-only changes. Drops rebuild from ~6 min to ~30 s.
- [x] **`RUST_VERSION` pinned to 1.95** to match local toolchain.
- [x] **Runtime image audit** — `debian:bookworm-slim` with only
      `ca-certificates` (TLS) + `wget` (HEALTHCHECK).
- [x] **`HEALTHCHECK` instruction** points at `/healthz`. Used by
      docker-compose and any non-Fly orchestrator; Fly ignores it
      and uses the explicit check in `fly.toml`.

### 8.3 Compose (`crates/orchext-server/docker-compose.yml`)

- [x] **Server healthcheck re-enabled** — explicit `wget` probe
      against `/healthz`, replacing the prior `disable: true`.
- [x] **`.env.example` updated** — `ORCHEXT_CORS_ALLOW_ORIGINS`
      added alongside the existing variables.
- [ ] **Document a real-deploy variant** in a comment block:
      Caddy/Traefik in front, persistent volume on a real disk,
      passwords from Docker secrets. *(Deferred — covered by this
      document for SaaS, less urgent for self-host.)*

### 8.4 Deploy artifacts (`deploy/`)

- [x] `deploy/fly/orchext-prod.toml`
- [x] `deploy/fly/orchext-test.toml`
- [x] `deploy/vercel/README.md` — points at the committed
      `apps/web/vercel.json`. Host-conditional `has` rewrites let one
      file route correctly for both prod and test projects.
- [x] `deploy/README.md` linking back to this document and listing
      first-time bring-up steps.
- [x] **`apps/web/vercel.json` security headers** — CSP
      (`default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; …;
      frame-ancestors 'none'; upgrade-insecure-requests`),
      `X-Content-Type-Options: nosniff`, `Referrer-Policy:
      strict-origin-when-cross-origin`, `Permissions-Policy`
      lockdown, and HSTS upgraded to `max-age=63072000;
      includeSubDomains; preload`. `'wasm-unsafe-eval'` is what
      keeps the orchext-crypto-wasm module compile working;
      `frame-ancestors 'none'` replaces the legacy `X-Frame-Options:
      DENY`.

### 8.5 CI (`.github/workflows/`)

- [x] **`server.yml`** — clippy + test against a Postgres 17
      service container. `--exclude orchext-desktop` because the
      Tauri crate pulls `gdk-sys` which needs `libgtk-3-dev` /
      `libwebkit2gtk-4.1-dev` (absent on `ubuntu-latest`);
      desktop has its own Phase 4 build pipeline. `cargo fmt
      --check` *not* gated yet — workspace has ~140 files of
      pre-existing rustfmt drift.
- [x] **`web.yml`** — `npm ci` + `npm run build` (which invokes
      `wasm-pack` via `prebuild`). Path-filtered so it only runs
      when `apps/web/` or the crypto crates change. The
      "verify committed wasm matches sources" step was removed
      after wasm-pack proved non-deterministic across host
      platforms (macOS-built bytes ≠ Linux-built bytes); Vercel's
      no-Rust prebuild path remains the actual gate against stale
      committed wasm.
- [ ] **Vercel auto-deploys** — production from `main`, test from
      `develop`. Configured in Vercel UI; no GH Actions needed.
      *(Done at first-deploy time; not a code-side artifact.)*
- [ ] **Fly deploy on tag** — `fly-deploy.yml` triggered on
      `release/*` tags. *(Deferred until release cadence justifies
      automation; manual `flyctl deploy` is fine until then.)*

### 8.6 Deferred follow-ups

- [ ] **Workspace fmt cleanup** — one-shot `cargo fmt --all` to clear
      ~140 files of drift, then add `cargo fmt --check` to CI.
- [ ] **Clippy warning floor to zero** — three pre-existing
      `unwrap_or_else(|_| Value::Null)` warnings in test helpers;
      fix and switch CI to `-D warnings`.

---

## 9. Cost expectations

Based on §4 of the deployment-architecture conversation; assumes
5000 registered users, ~30% DAU, ~3% concurrent peak.

| Component | Test | Prod | Notes |
|---|---|---|---|
| Vercel | $0 | $0 | Hobby tier sufficient for both. Move to Pro ($20/mo) if test needs password protection. |
| Fly.io compute | $2–3 | $7–10 | shared-cpu-1x; auto-stop on test. |
| Fly bandwidth + IPv4 | $2 | $2 | Two static IPv4 addresses. |
| Neon Postgres | $0 | $19 | Free tier or branched off prod for test; Launch tier for prod. |
| **Subtotal** | **~$5** | **~$30** | **Combined: ~$35/mo to start.** |

Growth thresholds:

| Trigger | Action | New monthly cost |
|---|---|---|
| Storage > 10 GB | Neon Scale tier | +$50 (replaces Launch) |
| Latency on cold start unacceptable | Neon disable auto-suspend | +$10–20 |
| Downtime cost > $50/mo | Neon HA replica | +$25–40 |
| Sustained > 100 req/s | Fly VM upsize to dedicated-cpu-1x | +$30 |
| Multi-region traffic | Fly second region + Neon read replica | +$40+ |

We can stay under $100/mo through ~10–15 thousand registered users
without architectural change.

---

## 10. Self-host parity

The following remain identical between SaaS and self-host:

- `crates/orchext-server/Dockerfile`
- `crates/orchext-server/docker-compose.yml`
- All migrations in `crates/orchext-server/migrations/`
- All env vars consumed by `Config::from_env`
- The full HTTP API surface (`/v1/*`, `/healthz`, `/v1/mcp`)
- The web app code in `apps/web/`

The following are **SaaS-only** and live under `deploy/`:

- `fly.toml` files
- Vercel project configuration
- Neon connection strings (in Fly secrets)
- DNS records

A self-hoster sees none of this. They run `docker compose up`, get
the API on `:8080`, build `apps/web` and put it behind any reverse
proxy or static host of their choice. The README's self-host story
is unchanged.

---

## 11. Operations

### 11.1 Backups

Neon keeps continuous WAL replication and 7-day PITR on the Launch
plan. Manual backup runbook (run quarterly to verify):

```bash
# Manual logical backup, dropped to operator's machine.
pg_dump "$NEON_PROD_DIRECT_URL" --format=custom --file=orchext-prod-$(date +%F).dump
```

Store the dump in a separate cold-storage bucket (S3 IA / Backblaze
B2) with at least 30-day retention. This is belt-and-braces against
Neon disappearing — not against operational mistakes (PITR covers
those).

### 11.2 Observability

Initially: Fly's built-in metrics + log feed, Vercel's analytics,
Neon's query insights. No third-party APM until we have a reason.

When we do add APM: prefer something that ingests OpenTelemetry —
the `tracing` crate emits OTLP cleanly, no app-side rewrite.

### 11.3 Incident handling

Single on-call (the founder, until the team grows). Failure modes:

| Symptom | First check | Likely cause |
|---|---|---|
| Web app blank / 500 on every API call | Fly app status | API down or migrations failing |
| `/v1/auth/me` returns 401 unexpectedly | Cookie domain | Misconfigured `SECURE_COOKIES` or domain mismatch |
| Slow first request after idle | Neon compute waking | Bump idle timeout |
| All API calls timing out | Neon project status | Outage on Neon's side |

Status page deferred — too few users to justify.

---

## 12. Migration paths

If a vendor goes wrong:

| From | To | Effort |
|---|---|---|
| Neon | Any other Postgres host | `pg_dump` + `pg_restore`, change `DATABASE_URL`. ~1 hour. |
| Fly.io | Render / Railway / Cloud Run | New deploy config; same Dockerfile. ~1 day. |
| Fly.io | Self-hosted (Hetzner + Caddy) | Same Dockerfile. ~1 day. |
| Vercel | Cloudflare Pages / Netlify | Static bundle; rewrites translate trivially. ~2 hours. |

The architecture is engineered so any single vendor can be replaced
without code change beyond config.

---

## 13. Deferred / out of scope

- **Public API hostname** (`api.orchext.ai`) — only needed when
  third-party MCP clients connect from outside the browser. Not yet.
- **Sentry / APM / synthetic monitors** — added when error volume
  justifies the noise floor.
- **Multi-region deploy** — single us-west region until latency or
  resilience needs make it worth the complexity.
- **Read replicas, sharding, etc.** — single Postgres until ≥50k
  users.
- **Status page** — deferred until we have customers who'd care.
- **SOC2 / formal compliance work** — separate Phase 5 concern, not
  blocking this deployment.
- **Desktop app distribution / signing / auto-update** — Phase 4
  concern.

---

## 14. Decision log

| Date | Decision | Why |
|---|---|---|
| 2026-04-26 | Vercel for SPA, Fly.io for API, Neon for DB | Cheapest viable shape that preserves self-host parity (§1, §9). |
| 2026-04-26 | Vercel rewrites instead of cross-origin API | Keeps cookies first-party; no server CORS or `SameSite=None` (§3.2). |
| 2026-04-26 | Single committed `apps/web/vercel.json` with host-conditional `has` rewrites | Earlier doc claimed `vercel.json` couldn't switch destinations per env; host-based `has` clauses do exactly that, and a committed file beats two manually-managed dashboard configs (§3.2). |
| 2026-04-26 | Neon direct (unpooled) endpoint, not pgbouncer | sqlx 0.8 has its own pool; transaction-mode pooling breaks prepared-statement caching (§5.3). |
| 2026-04-26 | Skip HA Postgres at launch | Single-node + 7-day PITR is enough until paid users or revenue depends on uptime (§9). |
| 2026-04-26 | API hostname not yet published | No third-party MCP clients yet; deferring `api.orchext.ai` keeps the cookie story simpler (§13). |
| 2026-04-26 | Region pinned to us-west (`sjc` + `aws-us-west-2`) instead of us-east | Neon project was created in `us-west-2` first; co-locating Fly avoids cross-coast latency. Reversible by changing `primary_region` in both `fly.toml`s and re-creating Neon projects. |
| 2026-04-26 | `SmartIpKeyExtractor` (XFF) for the auth rate limiter, not `PeerIpKeyExtractor` | Behind Fly the TCP peer is the proxy — `PeerIpKeyExtractor` would either 500 (no `ConnectInfo`) or rate-limit every request against a single proxy IP, which collectively locks out the world. XFF is set by Fly on every inbound; `ConnectInfo` is wired as fallback for self-host (§8.1). |
| 2026-04-26 | Drop the "verify committed wasm" CI step rather than fix it | wasm-pack output is not byte-identical across host platforms, so the committed (macOS-built) artefact will never match a CI (Linux) rebuild. Vercel's no-Rust prebuild path is the actual gate against stale wasm shipping (§8.5). |

Update this log whenever a vendor, topology, or config decision
changes. Each row is the smallest-possible "why" — if the decision
needs more, it gets a new section above and a one-line link here.
