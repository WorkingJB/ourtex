# Phase 3e — Orchestration, HITL, shared agents (plan)

Turns Orchext from an observer of agents (3d) into an **orchestrator**
of them. Atomic task checkout, approval gates, runtime skill
injection, goal-ancestry chains, and team-shared agents. The largest
phase in the Phase 3 arc; split into three sub-milestones so each
lands independently.

**Prereqs:** Phase 3d (agent sessions + heartbeats), Phase 3a
(`type: skill`, `type: task`), Phase 2b.5 (`context.propose` —
HITL rides on the propose flow).

Live status in [`../implementation-status.md`](../implementation-status.md).

---

## Goals

1. Many agents cannot double-claim the same task — **atomic
   checkout** at the server.
2. A task carries its **goal ancestry** — why the agent is working
   on this, up the chain to the user's top-level goal.
3. Agents can request **HITL approval** before taking an action;
   the right human sees it, approves or denies, and the agent
   continues or aborts deterministically.
4. Skills authored in the vault (3a) are **injected at session
   start** into compatible agent runtimes.
5. Team members can share an agent session — observers see the
   live stream, leads can intervene (pause, reassign, terminate).

## Architectural decisions

**D36. `orchext-orchestrator` is a server-only crate.** The state
machine (checkout, approval, release) is the coordination point.
Running it in the desktop would break multi-device orchestration and
team-shared sessions. Desktop/web are clients of the server
orchestrator via MCP + HTTP.

**D37. Atomic checkout via `SELECT … FOR UPDATE SKIP LOCKED`.**
One SQL-native primitive handles the race. No Redis lock, no
external coordination service. Acceptable throughput ceiling —
revisit if a single tenant ever runs thousands of concurrent
agents (it won't).

**D38. Goal ancestry is a wikilink chain with a depth cap.**
A task's `goal:` frontmatter points to a `type: goal` doc, which
may itself have a `parent_goal:` link. Orchestrator traverses up to
5 levels (configurable per tenant). Unbounded would trash the UI
and tempt people into designing hierarchies the product isn't built
for.

**D39. HITL approvals ride `context.propose`.** 2b.5 built the
propose plumbing for writes. Reuse it for agent actions: an
agent's "may I run this command" is a proposal; a user's approval
is the existing approval endpoint; the orchestrator reads the
proposal state to gate the next step. One surface, not two.

**D40. Skill injection happens server-side at session start.**
Before the first heartbeat, the orchestrator reads the vault for
skills where `runtimes` includes the session's adapter and
visibility is reachable, concatenates their bodies in `version`-
order (newest first), and passes them as initial context via the
adapter's `start_session` hook.

**D41. Shared team agent session key — M-of-N client publish.**
A team-shared session needs the server to decrypt its transcript
for live team-observer fan-out. If only one member's client
currently has a session key published, the server can decrypt. For
always-on teams, optionally publish a **team session key** that
any member's unlock publishes into a keychain slot — the server
treats any one member's key as sufficient. No new crypto; this is
a key-storage-shape change.

**D42. `orchestrator:manage` — new role-scoped permission.**
Lets admins pause, reassign, terminate any session in their
tenant. Added to `owner` + `admin` defaults in the role→scope
table (2c). Not a new role.

---

## Sub-milestones

### Phase 3e.1 — Orchestrator crate + atomic checkout + goal ancestry
*(Notion: [orchestrator crate + checkout](https://www.notion.so/34d47fdae49a818abb69d3f52f6a2a3d) · [goal ancestry traversal](https://www.notion.so/34d47fdae49a81b48113e6d85ab5c4ea) · [claimed-task label push-back](https://www.notion.so/34d47fdae49a811995f5f1c28bceb2b1))*

Lands the foundation. Agents can claim tasks, see the why, report
done. No approvals yet.

**Deliverables:**

- `orchext-orchestrator` new crate. Ticket queue,
  checkout/release state machine, goal-ancestry traversal against
  `orchext-vault`.
- Migration `0007_orchestrator.sql`: `task_checkouts (doc_id,
  tenant_id, session_id, claimed_at, released_at nullable,
  result)`.
- `orchext-mcp` extended: `task_checkout(query, goal?) -> task`
  (atomic, one-winner), `task_release(task_id, result)`,
  `goal_chain(task_id)` (returns the ancestry).
- Integration: when a session `task_checkout`s a task, the task's
  projection row gains `assignee: agent:<session_id>` and a
  "Claimed by agent X" label pushes back to the upstream source
  (via 3c.2's `push_status`) where the API permits.

### Phase 3e.2 — HITL approvals + runtime skill injection
*(Notion: [HITL approvals via context.propose](https://www.notion.so/34d47fdae49a8104aa50dbb69b0d458b) · [Approvals queue UI](https://www.notion.so/34d47fdae49a817b8c66fa97a3a7ad9f) · [skill injection at session start](https://www.notion.so/34d47fdae49a81c69d1cc8d72149b209) · [MCP skill_list tool](https://www.notion.so/34d47fdae49a81938f7cc5e15adb6816))*

The human-in-the-loop surface + skills actually flowing into
agents.

**Deliverables:**

- `orchext-orchestrator` extended: approval-gate state machine
  (propose → pending → approved / denied → release). Wraps
  2b.5's `context.propose` rather than parallel storage.
- Approval queue endpoint: `GET /v1/t/:tid/approvals` + desktop +
  web **Approvals** pane. Single-click approve / deny. Quorum
  rules: admin approval > member approval > own proposal cannot
  self-approve.
- `orchext-orchestrator` skill-injection: reads vault at session
  start via existing `VaultDriver`; filters by `runtimes` and
  scope; concatenates in version order; delivers to adapter.
- Adapter change: `AgentAdapter::start_session(ctx,
  initial_skills)` — 3d's no-arg start becomes a two-arg start.
- MCP: `skill_list(runtime)` — for manual browsing. Injection is
  implicit; this tool is for the user to verify what's in-context.

### Phase 3e.3 — Shared agents + team observer + team session key
*(Notion: [Shared agent sessions](https://www.notion.so/34d47fdae49a81c9afbee67244881eb1) · [Team session key keychain slot](https://www.notion.so/34d47fdae49a81919048d52fecd383b6) · [orchestrator:manage scope](https://www.notion.so/34d47fdae49a81368f2af140cd9141aa) · [Board oversight view](https://www.notion.so/34d47fdae49a81b8a1f0c7ed7a0f8400))*

The team story. Depends on 3e.2's HITL surface and Phase 2c
teams.

**Deliverables:**

- `agent_sessions.shared` used: a session started under a team
  tenant with `shared: true` appears in every member's Activity
  pane with the right scope.
- Team session key keychain slot in `session_keys.rs`: if any
  team member's client publishes a `team_transcript_key`, the
  server uses it for transcript decryption. Otherwise falls back
  to the session starter's own key.
- `orchestrator:manage` role-scoped permission wired: pause,
  reassign, terminate endpoints + UI controls for admins.
- **Board oversight view:** desktop + web. Shows all live
  sessions in the team tenant as cards — status, cost, goal,
  recent activity. Click a card for the full session view with
  admin actions.
- Delegation audit: every admin action (reassign, pause,
  terminate) appends a row to the existing audit chain.

## Verification

- **Atomicity:** two MCP clients race the same `task_checkout`
  query; exactly one gets the task, the other gets
  `{"err":"already_claimed"}` deterministically. Load test: 100
  concurrent claims on a 20-task pool → exactly 20 wins.
- **Goal ancestry:** `goal_chain(task_id)` returns up to 5
  ancestors; cycles caught and reported as `{"err":"cycle"}`.
- **HITL:** agent issues `context.propose` for a file write;
  admin approves in the Approvals pane; agent's next MCP call
  sees `proposal_state: approved` and proceeds. Denied path
  aborts the session with `status: aborted_denied`.
- **Skill injection:** session start for a `cursor` adapter does
  not include a skill with `runtimes: [claude-code]`; does
  include one with `runtimes: [cursor]` or `runtimes:
  [claude-code, cursor]`. Skill body appears in the initial
  context of the agent session.
- **Shared agent team observer:** member A starts a shared
  session; member B's Activity pane sees it within 2s (SSE);
  admin C on an admin account can click "terminate" and the
  session ends within one heartbeat.
- **Team session key fallback:** offline the originating member;
  another member's client was previously unlocked → transcript
  remains decryptable server-side. No team member unlocked past
  TTL → transcript goes opaque (expected; documented).

## Cuts — explicit

- **No agent-to-agent handoff.** Sessions stay independent.
  Cross-session coordination is a 3e follow-up or Phase 4+.
- **No multi-region orchestrator.** Single Postgres instance per
  deployment.
- **No approval templates / auto-approve rules.** Every
  proposal is a discrete manual action. Rule engines are a
  post-3e request-driven extension.
- **No budget hard-stop integrated with HITL.** Session ends on
  budget exhaustion via 3d's ledger; 3e does not convert
  budget-exhaust into an approval request ("may I have more").
  Would be a natural 3e.4 follow-up.
- **No skill marketplace.** Skills live in the user's vault only.
  Paperclip-style portable org templates (Clipmart) are out.
- **Board oversight view mobile parity** — desktop + web only,
  not mobile-responsive.

## Open questions

- **Proposal expiry.** How long can an approval sit pending
  before it auto-denies? Default 24h?
- **Reassignment semantics.** If an admin reassigns a live
  session to a different agent adapter, does the new agent
  resume the old transcript or start clean? Leaning clean-start
  with the old transcript linked as context.
- **Skill version pinning at session start.** If a skill bumps
  version mid-session, does the in-progress agent get the new
  body? Lean no — snapshot at start, use new version only at
  next session.
- **Team session key trust surface.** Any admin member's
  unlocked client grants server decrypt. Is that the right
  ceiling, or should it be quorum (M-of-N)? M-of-N is more
  secure but operationally noisy for a five-person team. Ship
  single-key-any-admin; revisit if a customer asks.
- **External-system write-back on orchestrator actions.** If
  the orchestrator terminates a session that had pushed a
  "Claimed by agent X" label to Jira, does Orchext also strip
  the label? Should; track as an outbound event in 3e.3.
