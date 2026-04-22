# Phase 2 — Multi-vault, server, teams (plan)

Forward-looking shape of Phase 2: goals, architectural decisions, and
per-sub-milestone plans. Shipped sub-milestones are cross-referenced
out to their frozen shipped docs; live status lives in
[`../implementation-status.md`](../implementation-status.md).

**Status snapshot:** Phase 2a, 2b.1, 2b.2, and 2b.3 shipped.
**Next up:** 2b.4 (`apps/web` web client + WASM crypto) — pulled ahead
of MCP HTTP/SSE so a shareable URL lands sooner. 2b.5
(`context.propose` + MCP HTTP/SSE + OAuth 2.1 PKCE for agent tokens)
follows.

---

## Goals — six use cases

1. **Personal self-host.** Desktop app + local MCP (shipped today).
2. **Personal synced.** One user, desktop + web client, context synced
   between devices.
3. **Team self-host.** Business customer runs `mytex-server` on their
   own infra; team members connect from desktop or web. Shared
   org-level context (marketing stance, goals, tone) alongside
   each member's personal workspace.
4. **Team SaaS.** Managed multi-tenant hosting of (3). Same artifact.
5. **Account + N memberships.** A single account can belong to
   personal + any number of teams; the client switches between
   workspaces (Slack/Linear model).
6. **Agent-led code updates.** Keep crates small and independently
   testable; `implementation-status.md` is the durable handoff so
   agents can pick up mid-stream.

## Deployment matrix

|              | Self-hosted                | Managed SaaS            |
| ---          | ---                        | ---                     |
| **Personal** | Desktop-only (today's v1)  | Desktop + web, synced   |
| **Team**     | On-prem `mytex-server`     | Hosted tenant of same   |

**Key claim:** one server artifact (`mytex-server`, axum) covers the
three non-trivial cells. SaaS is "we operate it for you" — no code
fork. Already promised by `ARCHITECTURE.md` §6.

## Architectural decisions (Phase 2)

**D7. Server packaging — Docker image + `docker-compose.yml`.**
On-prem customers get a published image plus a reference compose file
(server + Postgres + TLS-terminating reverse proxy). Lets them deploy
without owning an OS or dependency stack. The SaaS tenant runs the
same image. A signed standalone binary + systemd unit is possible
later but not first.

**D8. Identity — one account, N memberships.**
A Mytex account is a single login that can belong to any number of
workspaces (one personal + N teams). Client switches workspaces
in-app. Per-workspace tokens, audit logs, scopes, and visibility
labels stay isolated; an account is just the identity envelope.

**D9. Sync / decryption — session-bound, default on.**
`reconciled-v1-plan.md` Q3 cashes in. While any client is online and
unlocked, the server holds a short-lived session key and decrypts
server-side for hosted agents. When no client is online past the TTL,
the server falls back to opaque blobs and hosted integrations see a
locked state. Strict-E2EE opt-out available; those users' hosted
agents fall back to relay-to-device.

**D10. Org context — admin-write, first user is admin.**
Team workspaces get a seed `org/` top-level type. Only admins/owners
can write to `org/*`. The first user of a new team is made admin
automatically. Members read `org/*` subject to visibility. Members
with `read+propose` can submit `context.propose` patches for admin
review — the long-deferred propose flow finally earns its keep.

**D11. Team roles — three levels, mapped to scope.**
`owner` (billing + member management + org write), `admin` (member
management + org write), `member` (read + propose). Roles translate
to default scope sets; per-workspace tokens may narrow further.
No per-document ACLs.

**D12. No CRDTs.** Server is source of truth in sync mode. Writes are
version-checked against document hash (already computed by
`mytex-vault`). Conflicts surface as last-write-wins with a UI prompt.
Multi-device offline editing is a v3 concern.

**D13. Phase 2b is split into five sub-milestones.**
`mytex-server` + `mytex-crypto` + `mytex-sync` + `apps/web` is too
much to land atomically. Order:

- **2b.1** — Server skeleton + user auth. Axum, Postgres, sessions.
  Plaintext blob storage at rest. No vault endpoints.
- **2b.2** — Server vault + index + token endpoints; `mytex-sync`
  client. Desktop gains a remote workspace. Still plaintext.
- **2b.3** — `mytex-crypto` + session-bound decryption. Retrofit
  encryption onto 2b.2's endpoints.
- **2b.4** — `apps/web` web client + WASM crypto. Pulled ahead of
  MCP HTTP/SSE: 2b.4 unblocks a shareable URL; agent-over-HTTP
  access can wait. Web client uses the same session-token login
  desktop uses today (2b.2), so no auth-protocol dependency.
- **2b.5** — `context.propose` + MCP HTTP/SSE transport. OAuth 2.1
  + PKCE for agent tokens lands here (not 2b.1, because users don't
  need it until agents hit HTTP MCP).

**D14. No managed backend (no Supabase).**
Supabase buys us ~2–3 weeks on auth flows but costs us a 6–7
container self-host stack, a heavyweight external dependency in
fast flux, and a fork between SaaS and self-host paths. The
interesting parts of Mytex (MCP protocol, session-bound decryption,
audit chain) are custom and Supabase does not help with any of them.
Self-host stays at 2 containers (server + Postgres) which is the
story we want to tell. Stack tight: `sqlx` + `argon2` + `axum` SSE +
`argon2` migrations. Industry-light, not industry-thin.

**D15. Session model — opaque server-side sessions, not JWT.**
One-click revoke is a product feature (ARCH §5.2); JWTs would need
a denylist to support that, which defeats statelessness. `mytex-auth`
already uses opaque tokens for MCP agents; user-login sessions use
the same shape (opaque `mtx_*` prefix, Argon2id-hashed at rest,
revocable). Per-request DB work is already paid by audit logging.
Single-service, single-Postgres shape has no distributed-validation
surface for JWT to win on. Federated SSO later (Google/GitHub) fits
this cleanly — we receive a JWT from the IdP and issue our own
opaque session.

**D16. Auth implementation — rolled, not a library.**
OAuth 2.1 + PKCE is ~500 lines of well-specified Rust. Pulling in
`oxide-auth` or similar adds config complexity we don't need and
couples the auth surface to a dep's opinions. `argon2` is already
a workspace dep (used in `mytex-auth`); reuse it. `sqlx` offline-
checked queries match `rusqlite` ergonomics elsewhere in the repo.

**D17. Crate layout — `crates/mytex-server`, lib + bin.**
Same shape as `mytex-mcp`: a library crate exposing a library for
tests and integration, plus a `mytex-server` binary for the docker
image. Keeps `apps/` reserved for end-user clients (desktop, later
web); servers live under `crates/`.

## Sub-milestones

### Phase 2a — Multi-vault desktop + workspace switcher **[SHIPPED 2026-04-19]**

Desktop now tracks N registered vaults and switches between them from
the header. Unblocks use case 5 locally. See
[`phase-1-core.md`](phase-1-core.md) (rolled into the `mytex-desktop`
section).

**Cuts that held:** no cross-workspace search; no "all workspaces"
view; no URL routing (Phase 2a plan proposed `/w/:id/...`, dropped —
React state is enough).

### Phase 2b.1 — Server skeleton + user auth (plaintext) **[SHIPPED 2026-04-19]**

Gets the deployment shape real before anything depends on it: axum +
Postgres + sessions + Docker/compose packaging. See
[`phase-2b1-server.md`](phase-2b1-server.md) for route list,
schema, decisions, and tests.

**Cuts that held:** no email verification, no password reset, no rate
limiting beyond axum/tower defaults. All additive in 2b.x.

### Phase 2b.2 — Vault + index endpoints, `mytex-sync` client **[SHIPPED 2026-04-19]**

Server speaks the `VaultDriver` + `Index` + token + audit surface over
HTTP; `mytex-sync` client crate adapts it back into the trait shape
desktop already uses. Desktop gains remote workspaces. See
[`phase-2b2-remote-vault.md`](phase-2b2-remote-vault.md).

**Cuts that held:** plaintext at rest (→ 2b.3); no OAuth PKCE for
agent tokens (→ 2b.5); no offline write queue; no cross-tenant search.

**Follow-up still open:** desktop "Connect to server" modal is a
~5-minute UI task; backend is wired.

### Phase 2b.3 — `mytex-crypto` + session-bound decryption **[SHIPPED 2026-04-19]**

Encryption at rest + the session-bound key-publish model from
ARCH §3.4 / Q3. See [`phase-2b3-encryption.md`](phase-2b3-encryption.md)
for the route list, decisions, and test coverage. Highlights:

- `mytex-crypto` new: Argon2id KDF + XChaCha20-Poly1305 AEAD.
- Server `/vault/crypto`, `/vault/init-crypto`, `/session-key`
  endpoints; encrypted `documents.body_ciphertext` column.
- Desktop `workspace_unlock` / `workspace_lock` commands with a
  4-minute heartbeat task to refresh the server's content key.

**2b.3 follow-ups (still open):**

- Desktop unlock modal in the React UI (backend is wired).
- OS keychain for master key caching.
- Per-doc keys + key rotation endpoint.
- Strict-E2EE opt-out flag (D9).
- FTS on encrypted rows (re-populate `tsv` during write while a key
  is live).

### Phase 2b.4 — Web client (next up)

Separate app, feature-parity with desktop's remote-workspace mode.
Pulled ahead of MCP HTTP/SSE (previously 2b.4) so a shareable URL
lands sooner; depends only on 2b.2's HTTP surface and 2b.3's crypto,
both shipped.

- **New app:** `apps/web` — Vite + React + Tailwind, mirrors
  `apps/desktop/src/` components as much as possible.
- **WASM crypto:** `mytex-crypto` compiled to wasm32 target for
  in-browser decryption. Session-key publish happens from the
  browser once the user unlocks. Add a `wasm` feature that strips
  `tokio` / `rand::thread_rng()` for the browser build.
- **Auth:** same opaque session token flow the desktop uses today
  (`/v1/auth/login` → bearer). No OAuth dependency — OAuth PKCE
  (2b.5) is for *agent* tokens, not user login.
- **No stdio MCP** (browser can't spawn processes) — hosted
  integrations go through the server's HTTP/SSE MCP from 2b.5.
  Until 2b.5 lands, web users get the same UI as desktop minus any
  "connect an agent" affordances.

### Phase 2b.5 — `context.propose` + MCP HTTP/SSE

Finally makes the server reachable by external agents over MCP.

- **MCP transport** on `mytex-server`: JSON-RPC over HTTP + SSE
  per `MCP.md` §2.2. Same tools, same error model.
- **OAuth 2.1 + PKCE** for agent token issuance — rolled (D16).
  Authorization-code + PKCE, audience-bound bearer tokens, issued
  by the logged-in user via the desktop/web UI. Opaque token shape
  (still D15) — OAuth defines the *issuance* flow, not the token
  encoding.
- **`context.propose`** lands on both surfaces. Desktop + web
  proposal review queue for admins (2c-ready).

**Unblocks after full 2b:** use case 2 end-to-end. Also a power-user
flavor of use case 1 (own server, own devices).

### Phase 2c — Teams and org context

Multi-tenant on the same server, plus team semantics.

- **Crates touched:**
  - `mytex-server` — membership table, role middleware, workspace
    routing (`/w/:id/...`), invite flow, first-user-is-admin (D10).
  - `mytex-vault` — seed `org/` type directory, `org:` visibility
    label.
  - `mytex-auth` — workspace-aware tokens, role-derived default
    scopes (D11).
  - `mytex-desktop` + `apps/web` — team management UI (members,
    invites, role change), org-context editor (admin), propose-to-org
    (member).
- **SaaS is just multi-tenant signups on the same image.**
  On-prem is the same image inside the customer's firewall.
- **Unblocks:** use cases 3 and 4.
- **Cuts:** no billing integration (out of scope per user);
  no SCIM/SAML (add if enterprise customers ask); no per-doc ACLs.

## New crates / apps summary

| Name            | Kind  | Phase   | Role                                          |
| ---             | ---   | ---     | ---                                           |
| `mytex-server`  | crate | 2b.1+   | Axum HTTP + Postgres; Docker-packaged         |
| `mytex-sync`    | crate | 2b.2    | Client-side `VaultDriver` over HTTPS          |
| `mytex-crypto`  | crate | 2b.3    | Session-bound key hierarchy; WASM-compilable  |
| `apps/web`      | app   | 2b.4    | React web client (parallel to desktop)        |

Phases 2a and 2c add no new crates — both extend existing ones.

## Scope cuts (explicit)

- No CRDTs (D12).
- No mobile app, no browser extension.
- No federation between self-hosted servers.
- No per-document ACLs — reuse `visibility` + roles.
- No billing in 2c (deferred).
- No offline-first multi-device editing — online-first, version-checked.
- No cross-workspace search in 2a.
- No SSO/SCIM/SAML in 2c initial cut.

## Open questions

Resolved (captured in decisions above):

- ✅ **DB toolchain.** `sqlx` with runtime-checked queries in 2b.1;
  migrate to `query!` + offline cache when CI has Postgres.
- ✅ **Web client tech.** Vite + React + Tailwind, mirror desktop.
- ✅ **MCP transport for team workspaces.** Both — desktop runs local
  `mytex-mcp` against `RemoteVaultDriver` (2b.2); hosted integrations
  hit the server's HTTP/SSE MCP (2b.5).
- ✅ **Managed backend?** No Supabase (D14). Self-host stays at 2
  containers.
- ✅ **Session model.** Opaque server-side, not JWT (D15).

Still open:

- **OAuth provider for SaaS.** We run our own IdP (rolled, D16) for
  Phase 2. Integrating federated SSO (Google, GitHub, Okta, WorkOS)
  and making it pluggable for self-host enterprise customers is a
  Phase 2c+ question.
- **Conflict UI (2b.2+).** Version-miss on write returns
  `version_conflict`. UX options: inline diff + pick-a-side, or
  re-open the editor with both versions for a manual merge. TBD once
  users actually hit the case.
- **Invite UX (2c).** Email magic link, shareable join-code link, or
  both. Email needs an SMTP dep and deliverability story; join link
  is simpler but more copy-paste.
- **Keychain story for session tokens.** Today the desktop stores
  both the Anthropic API key and remote session tokens in plaintext
  in `~/.mytex/`. Move to OS keychain (`keyring` crate) before any
  distribution build (Phase 3).
- **Sync cache TTL + SSE fallback.** Start at 5 s TTL + SSE
  invalidation; revisit if SSE proves flaky on hosted infra.
