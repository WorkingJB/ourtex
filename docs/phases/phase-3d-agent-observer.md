# Phase 3d — Agent sessions (observer-only) (plan)

First slice of agent orchestration: users can **see** what their
agents are doing — session list, live status, heartbeat trail,
transcript — across devices. No ticket checkout, no approvals, no
skill injection; those come in 3e. Observer-only is the smallest
slice that's independently useful and doesn't require the full
orchestrator state machine.

**Prereqs:** Phase 2b.5 (MCP HTTP/SSE + OAuth PKCE — agents now
connect over HTTP with audience-bound tokens). Phase 3a (vault
structure). Independent of 3b/3c, but shares the server key
infrastructure from 3b.2 (encrypted agent transcripts).

Live status in [`../implementation-status.md`](../implementation-status.md);
follow-up in [`phase-3e-orchestration.md`](phase-3e-orchestration.md).

---

## Goals

1. An AI agent (Claude Code locally-spawned, Cursor connecting over
   MCP, or an HTTP-webhook agent) can register itself with Orchext
   and publish heartbeat events as it works.
2. User sees live agent status in desktop + web (activity pane):
   which agents are running, what they're working on, token spend,
   last heartbeat age, whether they're blocked.
3. Agent transcripts are persisted, **client-encrypted** before
   upload (same shape as vault bodies); viewable only while a
   session key is live.
4. When a session ends, the user can **promote a summary** into the
   vault as a `type: memory` or `type: decision` doc — the durable
   artifact.
5. Full audit trail: every agent event lands in `orchext-audit`'s
   existing hash-chained JSONL log.

## Architectural decisions

**D31. Agent sessions are server-first, never vault documents.**
High-churn (heartbeat every N seconds), multi-observer, short-lived.
Markdown + frontmatter is the wrong shape. They get dedicated
Postgres tables; only *summaries* (user-approved, post-session) flow
into the vault.

**D32. Three adapter families.** `orchext-agents` exposes an
`AgentAdapter` trait with three concrete families:

- **Local-spawned** — desktop spawns Claude Code / shell / codex as
  a child process, pipes stdio, emits events over Tauri IPC.
  Transcript stays local unless the user toggles cloud-mirror.
- **External-pull** — the IDE (Cursor, Zed, custom MCP client) asks
  Orchext for work via MCP `task_checkout` (shipped in 3e; until
  then just reports status), posts heartbeats via a new MCP tool.
  Orchext doesn't own the process.
- **HTTP-webhook** — Orchext issues a signed outbound webhook with
  a session id + goal; the remote agent POSTs back heartbeats
  + final result.

**D33. Transcripts are client-encrypted, server-decryptable while
a session key is live.** Reuses `orchext-crypto` `wrap`/`unwrap`.
New column `agent_session_ciphertext` on `agent_sessions`. Team
observers (in 3e) can read the transcript only if at least one
team-member client has published a session key recently — same
trust envelope as vault document bodies. This is not a new E2EE
carve-out; it's the existing envelope applied to transcripts.

**D34. Heartbeat via MCP, not a bespoke transport.** New MCP tool
`agent_heartbeat(session_id, status, tokens_in, tokens_out,
cost_cents, note?)` reuses the existing MCP scope evaluator +
rate limiter. Push-preferred; a pull-mode `agent_status` is the
fallback for callers that can't keep a connection alive.

**D35. Cost normalization — dual track.** Budget ledger stores both
provider-native counters (Anthropic tokens, OpenAI tokens, shell
minutes) **and** a normalized `cost_cents` field. Reports and
budget limits use `cost_cents`; audit / chargeback uses raw.

---

## Deliverables

- `orchext-agents` new crate. Defines `AgentAdapter`, `Agent`,
  `AgentSession`, `HeartbeatEvent`, `BudgetLedger`. Ships one
  adapter module: `local_spawned::claude_code` (spawns
  `claude-code` CLI, parses its JSONL stream). External-pull and
  HTTP-webhook families have trait impls but no concrete provider
  in 3d beyond a generic webhook receiver.
  *(Notion: [orchext-agents crate + trait](https://www.notion.so/34d47fdae49a81d4bdd4d941d0d974fa) · [Claude Code local adapter](https://www.notion.so/34d47fdae49a819baa8ff7668383d52a))*
- Migration `0005_agents.sql`:
  - `agent_sessions (id, tenant_id, adapter_family, adapter_id,
     started_at, ended_at nullable, status, goal_text, shared,
     agent_session_ciphertext bytea, cost_cents, tokens_native jsonb)`
  - `agent_heartbeats (session_id, at, status, tokens_in,
     tokens_out, cost_cents_delta, note)`
  - `agent_events (session_id, at, kind, payload jsonb)` —
     everything that isn't a heartbeat (tool call, error, ask-user,
     blocked).
  *([Notion](https://www.notion.so/34d47fdae49a8173b814cf78f7acce37))*
- `orchext-server` new routes (sessions, end, list, detail, SSE,
  webhook receiver) and `orchext-mcp` extended with
  `agent_heartbeat`, `agent_status`, `agent_event`. Heartbeat
  events appended to `orchext-audit`'s existing hash chain.
  *([Notion](https://www.notion.so/34d47fdae49a81c68b0cd656981f27a9))*
- Transcript ciphertext stored client-encrypted on
  `agent_sessions.agent_session_ciphertext` (D33).
  *([Notion](https://www.notion.so/34d47fdae49a8152b061eeb25afff98a))*
- Cost ledger + normalization (provider-native + `cost_cents`).
  *([Notion](https://www.notion.so/34d47fdae49a8171ba6cd316afb62753))*
- Desktop + Web **Activity** pane: live list of sessions across
  all workspaces, expandable rows with recent events; SSE-fed.
  *([Notion](https://www.notion.so/34d47fdae49a81a0a466f0995a7e20f6))*
- **Summary-promote** flow: end a session → UI prompts "save what
  was learned?" → user edits a draft summary → writes a vault doc
  with `type: memory` (defaults) or `type: decision`, linking
  back to the session id.

## Verification

- Unit tests on `orchext-agents` trait invariants (session state
  machine, transcript encryption round-trip).
- Integration test: spawn a local Claude Code agent via the
  desktop adapter; watch heartbeats land in `agent_heartbeats`;
  transcript ciphertext grows; two-browser test sees SSE-fed
  events in a second tab within 2s.
- End-to-end: session ends → promote summary → `type: memory`
  doc exists in vault with `linked_session_id` frontmatter.
- Kill -9 the spawned agent: session finalizes with
  `status = crashed` after 2 missed heartbeats.
- Cost ledger: normalized `cost_cents` matches provider-native
  tokens × configured price within floating-point tolerance.

## Cuts — explicit

- **No ticket checkout, no task assignment to agent.** Agents can
  declare what they're working on via `goal_text` free-form;
  structured task linkage is 3e.
- **No HITL approvals.** Agents run autonomously; users observe
  only. 3e adds approval gates.
- **No skill injection.** Skills list exists (3a), injection is 3e.
- **No agent-to-agent handoff.** Each session is independent.
- **No shared agents across team members.** `shared` column
  exists but is not flipped on yet; the sharing UX + M-of-N team
  session key story is 3e.3.
- **No mobile live-view.** Activity pane is desktop + web only.
- **Transcript size cap unenforced in 3d.** Expect long sessions to
  bloat rows; add a chunking strategy in 3e if it hurts.

## Open questions

- **Claude Code stdio schema stability.** If Anthropic changes the
  JSONL event shape, the `local_spawned::claude_code` adapter
  breaks. Version-pin? Fall back to a best-effort parser that
  treats unknown events as opaque?
- **Webhook-receiver URL provisioning for HTTP-webhook adapters.**
  SaaS has a public hostname; self-host does not by default. Ship
  a tunneling recipe (Tailscale, Cloudflare Tunnel) in
  `README.md`.
- **Budget enforcement hard-stop.** 3d ships ledger + reporting
  only; 3e decides whether a budget-exhausted session is terminated
  or just warned.
- **"Sensitive" events in audit chain.** An agent's tool-call
  payload might contain private data the user didn't intend to
  persist. Encrypt event payloads the same way transcripts are
  encrypted? Leaning yes — add `payload_ciphertext` alongside
  `payload jsonb` and choose per event kind.
- **Multi-device live-view of the same session.** Two of the
  user's own devices both SSE-subscribe — do they see the same
  stream? Yes, by session_id; already in scope of the endpoint.
