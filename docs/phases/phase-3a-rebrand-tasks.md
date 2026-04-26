# Phase 3a — Rebrand to Orchext + vault-native tasks & skills (plan)

Kicks off Phase 3 on the new name: `orchext` → `orchext` (orchestration +
context) in one clean sweep, and lands `type: task` and `type: skill` as
first-class vault seed types so users can author tasks and skills by
hand before any external integration exists. Pulled to 3a because
(a) the rebrand can't wait past 2b.4, (b) tasks-in-the-vault is the
smallest slice of the expanded scope that is independently useful.

**Starts when:** Phase 2b.4 (web writes, tokens, audit parity) has
shipped and the tree is quiet. Rebrand churn should not interleave
with in-flight UI work.

**Prereqs:** none beyond 2b.4. Does **not** depend on 2b.5 or 2c.

Live status in [`../implementation-status.md`](../implementation-status.md);
forward scope in [`phase-2-plan.md`](phase-2-plan.md) (Phase 2 goals +
decisions D7–D17) and [`phase-3b-integrations.md`](phase-3b-integrations.md)
(first external adapter).

---

## Goals

1. Every `orchext-*` identifier (crate, env var, disk directory, token
   prefix, bundle ID, package name, URL) renames to `orchext-*` /
   `ORCHEXT_*` / `.orchext` / `ocx_*` / `app.orchext.desktop` /
   `orchext-web`, in one feature branch.
2. `FORMAT.md` bumps to v0.2 with four new seed types: `task`, `skill`,
   `integration`, `agent`. (The `org/` seed + `org:` visibility from
   2c's D10/D11 also land in v0.2 if 2c has shipped; otherwise they
   can come in a v0.2.1 bump with 2c.)
3. Desktop + web gain a **Tasks** view that lists and opens
   `type: task` docs in the active workspace. No external sync yet.
4. Desktop gains a **Skills** view that lists `type: skill` docs;
   skill *injection into agent sessions* is deferred to 3e.
5. `orchext-mcp` exposes three new tools: `task_list`, `task_get`,
   `skill_get`. All read-only; writes come through the existing
   `docWrite` path since tasks are just docs.

## Architectural decisions

**D18. Visibility is the storage-tier selector.** Introduced here as
context for everything 3b+ does. Tasks authored in 3a sit in the vault
at whatever visibility the user picks — nothing server-side leaks yet
because there is no server projection table until 3b.1. Documenting
the rule here so that 3a's vault-native task doesn't become a
precedent for "tasks are E2EE-only forever."

**D19. Tasks are vault documents, not a parallel store.** A task is a
markdown doc with `type: task` in the YAML frontmatter. The body is
free-form (notes, acceptance criteria, ancestry). No task table at
this phase. This keeps everything indexable by the existing FTS
pipeline, round-trippable to Obsidian, and scope/visibility-evaluated
by the same code path as every other doc.

**D20. Skills are vault documents with runtime gating frontmatter.**
`type: skill`, body = instructions, frontmatter carries `runtimes:
[claude-code, cursor, ...]`, `version`, optional `inputs` / `outputs`.
Gating is a correctness check, not a security feature — the
orchestrator in 3e refuses to inject a skill whose declared runtimes
don't include the session's adapter.

**D21. Clean rebrand, no shims.** Matches the 2026-04-21 mytex → orchext
playbook. Old installs rebuild. No `orchext migrate` helper.

## Rebrand sweep — what moves

| Surface | From | To |
|---|---|---|
| Crate names (9) + directories | `orchext-*`, `crates/orchext-*/` | `orchext-*`, `crates/orchext-*/` |
| Workspace + Cargo deps | `orchext-*` | `orchext-*` |
| Env var prefix | `ORCHEXT_*` (e.g. `ORCHEXT_BIND`, `ORCHEXT_SERVER_URL`) | `ORCHEXT_*` |
| Vault directory on disk | `~/.orchext/` | `~/.orchext/` |
| Workspaces registry file | `~/.orchext/workspaces.json` | `~/.orchext/workspaces.json` |
| Token prefix | `ocx_*` (constant in `orchext-auth`) | `ocx_*` |
| Desktop bundle ID | `app.orchext.desktop` (or current value) | `app.orchext.desktop` |
| Tauri identifier / signing config | `orchext` | `orchext` |
| Server binary | `orchext-server` | `orchext-server` |
| Docker image / compose | `orchext-server:*` | `orchext-server:*` |
| npm package names | `orchext-desktop-frontend`, `orchext-web` | `orchext-desktop-frontend`, `orchext-web` |
| Types/identifiers in Rust | `Orchext*`, `orchext_*` | `Orchext*`, `orchext_*` |
| GitHub org/repo | `orchext-app/orchext` | `orchext-app/orchext` |
| Docs | every `orchext` / `Orchext` reference | `orchext` / `Orchext` |

## FORMAT v0.2 additions
*([Notion](https://www.notion.so/34d47fdae49a812aa86cd06ccd5994de))*

Brief — full spec in [`../FORMAT.md`](../FORMAT.md) (to be updated
alongside this phase).

### `type: task`

**Frontmatter:**

- `status: backlog | todo | doing | blocked | done | cancelled`
- `priority: low | medium | high | urgent` (optional)
- `due: YYYY-MM-DD | ISO datetime` (optional)
- `assignee: string` (free-form; usernames or emails; optional)
- `source: string` (e.g. `authored`, `todoist`, `linear`, `jira`;
  required, defaults to `authored`)
- `source_id: string` (external system task id; set by integrations
  in 3b; absent for user-authored tasks)
- `parent: [[wikilink]]` (optional — parent task)
- `goal: [[wikilink]]` (optional — links to a `type: goal` doc)
- `visibility` (as today — `public` / `work` / `personal` / custom)

**Body:** free markdown. Acceptance criteria, ancestry, notes.

### `type: skill`

**Frontmatter:**

- `runtimes: [claude-code, cursor, codex, http, shell]` (one or
  more; required)
- `version: semver` (required; bumps force re-injection in later
  phases)
- `inputs: [...]` (optional schema hints — strings for now)
- `outputs: [...]` (optional)
- `visibility` (as today; team skills use `org` once 2c ships)

**Body:** instructions to inject at agent-session start.

### `type: integration`

Placeholder in 3a — the full schema lands in 3b.1 alongside
credentials plumbing. Declaring the type in v0.2 lets 3a's vault
format spec stabilize in one bump rather than two.

### `type: agent`

Placeholder for 3d's agent registry. Same reason: one FORMAT bump.

## Deliverables

- Every tracked crate, app, doc, config, and disk constant renamed.
  *([Notion](https://www.notion.so/34d47fdae49a811fb29af81c1e4e503a))*
- `orchext-tasks` new crate: pure domain (`Task`, `TaskStatus`,
  `Goal`, parsers/serializers `markdown ↔ struct`). No I/O.
  Consumed by `orchext-vault` and `orchext-index`.
  *([Notion](https://www.notion.so/34d47fdae49a8197bf6aee1abc7f6b42))*
- `orchext-vault` extended: seed-type enum includes `task`, `skill`,
  `integration`, `agent`; visibility evaluator unchanged.
  *([Notion](https://www.notion.so/34d47fdae49a816daa7ce593c1156a83))*
- `orchext-index` extended: new views `active_tasks_by_goal`,
  `skills_by_runtime` (backed by FTS).
  *([Notion](https://www.notion.so/34d47fdae49a81209ef3c925818eb982))*
- `orchext-mcp` extended: `task_list(status?, due_before?)`,
  `task_get(id)`, `skill_get(name, runtime?)` tools. Read-only;
  scope-gated by the existing evaluator.
  *([Notion](https://www.notion.so/34d47fdae49a818ab89be2b10c2c8245))*
- Desktop: **Tasks** pane (sortable list by status / due / priority;
  click opens the underlying doc in the existing editor) and
  **Skills** pane (read-only list for now).
  *(Notion: [Tasks pane](https://www.notion.so/34d47fdae49a81ed80e3de9472e96e5f) · [Skills pane](https://www.notion.so/34d47fdae49a8131a033dfd0eae46506))*
- Web: **Tasks** pane parity with desktop. Skills pane deferred to
  3a.1 follow-up if time is tight.
  *([Notion](https://www.notion.so/34d47fdae49a81ed80e3de9472e96e5f))*

## Execution order

1. **Rename PR** — one feature branch, one commit per category
   (crates; Cargo.toml sweep; env vars; disk paths; package.json;
   bundle IDs; docs). `cargo check --workspace` must pass after each
   commit. Merge to `main`; rename the GitHub repo in the same
   window; update any external links.
2. **FORMAT v0.2 spec** — write the new seed-type sections in
   `FORMAT.md` before writing any code against them.
3. **`orchext-tasks` crate** — pure domain, unit-tested. No vault
   dependency yet.
4. **Wire into `orchext-vault`** — register seed types; front-matter
   parser handles the new fields.
5. **Wire into `orchext-index`** — new views.
6. **MCP tools** — read-only surface.
7. **Desktop + web UI** — tasks pane first, skills pane second.

## Verification

- `rg -i "orchext|mytex"` returns zero hits outside `docs/` rebrand
  history notes.
- `cargo test --workspace` — ≥ 148/148 pass with `DATABASE_URL` set
  (plus new `orchext-tasks` unit tests).
- Desktop: fresh vault → create `type: task` doc via UI → see it in
  Tasks pane → edit on disk → reload → changes reflected.
- Web: parity with desktop for the tasks flow.
- MCP: `task_list` from a connected Claude Code / Codex client
  returns correctly scoped task summaries.
- `ocx_*` tokens issued from `auth::create_token`. `.orchext/`
  directory created on fresh desktop install.

## Cuts — explicit

- **No external task sync.** Todoist and friends come in 3b.
- **No `task_propose` / task write MCP tool.** Tasks get created by
  the user in the UI or by the desktop app — the propose flow is
  2b.5's surface.
- **No skill injection.** Skills listed but not injected at
  session-start; that's 3e.2.
- **No goal-type doc yet.** `goal:` wikilinks can point at any doc
  until `type: goal` lands in 3e (for ancestry traversal). This is
  a soft cut — the user can use existing `type: goal`-shaped docs.
- **No mobile responsiveness pass** on the new panes beyond what
  Tailwind gives for free.

## Open questions

- **Does the rebrand drop `mytex` from `implementation-status.md`
  rebrand history?** No — both renames stay in the note as product
  archaeology. Only the active surface (code, disk, envs) moves.
- **Task id format?** Leaning on the existing doc-id scheme
  (`<type>/<slug>` relative to vault root). No UUIDs.
- **Does the Tasks pane filter by `visibility` by default?** First
  cut shows all visibilities the current token can see. Filters are
  a follow-up.
