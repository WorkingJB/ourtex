# Phase 3b — First external integration + visibility trust-tier (plan)

Proves the integration pipeline end-to-end with the simplest OAuth
provider (Todoist) and lands the **visibility-driven storage tier**
that every later task-aggregation feature relies on. Split into two
sub-milestones so the trust-tier infrastructure lands cleanly before
the first adapter rides on top of it.

**Prereqs:** Phase 3a (`type: task` seed, `orchext-tasks` crate).
**Independent of:** 2c teams — personal tenant is sufficient for 3b;
team aggregation is 3c.

Live status in [`../implementation-status.md`](../implementation-status.md);
follow-ups in [`phase-3c-task-expansion.md`](phase-3c-task-expansion.md).

---

## Goals

1. Server can answer structured aggregation queries ("overdue tasks
   assigned to me", "all work tasks due this week") without any
   client unlocked — but only for `visibility: work` tasks.
2. Personal-visibility tasks stay fully E2EE; server sees only a
   narrow status/due/assignee row (no title, no body).
3. A user can connect a Todoist account, classify the connection as
   work or personal, and watch their Todoist items land in an
   "Incoming" tray. One click promotes an item into the vault as a
   real `type: task` doc.
4. Integrations survive a server restart with no client online:
   OAuth refresh tokens are server-held, encrypted with a per-
   provider server key.
5. A sharing flow for personal-visibility tasks exists: sender's
   client re-wraps the content key for a named recipient; recipient
   decrypts locally when next unlocked. Server never sees plaintext.

## Architectural decisions

**D18 (reified). Visibility drives storage tier.** Tasks written with
`visibility: work` produce a `task_projection` row with plaintext
title + body + structured fields. Tasks with any non-`work` non-
`public` visibility produce a projection row with status/due/assignee
only — no title, no body. Projection writes happen alongside
encrypted body writes in `docWrite`, under a live session key. This is
a narrowing of the E2EE claim; documented explicitly in `SECURITY.md`.

**D22. Integration OAuth refresh tokens are server-trusted.** The
server must call upstream APIs at 3am with no client online. Refresh
tokens sit in `integration_credentials`, encrypted with a per-provider
**server key** (not the user's vault key). Mitigations: per-tenant
isolation, HSM-ready key wrapping, documented revocation drill. This
is an explicit, narrow E2EE carve-out — the second after D18 — and
the only one that involves server-held secrets for a user resource.

**D23. Sync is server-side by default, desktop-side for strict
E2EE.** `orchext-integrations` compiles both as a library for
`orchext-server` and as an in-process module for `apps/desktop`.
Server-side is the default (works 24/7, no client required).
Desktop-side is gated behind a feature flag for users who never want
the server to hold their upstream token; they accept the degraded
"only syncs when desktop is open" trade.

**D24. Per-connection default visibility + per-task override.**
Every integration connection carries `default_visibility: work |
personal` set at OAuth time. Synced tasks inherit the default.
Users can flip any individual task's visibility in the UI — flipping
to work copies title+body into the projection; flipping back strips
them. The visibility label is the user-facing, per-task security knob.

**D25. External task cache for un-promoted items.** Upstream items
the user hasn't promoted sit in `external_task_cache` (server-
encrypted with the same provider key as credentials). Not E2EE
because upstream already sees the data; this is a server-side read-
model, not user content. Promotion copies a snapshot into the vault
as a real `type: task` doc.

**D26. Cooperative key share for personal sharing.** Sharing a
personal-visibility task to another user rewraps the task's content
key to the recipient's public key. Uses existing `orchext-crypto`
`wrap` / `unwrap`. Offline recipients → pending share (server holds
the wrapped bundle until recipient's client next comes online and
decrypts).

---

## Sub-milestones

### Phase 3b.1 — Visibility trust-tier + projection (server-only)

Lands the tier infrastructure with **no external sync**. User-
authored tasks (from 3a) start flowing into the projection
immediately, giving the server its first aggregation surface.

**Deliverables:**

- Migration `0004_tasks.sql` adds `task_projection (tenant_id,
  doc_id, status, due, assignee, source, source_external_id,
  title_plaintext nullable, body_plaintext nullable, updated_at,
  visibility)`. `title_plaintext` + `body_plaintext` are non-null
  only for `visibility = 'work'`.
- `orchext-server` `docWrite` extended: when a written doc is
  `type: task`, write/update the projection row under the same
  transaction. Live session key required for personal-visibility
  tasks (to read the structured fields out of the plaintext body
  before encryption; these structured fields are then stored
  plaintext in the projection alongside the encrypted body).
- `GET /v1/t/:tid/tasks` — structured query endpoint. Params:
  `status`, `due_before`, `due_after`, `assignee`, `source`,
  `visibility`. Returns projection rows. Does not decrypt bodies.
- `DELETE /v1/t/:tid/tasks/:doc_id/projection-body` — strip
  plaintext title/body from projection (no-op if already stripped).
  Called by the UI when a user flips a task from work → personal.
- `orchext-mcp` extended: `task_query(status?, due_before?, ...)`.
  Server-side aggregation; works even when MCP client has a
  personal-only scope (results filtered by visibility × scope).
- `SECURITY.md` new: documents D18 and D22 as explicit carve-outs.

**Verification:**

- Integration test: create a work-visibility task via `docWrite`;
  `GET /tasks` returns full row with title+body. Flip to personal;
  body endpoint strips; `GET /tasks` returns status/due/assignee
  only.
- Integration test: with no live session key, `docWrite` for a
  personal-visibility task fails fast (same contract as today's
  encrypted writes). For work-visibility, succeeds.
- Test: cross-tenant isolation — tenant A's tasks invisible to
  tenant B's queries at every endpoint.
- DB inspection: `SELECT body_plaintext FROM task_projection WHERE
  visibility != 'work'` returns zero rows post-migration of any
  accidentally-populated data (projection writer is the sole
  inserter; no backfill).

### Phase 3b.2 — Todoist adapter + OAuth + promote-to-vault

First concrete integration. Rides on 3b.1's projection infrastructure.

**Deliverables:**

- `orchext-integrations` new crate. `TaskSourceAdapter` trait:
  - `connect(oauth_code, pkce_verifier) -> CredentialBundle`
  - `refresh(creds) -> CredentialBundle`
  - `poll(creds, since: cursor) -> (Vec<ExternalTask>, cursor)`
  - `ingest_webhook(payload, creds) -> Vec<ExternalTaskEvent>`
  - `push_status(creds, external_id, status)` (stub in 3b.2;
    two-way sync is 3c+)
  One module per provider; `todoist` is the only module landed here.
- Migration `0006_integrations.sql`: `integration_credentials
  (tenant_id, integration_doc_id, provider, encrypted_tokens,
  expires_at, last_rotated_at)`, `external_task_cache (tenant_id,
  provider, external_id, title, url, due, status, assignee,
  cached_at, UNIQUE(tenant_id, provider, external_id))`.
- `orchext-server` new routes:
  - `POST /v1/t/:tid/integrations/todoist/oauth-start` — returns
    Todoist auth URL + PKCE challenge.
  - `GET /v1/t/:tid/integrations/todoist/oauth-callback` —
    completes the exchange; writes `type: integration` doc to
    vault + credentials row.
  - `POST /v1/t/:tid/integrations/:id/webhook` — Todoist webhook
    target.
  - `GET /v1/t/:tid/incoming` — external task cache for UI tray.
  - `POST /v1/t/:tid/incoming/:external_id/promote` — writes a
    `type: task` doc; populates `source: todoist`, `source_id`,
    `visibility` from the integration's default.
- Server sync worker: `tokio` task per connected integration, 60s
  minimum poll interval, webhook-first (polls only as fallback).
  Exponential backoff on errors; credential refresh handled
  transparently.
- Strict-E2EE desktop-side variant: `orchext-integrations` imported
  by `apps/desktop` behind feature `desktop-integrations`.
  `~/.orchext/integrations.yml` holds local sync config.
- Desktop + web UI: **Connect Todoist** flow (OAuth popup) →
  Incoming tray → **Promote** button → task lands in vault with
  connection default visibility.

**Verification:**

- Integration test against Todoist sandbox (credentials in CI
  secrets; fallback to recorded-cassette mode for offline runs).
- End-to-end: connect, create task in Todoist UI, see it in
  Incoming within 60s, promote, see the task doc on disk / in
  vault, see its projection row queried via `GET /tasks`.
- Visibility default: connect as work → promoted tasks are work
  → show in projection with title+body. Connect as personal →
  promoted tasks are personal → projection has no title/body.
- Per-task override flipped via UI updates projection atomically.
- Leak-revocation drill: rotate the per-provider server key; old
  encrypted_tokens re-encrypt on next refresh; new writes use
  new key; drill documented in `SECURITY.md`.

## Cuts — explicit

- **Todoist only.** Linear / Jira / Asana / MS To Do land in 3c.
  Adapter interface is stable once 3b.2 ships.
- **Read-only sync.** Status changes in Orchext do not push back
  to Todoist yet. `push_status` is a stub. Two-way sync is 3c+.
- **No aggregation UI.** Team inbox and manager views are 3c.
  3b ships the raw `GET /tasks` endpoint and per-user task list.
- **No billing / rate-limit UX.** Todoist's limits are generous;
  revisit if they become user-visible.
- **No bulk import.** First sync paginates via `poll` — no "import
  all 5000 historic tasks" short path. Accept the slow cold start.
- **Desktop-side integration variant is feature-flagged and
  undocumented to end users until 3c finalizes it.** Ship the
  code; gate the UI.

## Open questions

- **Should promote copy the full Todoist body into the new doc, or
  keep a pointer?** Lean copy — the vault doc is then the source of
  truth and upstream edits don't silently rewrite user notes.
- **What happens if upstream deletes a task the user has already
  promoted?** Mark the doc's `source_status: orphaned` in
  frontmatter; do not auto-delete the doc. User decides.
- **Cooperative key share UX.** First cut: share by entering a
  recipient username; the UI surfaces a "pending until recipient
  online" state. Push notifications into 3e once real infra exists.
- **Server provider key rotation cadence.** Every 180 days?
  Annually? Set a default; document the process; revisit after
  first security review.
- **OAuth client registration for Todoist.** Need a Todoist
  developer account + client_id/secret provisioned in SaaS +
  bundled into self-host defaults. Document in `README.md`.
