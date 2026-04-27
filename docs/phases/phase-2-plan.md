# Phase 2 — Multi-vault, server, teams (plan)

Forward-looking shape of Phase 2: goals, architectural decisions, and
per-sub-milestone plans. Shipped sub-milestones are cross-referenced
out to their frozen shipped docs; live status lives in
[`../implementation-status.md`](../implementation-status.md).

**Status snapshot:** Phase 2a, 2b.1, 2b.2, 2b.3, 2b.4, and 2b.5
shipped. 2b.5 closed 2026-04-27 with `context.propose` (slice 4).
Phase 2c teams was absorbed into [Phase 3 platform](phase-3-platform.md)
on 2026-04-25 alongside the web onboarding chat and OS keychain
follow-ups; Phase 3a rebrand kicks off next.

---

## Goals — six use cases

1. **Personal self-host.** Desktop app + local MCP (shipped today).
2. **Personal synced.** One user, desktop + web client, context synced
   between devices.
3. **Team self-host.** Business customer runs `orchext-server` on their
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
| **Team**     | On-prem `orchext-server`     | Hosted tenant of same   |

**Key claim:** one server artifact (`orchext-server`, axum) covers the
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
A Orchext account is a single login that can belong to any number of
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
`orchext-vault`). Conflicts surface as last-write-wins with a UI prompt.
Multi-device offline editing is a v3 concern.

**D13. Phase 2b is split into five sub-milestones.**
`orchext-server` + `orchext-crypto` + `orchext-sync` + `apps/web` is too
much to land atomically. Order:

- **2b.1** — Server skeleton + user auth. Axum, Postgres, sessions.
  Plaintext blob storage at rest. No vault endpoints.
- **2b.2** — Server vault + index + token endpoints; `orchext-sync`
  client. Desktop gains a remote workspace. Still plaintext.
- **2b.3** — `orchext-crypto` + session-bound decryption. Retrofit
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
interesting parts of Orchext (MCP protocol, session-bound decryption,
audit chain) are custom and Supabase does not help with any of them.
Self-host stays at 2 containers (server + Postgres) which is the
story we want to tell. Stack tight: `sqlx` + `argon2` + `axum` SSE +
`argon2` migrations. Industry-light, not industry-thin.

**D15. Session model — opaque server-side sessions, not JWT.**
One-click revoke is a product feature (ARCH §5.2); JWTs would need
a denylist to support that, which defeats statelessness. `orchext-auth`
already uses opaque tokens for MCP agents; user-login sessions use
the same shape (opaque `ocx_*` prefix, Argon2id-hashed at rest,
revocable). Per-request DB work is already paid by audit logging.
Single-service, single-Postgres shape has no distributed-validation
surface for JWT to win on. Federated SSO later (Google/GitHub) fits
this cleanly — we receive a JWT from the IdP and issue our own
opaque session.

**D16. Auth implementation — rolled, not a library.**
OAuth 2.1 + PKCE is ~500 lines of well-specified Rust. Pulling in
`oxide-auth` or similar adds config complexity we don't need and
couples the auth surface to a dep's opinions. `argon2` is already
a workspace dep (used in `orchext-auth`); reuse it. `sqlx` offline-
checked queries match `rusqlite` ergonomics elsewhere in the repo.

**D17. Crate layout — `crates/orchext-server`, lib + bin.**
Same shape as `orchext-mcp`: a library crate exposing a library for
tests and integration, plus a `orchext-server` binary for the docker
image. Keeps `apps/` reserved for end-user clients (desktop, later
web); servers live under `crates/`.

## Sub-milestones

### Phase 2a — Multi-vault desktop + workspace switcher **[SHIPPED 2026-04-19]**
*([Notion](https://www.notion.so/34b47fdae49a80428509dd81db41891a))*

Desktop now tracks N registered vaults and switches between them from
the header. Unblocks use case 5 locally. See
[`phase-1-core.md`](phase-1-core.md) (rolled into the `orchext-desktop`
section).

**Cuts that held:** no cross-workspace search; no "all workspaces"
view; no URL routing (Phase 2a plan proposed `/w/:id/...`, dropped —
React state is enough).

### Phase 2b.1 — Server skeleton + user auth (plaintext) **[SHIPPED 2026-04-19]**
*([Notion](https://www.notion.so/34b47fdae49a80d7a07aca2c31db3cba))*

Gets the deployment shape real before anything depends on it: axum +
Postgres + sessions + Docker/compose packaging. See
[`phase-2b1-server.md`](phase-2b1-server.md) for route list,
schema, decisions, and tests.

**Cuts that held:** no email verification, no password reset, no rate
limiting beyond axum/tower defaults. All additive in 2b.x.

### Phase 2b.2 — Vault + index endpoints, `orchext-sync` client **[SHIPPED 2026-04-19]**
*(Notion: [vault+index endpoints](https://www.notion.so/34b47fdae49a8007b10ecec54458f25e) · [orchext-sync](https://www.notion.so/34b47fdae49a8054bd86c7de49c7dd7e) · [desktop remote registration](https://www.notion.so/34d47fdae49a81718f80f6a184b3c3fc))*

Server speaks the `VaultDriver` + `Index` + token + audit surface over
HTTP; `orchext-sync` client crate adapts it back into the trait shape
desktop already uses. Desktop gains remote workspaces. See
[`phase-2b2-remote-vault.md`](phase-2b2-remote-vault.md).

**Cuts that held:** plaintext at rest (→ 2b.3); no OAuth PKCE for
agent tokens (→ 2b.5); no offline write queue; no cross-tenant search.

**Follow-up still open:** desktop "Connect to server" modal is a
~5-minute UI task; backend is wired.

### Phase 2b.3 — `orchext-crypto` + session-bound decryption **[SHIPPED 2026-04-19]**
*(Notion: [orchext-crypto](https://www.notion.so/34b47fdae49a80adb0fac091491f0d60) · [server session-key](https://www.notion.so/34b47fdae49a80fabe34da9df833c33e) · [desktop unlock/heartbeat](https://www.notion.so/34b47fdae49a808988f3f14d8b846e9a))*

Encryption at rest + the session-bound key-publish model from
ARCH §3.4 / Q3. See [`phase-2b3-encryption.md`](phase-2b3-encryption.md)
for the route list, decisions, and test coverage. Highlights:

- `orchext-crypto` new: Argon2id KDF + XChaCha20-Poly1305 AEAD.
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

### Phase 2b.4 — Web client **[SHIPPED 2026-04-25]**
*(Notion: [web client](https://www.notion.so/34b47fdae49a806b8e86fcfb24fcdc8d) · [WASM crypto wrapper](https://www.notion.so/34d47fdae49a810f8e65f18bb9667e21) · [doc CRUD + editor](https://www.notion.so/34d47fdae49a8109a1c2f5728d76bfca) · [tokens + audit views](https://www.notion.so/34d47fdae49a81a9af78cb30a33c225b))*

Separate app, feature-parity with desktop's remote-workspace mode.
Pulled ahead of MCP HTTP/SSE so a shareable URL lands sooner;
depended only on 2b.2's HTTP surface and 2b.3's crypto, both
shipped. Detailed status + decisions live in
[`phase-2b4-web.md`](phase-2b4-web.md).

**What landed:** login / signup, tenant picker, browser unlock
(seed-fresh or unwrap-seeded) backed by `orchext-crypto-wasm`,
4-min heartbeat, doc CRUD with base-version optimistic
concurrency, tokens admin, audit list.

**Cuts that held:**

- **Graph view dropped** from both clients on 2026-04-25 — the
  desktop `GraphView.tsx` + `react-force-graph-2d` were removed,
  and the web client never adopted them.
- **Onboarding chat deferred to Phase 3 platform.** Desktop's
  flow reaches Anthropic via a Tauri-only command; web needs a
  server-mediated proxy that's better introduced alongside the
  agent-observer work. See
  [`phase-3-platform.md`](phase-3-platform.md).
- **Session token hardening rolled into 2b.5.** Web still parked
  the bearer in `localStorage` at 2b.4 close; the move to httpOnly
  cookie + CSRF is the opening slice of 2b.5.

**Unchanged commitments:**

- Auth: the opaque session token flow desktop uses today. No OAuth
  dependency — OAuth PKCE (2b.5) is for *agent* tokens.
- No stdio MCP (browser can't spawn processes). Hosted integrations
  go through 2b.5's HTTP/SSE MCP. Until then, the web UI just
  doesn't offer a "connect an agent" affordance.

### Phase 2b.5 — Auth hardening + MCP HTTP/SSE **[SHIPPED 2026-04-27]**

Four slices, in order:

1. **Web auth hardening** — opening slice. httpOnly `orchext_session`
   cookie + readable `orchext_csrf` cookie issued on login/signup;
   double-submit CSRF on cookie-authed state-changing requests;
   bearer flow preserved for desktop. Drops `localStorage` from
   `apps/web` and replaces session-bootstrapping with an
   `/v1/auth/me` probe.
   *([Notion](https://www.notion.so/34d47fdae49a81d4add7cfd2b7151ca8) — Done 2026-04-25)*
2. **OAuth 2.1 + PKCE** for agent token issuance — rolled (D16).
   Authorization-code + PKCE, audience-bound bearer tokens, issued
   by the logged-in user via the desktop/web UI. Opaque token shape
   (still D15) — OAuth defines the *issuance* flow, not the token
   encoding. **Shipped 2026-04-26 in three parts:**
   server surface (`POST /v1/oauth/authorize` session-authed,
   S256-only PKCE, loopback-or-HTTPS redirect URIs, single-use 10-min
   codes; `POST /v1/oauth/token` PKCE verify, exact redirect-uri
   match, `mcp_tokens` issuance); web consent UI (`apps/web` route
   `/oauth/authorize` parses agent-supplied params, gates on session
   auth, renders approve/deny with private-scope warning, 302s to
   `redirect_uri?code=…&state=…` on approve or
   `error=access_denied&…` on deny per RFC 6749 §4.1.2.1); and the
   agent client (`crates/orchext-oauth-client` library + `orchext-oauth`
   CLI — PKCE generation, `127.0.0.1:0` loopback, browser opener,
   callback parsing, code exchange). **Desktop consent UI deferred**
   to land alongside the installer slice (Phase 4) — needs deep-link
   plugin + per-OS `orchext://` registration that's much cheaper
   to bundle with packaged builds; the web consent works for any
   agent on any OS in the meantime.
   *([Notion](https://www.notion.so/34b47fdae49a80f8bf91d7f85aa1590c))*
3. **MCP transport** on `orchext-server`: JSON-RPC over HTTP per
   `MCP.md` §2.2. Same tools, same error model. **Shipped 2026-04-26.**
   `POST /v1/mcp` exposes initialize, ping, tools/{list,call},
   resources/{list,read}; bearer auth against the `mcp_tokens` table
   (the same row OAuth issues), so agents acquired via slice 2 finally
   have somewhere to call. Wire format reuses orchext-mcp's rpc
   envelope + error codes + tool definitions — HTTP and stdio agents
   see byte-identical JSON. SSE (`GET /v1/mcp/events`) +
   `notifications/*` (incl. `resources/subscribe`) deferred until a
   real remote MCP client appears; every current MCP client (Claude
   Desktop, Cursor, etc.) uses stdio so the SSE-driven notification
   surface has no audience today and lands when there's a driver.
   *([Notion](https://www.notion.so/34b47fdae49a80cfaf2deabe4f71c339))*
4. **`context.propose`** — **shipped 2026-04-27** in one slice across
   four surfaces. *([Notion](https://www.notion.so/34b47fdae49a8090a361ca985f9ebd6c))*
   - **Server schema + MCP tool** — migration `0006_proposals.sql`
     adds the `proposals` table; `context_propose` available on both
     stdio (`crates/orchext-mcp`) and HTTP (`crates/orchext-server`).
     Mode-gated on `read_propose`; `proposals_disabled` for read-only
     tokens. Best-effort `version_conflict` at propose time;
     authoritative re-check inside the approve transaction.
   - **Server review endpoints** — admin-gated under
     `/v1/t/:tid/proposals*`. Approve applies the patch under
     base-version optimistic concurrency, re-encrypts under the live
     session key when the row was encrypted, bumps `documents.version`,
     and audit-logs `proposal.approve` / `proposal.reject`. Patch
     model: shallow frontmatter merge (`null` clears) + exactly-zero-
     or-one body op (`body_replace` / `body_append`).
   - **Web review UI** — `/proposals` pane in `apps/web` with status
     filter tabs, frontmatter + body diff preview, approve / reject
     buttons that surface `version_conflict` inline.
   - **Desktop review UI** — same pane in `apps/desktop`. Unified DTO
     across local + remote backends so the React side renders
     identically. Local workspaces back the queue by reading the
     `.orchext/proposals/<id>.json` files dropped by stdio
     `orchext-mcp`; remote workspaces hit the new server endpoints
     via a new `crates/orchext-sync::proposals` module.

**Unblocks after full 2b:** use case 2 end-to-end. Also a power-user
flavor of use case 1 (own server, own devices).

### Phase 2c — Teams and org context **[MOVED to Phase 3 platform 2026-04-25]**

Originally scoped to land before Phase 3a. Folded into
[`phase-3-platform.md`](phase-3-platform.md) along with the web
onboarding chat and OS keychain follow-ups, because none of these
items block 2b.5 and bundling them keeps the rebrand sweep clean.
Decisions D10 and D11 carry over verbatim. Same scope, same crates
(`orchext-server`, `orchext-vault`, `orchext-auth`, `apps/desktop`,
`apps/web`); same cuts (no billing, no SCIM/SAML, no per-doc ACLs).

## New crates / apps summary

| Name                  | Kind  | Phase   | Role                                          |
| ---                   | ---   | ---     | ---                                           |
| `orchext-server`       | crate | 2b.1+   | Axum HTTP + Postgres; Docker-packaged         |
| `orchext-sync`         | crate | 2b.2    | Client-side `VaultDriver` over HTTPS          |
| `orchext-crypto`       | crate | 2b.3    | Session-bound key hierarchy; WASM-compilable  |
| `orchext-crypto-wasm`  | crate | 2b.4    | wasm-bindgen wrapper; browser crypto surface  |
| `apps/web`            | app   | 2b.4    | React web client (parallel to desktop)        |

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
  `orchext-mcp` against `RemoteVaultDriver` (2b.2); hosted integrations
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
  in `~/.orchext/`. Move to OS keychain (`keyring` crate) before any
  distribution build (Phase 4 — renumbered from Phase 3 on 2026-04-22
  when Phase 3 absorbed the orchext rebrand + capability expansion).
- **Web session token storage.** `apps/web` currently parks the
  bearer in `localStorage` — XSS-vulnerable. Move to an httpOnly
  cookie issued by `/v1/auth/login` when 2b.5 reworks the auth
  surface for OAuth PKCE + CSRF anyway.
- **Sync cache TTL + SSE fallback.** Start at 5 s TTL + SSE
  invalidation; revisit if SSE proves flaky on hosted infra.
