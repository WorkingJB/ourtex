# Phase 2b.1 — Server skeleton + auth (shipped)

Shipped 2026-04-19. Axum HTTP server + Postgres account/session store,
proving the deployment shape before any vault endpoints, crypto, or
MCP HTTP depend on it. Forward-looking plan context in
[`phase-2-plan.md`](phase-2-plan.md); live status in
[`../implementation-status.md`](../implementation-status.md).

---

### `orchext-server` — 2026-04-19 (Phase 2b.1)
*([Notion: server skeleton + auth](https://www.notion.so/34b47fdae49a80d7a07aca2c31db3cba))*

Axum HTTP server, Postgres-backed account + session store. Proves the
deployment shape (Docker, Postgres, migrations) before vault endpoints,
crypto, or MCP HTTP depend on it. No vault endpoints yet — those are
2b.2.

**Public API (library):**

- `router(state: AppState) -> axum::Router` — full app router,
  including `/healthz` and `/v1/auth/*`.
- `migrate(&PgPool)` — runs embedded `./migrations` against the pool.
- `AppState { db, sessions }` — handle shared across handlers.
- `sessions::SessionService` — `issue / authenticate / revoke /
  list_for_account`.
- `accounts::{signup, by_id, verify_password}` — account CRUD with
  argon2 password hashing and email normalization.
- `password::{hash, verify}` — thin Argon2id wrapper (PHC strings).

**Binary:** `orchext-server`. Reads `DATABASE_URL` and optional
`ORCHEXT_BIND` (default `0.0.0.0:8080`); runs migrations on startup,
serves traffic, shuts down cleanly on SIGINT/SIGTERM.

**Routes:**

- `GET  /healthz` — `{"ok": true}`, no auth.
- `POST /v1/auth/signup` — email + password; creates account +
  personal tenant + owner membership + first session.
- `POST /v1/auth/login` — returns a new session for valid creds.
- `GET  /v1/auth/me` — authenticated; returns current account.
- `GET  /v1/auth/sessions` — authenticated; lists caller's
  non-revoked sessions.
- `DELETE /v1/auth/logout` — authenticated; revokes the current
  session.

**Schema (`migrations/0001_initial.sql`):**

- `accounts(id, email, password, display_name, created_at, updated_at)`
  with a `lower(email)` index.
- `sessions(id, account_id, token_prefix, token_hash, label,
  created_at, expires_at, last_used_at, revoked_at)` — opaque
  `ocx_*` secret, Argon2id-hashed at rest, first-12 prefix indexed.
- `tenants(id, name, kind)` — `kind in {personal, team}`, one
  personal tenant auto-created per account.
- `memberships(tenant_id, account_id, role)` — `role in {owner,
  admin, member}`; currently only the owner row is created at signup.
  Unused beyond bootstrap until Phase 2c.

**Decisions recorded here:**

- **Runtime-checked queries, not `sqlx::query!` macros.** The macros
  need a live DB at compile time (or a prepared `.sqlx/` cache).
  Neither is set up yet. Using `sqlx::query_as::<_, StructWithFromRow>`
  gives us runtime-checked queries without compile-time infra. Tests
  (integration, against real Postgres) catch query errors. Migrate
  to `query!` once CI has Postgres + we run `cargo sqlx prepare`.
- **Enumeration-resistant auth errors.** Unknown email, wrong
  password, revoked session, expired session all map to the same
  `401 Unauthorized` with `error.tag: "unauthorized"`. The unknown-
  email branch of `verify_password` runs a dummy Argon2 verify
  against a fixed valid PHC string to keep response time roughly
  constant. A unit test (`accounts::tests::dummy_phc_parses`) pins
  the dummy so a future Argon2 upgrade can't break it silently.
- **Signup always issues a session.** A new user should not have to
  POST login immediately after signup. The signup response body
  matches the login response shape; clients treat them identically.
- **Personal tenant bootstrap is in the signup transaction.** If the
  tenant / membership insert fails, the account insert rolls back.
  Guarantees the invariant "every account has exactly one personal
  tenant they own" even under partial failure.
- **In-memory session cache, 60s TTL.** Every request hits the DB
  for session validation by default — cached for 60s after a
  successful lookup. Revocation invalidates the cache by session_id
  so `revoke → subsequent request` is immediately rejected (the test
  `logout_revokes_session` pins this). 60s is a deliberate staleness
  budget: an expired or password-changed session stays live up to
  60s after the change, which is fine for the product's threat
  model and saves one Argon2 verify per request. Cache is bounded
  at 10k entries (drops all on overflow — unsophisticated eviction
  is fine at this scale).
- **Session prefix of 12 chars** (`ocx_` + 8) for the lookup. Same
  pattern as `orchext-auth`: enough entropy in the prefix that there
  is effectively no collision risk in a single-tenant DB, indexed
  for O(1) lookup. The real verify is Argon2id against the stored
  hash.
- **`tenant_id` columns live now even though multi-tenancy isn't
  enforced** (2c). Avoids a future schema migration at the moment
  enforcement lands.
- **Runtime-only config from env.** No TOML / YAML config file. Two
  required vars (`DATABASE_URL`, optional `ORCHEXT_BIND`); anything
  else needs a code change. Keeps the deploy story tight.

**Notable tests:**

- `signup_then_me_roundtrip` — signup returns a bearer that lets
  `/v1/auth/me` return the same account.
- `login_unknown_email_indistinguishable_from_wrong_password` — pins
  the enumeration-resistance invariant. Both branches return 401
  with `error.tag: "unauthorized"`.
- `logout_revokes_session` — after DELETE /v1/auth/logout, the same
  bearer is rejected on the next request (cache invalidation works).
- `duplicate_signup_conflicts` — 409 on email re-registration,
  mapped from Postgres `23505 unique_violation`.
- `short_password_rejected` — 400 if password under 12 chars.
- `healthz_ok` — `/healthz` serves without auth.
- Plus 14 unit tests across `accounts`, `sessions`, `password`,
  `auth::bearer_from_headers`.

**Packaging:**

- `crates/orchext-server/Dockerfile` — multi-stage (rust-slim builder
  → debian-slim runtime). Runs as unprivileged user `orchext` uid 1000.
  No curl/wget baked in; healthcheck is compose's responsibility.
- `crates/orchext-server/docker-compose.yml` — spins up `postgres:16-alpine`
  + the server image built from the repo root. Dev uses
  `localhost:8080` over plain HTTP; production expects a TLS
  terminator in front.
- `crates/orchext-server/.env.example` — documented env vars
  (`ORCHEXT_POSTGRES_PASSWORD`). Not committed as `.env`.

**Known gaps after Phase 2b.1:**

- **No vault endpoints.** That's 2b.2. Today's server only does auth.
- **No email verification / password reset / rate limiting.** All
  additive; not in 2b.1's tight scope.
- **No CI Postgres.** Integration tests run locally against a
  docker-run'd Postgres (`docker run --rm -d -e POSTGRES_USER=orchext
  -e POSTGRES_PASSWORD=testpw -e POSTGRES_DB=orchext_test -p 5555:5432
  postgres:16-alpine` + `DATABASE_URL=postgres://orchext:testpw@
  localhost:5555/orchext_test`). Wiring the same into CI is a follow-
  up; currently a dev must have Docker to run the integration suite.
- **`sqlx::query!` macro migration.** Deferred until CI can run
  `cargo sqlx prepare` against a live DB and commit the `.sqlx/`
  cache. Until then, query errors surface only at runtime (caught
  by integration tests).
- **No MCP transport yet.** HTTP/SSE MCP lands with 2b.5.
- **No TLS in the reference compose file.** Plain HTTP on `:8080`.
  Production deployments add Caddy/Traefik/Nginx in front; we ship
  compose snippets for those when we publish the first image.
