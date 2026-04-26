# Phase 3 platform — teams, onboarding, keychain (planned)

The platform-foundation slice of Phase 3. Bundles the work that
got pushed out of Phase 2 once 2b.5 narrowed: team workspaces +
invites (formerly Phase 2c), web onboarding chat (formerly 2b.4
follow-up), and OS keychain caching for the desktop app
(formerly 2b.3 follow-up). Sits between 2b.5 wrap and Phase 3a
(rebrand + tasks), because the rebrand sweep should not interleave
with active feature work and these three items are gating the
team-SaaS use cases.

**Starts when:** Phase 2b.5 closes (cookie/CSRF auth + OAuth PKCE
+ MCP HTTP/SSE + `context.propose` all landed).

**Prereqs:** 2b.5 complete. Cookie auth in particular gives us
the foundation for invite-link redemption flows.

Live status in [`../implementation-status.md`](../implementation-status.md);
forward scope continues in
[`phase-3a-rebrand-tasks.md`](phase-3a-rebrand-tasks.md).

---

## Goals

1. **Teams + invites.** Owner can create a team workspace, invite
   members by email or join code, and assign roles (owner / admin
   / member). Members switch between personal + N teams in the
   same client.
2. **Web onboarding chat.** Web client gets parity with desktop's
   `OnboardingView.tsx` — an Anthropic-mediated conversation that
   seeds the new account's vault. Requires a server-side chat
   route since the browser can't hold an Anthropic API key.
3. **OS keychain — desktop.** Replace plaintext storage of the
   desktop's Anthropic API key and remote session tokens in
   `~/.orchext/` with the OS keychain (`keyring` crate). Required
   before any Phase 4 distribution build.

## What was originally elsewhere

| Item | Was | Moved here on |
|---|---|---|
| Team workspaces (membership, roles, routing) | Phase 2c | 2026-04-25 |
| Invite flow (join code or email link) | Phase 2c | 2026-04-25 |
| `org/` seed type + `org:` visibility | Phase 2c | 2026-04-25 |
| Web onboarding chat | 2b.4 follow-up | 2026-04-25 |
| OS keychain (desktop) | 2b.3 follow-up | 2026-04-25 |

The decisions and scope around teams (D10, D11) carry over
unchanged from `phase-2-plan.md` — they are reproduced below for
convenience but are not new.

## Architectural decisions (reproduced)

**D10. Org context — admin-write, first user is admin.** Team
workspaces get a seed `org/` top-level type. Only admins/owners
can write to `org/*`. The first user of a new team is made admin
automatically. Members read `org/*` subject to visibility.
Members with `read+propose` can submit `context.propose` patches
for admin review.

**D11. Team roles — three levels, mapped to scope.** `owner`
(billing + member management + org write), `admin` (member
management + org write), `member` (read + propose). Roles
translate to default scope sets; per-workspace tokens may narrow
further. No per-document ACLs.

## Architectural decisions (new)

**D17a. Invite-link redemption, not email-first.** First cut uses
shareable join codes (UUID v4 in URL fragment, server-recorded
with a TTL + role + tenant). Email delivery requires SMTP +
deliverability story; defer until customers ask for it. Codes
are single-use and expire (default 7 days).

**D17b. Onboarding chat goes through the server, not direct.**
Web has no Tauri-equivalent escape hatch for an Anthropic key.
Server adds `POST /v1/onboarding/chat` and `POST /v1/onboarding/finalize`
which proxy to Anthropic with a server-held key. Falls into the
same shape we'll need for Phase 3d agent observer anyway, so
this earns its keep beyond onboarding.

## Deliverables

### Teams + invites
*(Notion: [Team workspaces + memberships](https://www.notion.so/34b47fdae49a80a09100d7e9ec10afe8) · [Seed `org/` type + visibility](https://www.notion.so/34b47fdae49a80f3aa60c780298ebe07) · [Team management UI](https://www.notion.so/34b47fdae49a8033bec2e5f0a2eeaf33))*

- **Server**
  - `POST /v1/tenants` — create a team workspace (caller becomes
    owner). Personal tenants are still auto-created at signup.
  - `POST /v1/t/:tid/invites` — issue a join code (admin/owner).
  - `GET /v1/t/:tid/invites` — list active invites (admin/owner).
  - `DELETE /v1/t/:tid/invites/:id` — revoke (admin/owner).
  - `POST /v1/invites/:code/accept` — redeem code; creates a
    membership row at the encoded role.
  - `GET /v1/t/:tid/members`, `PATCH /v1/t/:tid/members/:account_id`,
    `DELETE /v1/t/:tid/members/:account_id` — list / role-change
    / remove (admin/owner).
  - Role middleware: enforce admin-write on `org/*` doc paths.
  - New migration for `invites` table (code prefix + Argon2id
    hash, role, tenant_id, expires_at, redeemed_at).
- **Desktop + web**
  - "Create team" entry on the workspace switcher / tenant
    picker.
  - Members pane (admin/owner only): list, change role, remove,
    issue invite.
  - Invite-redemption page: `/invite/:code` route (web) or
    paste-code modal (desktop).
- **Crates touched:** `orchext-server`, `orchext-vault` (org seed
  type), `orchext-auth` (role-derived scopes already plumbed via
  `TenantContext::is_admin`), `apps/desktop`, `apps/web`.

### Web onboarding chat
*([Notion](https://www.notion.so/34d47fdae49a81d6a012e90cbbcb0d0b))*

- **Server**
  - `POST /v1/onboarding/chat` — proxy a turn to Anthropic;
    server holds `ANTHROPIC_API_KEY` env var.
  - `POST /v1/onboarding/finalize` — single-shot Claude call
    that turns the chat into seed `OnboardingSeedDoc[]`.
- **Web**
  - `OnboardingView.tsx` mirroring desktop's flow.
  - Wire into the `LoginView → TenantPicker → Onboarding (if
    empty) → Documents` post-login state machine.

### OS keychain
*([Notion](https://www.notion.so/34d47fdae49a819c8ce9dd6511989596))*

- **Desktop** (`orchext-desktop` crate)
  - Replace `~/.orchext/anthropic_key` plaintext with `keyring`
    crate writes (per-user, per-host).
  - Replace remote-workspace session token storage in
    `workspaces.json` with keyring-backed lookup keyed by
    workspace id; the JSON file keeps id + name + URL but not
    the secret.
  - Migration: on first run, if a plaintext key exists, move it
    into the keychain and delete the plaintext copy. Log the
    migration to stderr.
- **Crates touched:** `orchext-desktop`. No server changes.

## Cuts — explicit

- **No SCIM / SAML / SSO.** Email + join code only. Federated
  IdP (Google / GitHub / Okta / WorkOS) tracked as Phase 2c+
  question — re-evaluate when first enterprise customer asks.
- **No billing.** Team count and seat count are uncapped; pricing
  is a SaaS-launch decision, not a feature.
- **No per-document ACLs.** `visibility` + roles still cover it.
- **No invite expiry editing.** Set at issuance, can revoke; can't
  extend in place.
- **No org-level audit log split.** Audit chain stays per-tenant
  and contains both personal and team events for that tenant.

## Open questions

- **Email path eventually.** SMTP provider (SES, Postmark, …) is
  a Phase 4 / GA decision.
- **Member display.** Show email or display name in the members
  pane? Display name is friendlier; email is unambiguous for
  invite reconciliation. Probably both, with display name primary.
- **Web onboarding rate limit.** Anthropic costs are real once
  the server is the proxy. Soft per-account daily cap (TBD).
