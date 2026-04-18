# Mytex Architecture

> API and AI documentation but for you.

Mytex lets a person define, manage, and transport a living set of context
files about themselves, and connect any AI agent to them — without handing
every agent full access to everything, and without being locked into a
single AI provider's memory system.

This document describes the v1 architecture and the decisions behind it.
It is the starting contract for contributors and should be kept in sync
with the code.

---

## 1. Guiding principles

The design is evaluated against four principles, in order:

1. **User first** — the user owns their data, their keys, and the
   relationship with every agent. No feature ships that weakens that.
2. **Easy for everyone** — a non-technical person can install Mytex,
   onboard in a conversation, and connect an agent in under ten minutes.
3. **Simple over fancy** — boring, auditable, well-understood tech wins
   over novel tech. Plain markdown beats a bespoke database.
4. **Always secure** — the default configuration is the secure
   configuration. Security is not a tier or an upsell.

When these conflict, earlier principles win.

---

## 2. System shape: local-first, sync-optional

The source of truth is a **vault** — a plain directory of markdown files
on the user's machine. Every other component (desktop UI, local API,
MCP server, cloud sync) is a view or transport over that directory.

```
┌──────────────────────────────────────────────────────────────┐
│  Desktop app (Tauri)                                         │
│  ├─ UI                 (TypeScript / React)                  │
│  ├─ Core engine        (Rust — small, audited surface)       │
│  │   ├─ Vault driver                                         │
│  │   ├─ Indexer                                              │
│  │   ├─ Token + permission service                           │
│  │   ├─ Audit log                                            │
│  │   └─ Crypto                                               │
│  ├─ Local API          (HTTPS on 127.0.0.1)                  │
│  └─ MCP server         (stdio + HTTP/SSE)                    │
└──────────────────────┬───────────────────────────────────────┘
                       │  reads / writes
                       ▼
               ~/Mytex/ (vault)
               ├─ .mytex/        config, keys, index, audit
               ├─ identity/
               ├─ goals/
               ├─ relationships/
               ├─ tools/
               └─ …               markdown + YAML frontmatter
                       │
                       │  optional, E2EE
                       ▼
               Cloud tier (paid)
               ├─ Encrypted blob sync
               ├─ MCP/API relay
               └─ Web UI (decrypts in-browser via WASM)
```

**Why local-first**

- The user's disk is the most secure default storage available.
- A vault-as-folder is portable, grep-able, diff-able, and git-able.
- Cloud becomes encrypted transport, not a second system.
- One codebase serves self-hosted and hosted modes.

---

## 3. Key decisions

These are the choices that shape everything else. They should not be
changed without updating this document.

### 3.1 Framework: Tauri with a thin Rust core

The UI is TypeScript/React. A deliberately small Rust core
(filesystem I/O, crypto, token validation, MCP server) is kept
auditable by humans.

**Why:** Tauri's default-deny IPC model catches mistakes that AI-written
code tends to make in Electron (unsafe IPC, `nodeIntegration`, sloppy
preload scripts). The UI — where the bulk of AI-generated code will live
— is still TypeScript. The trusted core is small enough to review line
by line.

**Escape hatch:** the file format, MCP protocol, and API contract are
framework-independent. A port to Electron later would not break any
user's vault.

### 3.2 Storage: pluggable vault driver, plain files first

All vault access goes through a `VaultDriver` interface. v1 ships a
`PlainFileDriver`. A future `EncryptedDriver` (per-file envelope, so
git diffs still work) is a drop-in replacement.

```ts
interface VaultDriver {
  list(path: string): Promise<Entry[]>
  read(id: string): Promise<Document>
  write(id: string, content: Document): Promise<void>
  watch(callback: ChangeHandler): Unsubscribe
}
```

**Why:** users get the portability of plain markdown today, with a
clean path to encryption-at-rest later. Nothing in the UI, API, or MCP
server knows which driver is in use.

### 3.3 Onboarding: in-app agent, external agents read-only

Two trust tiers for writes:

- **In-app onboarding agent** runs inside the desktop app using the
  user's chosen model. Because the user is actively watching the
  conversation, it writes directly to the vault. No MCP, no token, no
  per-write prompt.
- **External agents via MCP** are read-only by default. Writes go
  through `context.propose(id, patch)`, which lands in
  `.mytex/proposals/`. The desktop app surfaces proposals for user
  review; approval merges them.

**Why:** conversational onboarding is the easiest possible UX, and it
doesn't require opening the external write surface to every agent the
user ever connects.

### 3.4 Cloud tier: end-to-end encrypted relay

Cloud storage and the hosted MCP relay never see plaintext.

- Master key derived from the user's passphrase via Argon2id
  (tuned to ~500 ms on a mid-range laptop).
- Files encrypted client-side (libsodium `secretstream` or age) before
  upload. Server sees opaque blobs plus minimal metadata.
- Web UI decrypts in-browser via WASM; the server never holds the
  passphrase or the derived key.
- Recovery = a one-time recovery code generated at setup, printable.
  Optional Shamir split across trusted contacts for users who want it.
  No server-side recovery.

**UX:** modelled on Bitwarden / 1Password. Passphrase once per
session, cached in the OS keychain, biometric unlock after that.

### 3.5 File format: markdown + YAML frontmatter + wikilinks

See [`FORMAT.md`](./FORMAT.md) for the full spec.

Short version: every context object is a `.md` file with a YAML
frontmatter header declaring `id`, `type`, `visibility`, `tags`,
`links`, and a handful of reserved fields. The body is freeform
markdown. Inter-document references use Obsidian-style `[[wikilinks]]`.

**Why:** Obsidian-compatible on purpose. No lock-in, git-friendly, and
any text editor is a fallback UI.

---

## 4. Components

### 4.1 Vault

A directory on disk with a fixed top-level layout:

```
~/Mytex/
├─ .mytex/
│   ├─ config.json        user preferences, driver selection
│   ├─ tokens.json        hashed agent tokens + scopes
│   ├─ audit.log          append-only, hash-chained
│   ├─ index.sqlite       derived search + graph index
│   └─ proposals/         pending agent-proposed writes
├─ identity/
├─ roles/
├─ goals/
├─ relationships/
├─ memories/
├─ tools/
├─ preferences/
├─ domains/
└─ decisions/
```

The directories under the root map to the seed `type` values. Users may
add their own top-level folders; the indexer treats them as custom
types.

### 4.2 Core engine (Rust)

Responsibilities, kept narrow:

- **Vault driver** — the only code that touches files.
- **Indexer** — watches the vault, rebuilds `index.sqlite`, exposes
  search and graph queries. The SQLite index is derived, never
  authoritative.
- **Token + permission service** — issues, hashes, validates, and
  revokes per-agent tokens; evaluates scope against document
  `visibility`.
- **Audit log** — append-only, hash-chained record of every read and
  write (by whom, when, what, scope used).
- **Crypto** — key derivation, vault encryption (v2), recovery codes,
  signing.

Everything else lives in the TypeScript layer.

### 4.3 UI (TypeScript / React)

- Vault picker and onboarding wizard.
- Markdown editor with a frontmatter form (type-aware field hints).
- Graph view of `[[wikilinks]]`.
- Token manager: create, scope, revoke, last-used.
- Audit log viewer.
- Proposal review queue.
- In-app onboarding agent chat.

### 4.4 Local API (HTTPS on 127.0.0.1)

A small REST surface for agents and automations that can't speak MCP.
Same auth and scoping as MCP. Bound to loopback only. See
`docs/API.md` (not yet written) for the wire format.

### 4.5 MCP server

The primary agent surface. Tools exposed in v1:

- `context.search(query, scope?)` — full-text + semantic search,
  filtered by the calling token's scope.
- `context.get(id)` — fetch a single document.
- `context.list(type?, tags?)` — enumerate documents.
- `context.propose(id, patch)` — submit a change for user review.

Transport: stdio for local agents, HTTP/SSE for remote agents via the
cloud relay.

### 4.6 Cloud sync + relay (paid tier)

- **Sync service** stores encrypted blobs keyed by vault ID and file ID.
- **Relay service** forwards MCP/API calls from remote agents to the
  user's desktop app over an authenticated tunnel, or to a user-run
  lightweight decryptor instance when the desktop is offline.
- **Web UI** is a static site that pulls encrypted blobs, decrypts in
  the browser, and reuses most of the desktop UI components.

The cloud tier is open source too; the paid value is running it.

---

## 5. Security model

### 5.1 Data at rest

- v1: plain markdown files. OS-level file permissions only.
- v2: optional vault encryption (per-file envelope). OS keychain
  holds the derived key; passphrase required once per session.
- Cloud blobs: always encrypted, even in v1.

### 5.2 Agent authentication

- Each agent connection has its own opaque token.
- Server stores only a hash.
- Tokens carry: scope (which `visibility` labels it can read), mode
  (read or read+propose), expiry, and a human-readable label
  ("Claude — work laptop").
- One-click revoke. Last-used timestamp visible in the UI.
- Tokens never appear in logs or audit entries; the audit refers to
  token IDs.

### 5.3 Scope evaluation

Every request is evaluated as:

```
allowed = token.mode covers request.action
       ∧ document.visibility ∈ token.scope
       ∧ token not expired
       ∧ token not revoked
```

Scope is the atom of permission. `visibility` values in v1:
`personal`, `work`, `public`, plus any custom labels the user creates.

### 5.4 Write surface

External agents can never write directly. `context.propose` lands in
`.mytex/proposals/` as a patch against a specific document version.
The desktop app shows a diff; user approves or rejects. The in-app
onboarding agent is the single exception, and only while the user is
watching.

### 5.5 Prompt injection

Context documents are user-controlled but can contain text copied from
untrusted sources. The core treats all document bodies as untrusted
input when rendering to agents: no special escape sequences, no
instruction-like phrasing is given elevated meaning. Agents are
responsible for their own prompt hygiene; Mytex does not sanitize.

### 5.6 Transport

- Local API and MCP HTTP bind to `127.0.0.1` only. No LAN exposure.
- Cloud relay uses mTLS between desktop and relay.
- Web UI served over HTTPS with HSTS and a strict CSP.

### 5.7 Audit

Every read and write, local or remote, produces an audit entry:
`(timestamp, token_id or "owner", action, document_id, scope_used,
outcome)`. The log is append-only and hash-chained, so tampering is
detectable. The UI can filter by token, document, or time range, and
export the log to the user on demand.

### 5.8 Recovery

- Cloud-synced vaults: recovery code shown once at setup, printable.
- Optional: Shamir-split recovery across N trusted contacts.
- No server-side recovery. This is a product feature, not a limitation.

### 5.9 Supply chain

- Reproducible builds for all desktop binaries.
- Signed releases.
- The Rust core's dependency set is kept deliberately small and
  pinned. Adding a dependency to the core requires explicit review.
- The TypeScript layer uses a locked, audited dependency set; updates
  go through Dependabot-style review.

---

## 6. Open source and commercial split

- **Open source (permissive license):** core engine, file format
  spec, desktop app, local API, MCP server, self-hosted sync server,
  web UI source.
- **Commercial (hosted):** operated cloud sync, operated MCP relay,
  priority support, and — later — team/org features (SSO, shared
  context, admin console, policy).

Openness of the format and protocol is load-bearing: the security
claims are only credible because anyone can verify them.

---

## 7. Designing now for teams later

Two decisions keep the team path open without adding team complexity
to v1:

- **Actor model.** Every document, token, and proposal has a
  `principal` field. In v1 it is always the single user. A team is
  just another principal with members, added later.
- **Namespace.** Vault paths are already a tree. Teams become new
  roots (`~/Mytex/personal/`, `~/Mytex/acme-team/`) with their own
  keys and policies. No schema change required.

v1 does **not** ship any org, team, or RBAC UI.

---

## 8. v1 scope

In scope:

1. Tauri desktop app: create vault, browse and edit markdown with
   frontmatter, graph view.
2. Seed context types: identity, roles, goals, relationships,
   memories, tools, preferences, domains, decisions (see
   `FORMAT.md`).
3. Local MCP server with `context.search`, `context.get`,
   `context.list`.
4. Scoped token management UI and audit log viewer.
5. Import from Obsidian vault.
6. In-app onboarding agent flow.

Out of scope for v1:

- Cloud sync and relay.
- REST API (can ship in v1.1 if needed).
- `context.propose` write-back flow.
- Any team, org, or RBAC surface.
- Mobile apps.
- Browser extension.

---

## 9. Glossary

- **Vault** — a user's root directory of context files.
- **Document** — a single markdown file in the vault.
- **Type** — a document's top-level category (`identity`, `goal`, …).
- **Visibility** — a label on a document used for permission scoping.
- **Token** — an opaque credential granting a specific agent a
  specific scope.
- **Principal** — the owner of a vault, token, or document. Always
  the single user in v1.
- **Scope** — the set of `visibility` labels a token may read.
- **Proposal** — an agent-submitted change awaiting user approval.
