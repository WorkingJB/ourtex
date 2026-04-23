# Phase 3c — Task-source expansion + team inbox (plan)

Generalizes 3b.2's adapter template to cover the four remaining task
systems that surfaced in customer interviews — Linear, Jira, Asana,
Microsoft To Do — and adds the **team inbox** aggregation view on top
of the `task_projection` table.

**Prereqs:** Phase 3b.2 (adapter template + `task_projection`),
Phase 2c (team tenancy + memberships + `visibility: org` label).
Either prereq slipping slips 3c.

Live status in [`../implementation-status.md`](../implementation-status.md);
related: [`phase-3b-integrations.md`](phase-3b-integrations.md).

---

## Goals

1. Four additional `TaskSourceAdapter` implementations — each a
   thin provider module consuming the trait 3b.2 defined. No
   changes to the trait itself.
2. Team-inbox view: a tenant admin or member with the right scope
   queries tasks across all members of the team tenant in one pane.
   Work-visibility tasks aggregate in full; personal-visibility
   tasks show as counts only.
3. Manager oversight views: "tasks overdue by member", "tasks
   assigned to agents (source = agent)" — all backed by the same
   projection table, no new storage.

## Architectural decisions

**D27. No new crates for 3c.** All four new providers are modules
inside `orchext-integrations`. If a provider's API surface justifies
a dedicated crate (e.g., Jira's REST + JQL + webhook setup), that is
a *later* refactor, not part of 3c.

**D28. Team-inbox is an aggregation query, not a new data store.**
Reuses `task_projection` with `tenant_id = <team_tenant_id>` and
filters by role-visible scope. Personal-visibility tasks in a team
tenant surface as a bare count by assignee (no title, no body) —
matches D18's contract.

**D29. `visibility: org` goes into projection like `work`.** 2c's
D10 introduced `org:` for admin-writeable team context. Extended to
tasks: `visibility: org` is a team-wide work label that writes
title + body into the projection so the team inbox can show them to
all team members. `visibility: work` remains an individual's work-
but-not-team-broadcast label; it appears in *their* row but not in
the team inbox by default. (Surface a "my work tasks" filter
separately.)

**D30. Two-way sync deferred, status-label push-back shipped.**
3c still does not fully two-way sync (upstream task status ↔ Orchext
task status). But `push_status` wakes up to the extent of writing a
"Claimed by agent X" label/comment back to Todoist / Linear / Jira /
Asana where their APIs permit. This is the interlock that prevents
double-work when an agent picks up an aggregated task (3e-adjacent).

---

## Sub-milestones

### Phase 3c.1 — Linear adapter + team-inbox infra

Smallest adapter after Todoist. Linear's GraphQL + webhook story is
well-documented; use its shape to finalize the trait boundary before
adding the messier providers.

**Deliverables:**

- `orchext-integrations::linear` module implementing
  `TaskSourceAdapter`.
- `GET /v1/t/:tid/inbox` — team-inbox endpoint. Query params:
  `visibility_in` (default `work,org`), `status_in`, `due_before`,
  `assignee_in`, `source_in`. Returns projection rows + personal-
  visibility counts by assignee as a sibling `private_counts` field.
- Role gate: `GET /v1/t/:tid/inbox` requires `tasks:read` scope
  (new, added to `owner`/`admin`/`member` defaults in 2c).
- Desktop + web: **Team inbox** pane, visible when the active
  tenant is `kind = team`. Sorted by due; filter chips for
  status / assignee / source.
- Manager view (admin-only filter): "tasks overdue by member" —
  group-by `assignee` on projection.

### Phase 3c.2 — Jira + Asana + MS To Do adapters

Three adapters in one phase because each is a repeat of the
3b.2 / 3c.1 template; no new architectural surface.

**Deliverables:**

- `orchext-integrations::jira` — JQL queries + webhook
  handler + OAuth 2.0 3LO flow (Atlassian-flavoured).
- `orchext-integrations::asana` — personal + team project
  scopes; webhook handler; OAuth 2.0.
- `orchext-integrations::mstodo` — Graph API + polling only
  (MS To Do lacks webhooks); OAuth 2.0 with refresh.
- Shared: connector-health endpoint (`GET /v1/t/:tid/integrations`)
  surfaces last-sync, last-error per connection.
- UI: **Integrations** settings page lists connected providers
  with per-connection default visibility flip + disconnect.

## Cuts — explicit

- **No provider-native filters in `GET /inbox`.** Orchext's query
  params are the only surface; provider-specific fields (Jira
  priority, Linear project) ride as opaque metadata in frontmatter
  and surface in the vault doc, not the team inbox.
- **No custom-field mapping UI.** Frontmatter carries everything;
  users map by convention, not config.
- **No "assign across providers" action.** Reassigning a task in
  Orchext does not push an assignment back to Jira. Users do that
  upstream. 3e revisits if it blocks orchestration.
- **MS To Do is polling-only.** Webhook support lands when
  Microsoft ships it.
- **Linear adapter assumes one workspace per connection.** Multi-
  workspace-per-token is a 3c+ follow-up.

## Open questions

- **Slack-style @mentions in task bodies.** If a task body contains
  a `@username` that maps to a team member, do we render them as
  links in the UI? First cut: no — plain text. Revisit when the
  team inbox has real usage.
- **Per-member "personal" counter privacy.** Showing "Alice: 12
  private tasks" may feel surveillance-y. Proposal: show only
  *self* counts, not per-other-member. Let admins opt-in to a
  "team hygiene" aggregate if asked.
- **Push-status label naming.** Different systems spell claims
  differently — "Claimed by agent X" / "assigned: agent:x" /
  label `@agent-x`. Each adapter picks the provider-idiomatic form.
- **Jira Data Center vs. Cloud.** Ship Cloud-only first; DC needs
  a different auth story (PAT / Application Links). Flag as a
  late-3c add if a customer asks.
