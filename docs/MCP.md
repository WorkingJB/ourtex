# Orchext MCP Server (v0.1)

This document specifies the Model Context Protocol (MCP) surface that
Orchext exposes to AI agents. It is the contract between the Orchext core
and any MCP client — Claude Desktop, an IDE plugin, a CLI agent, or a
future third-party tool.

The goal is the one from the slogan: an agent connecting to Orchext
should be able to read (and eventually propose changes to) a user's
context as easily as it reads its own documentation.

---

## 1. Principles specific to this surface

1. **Least privilege by default.** Every connection uses a token with
   an explicit scope. A token with no scope returns no documents.
2. **Deterministic, grep-shaped responses.** Tools return data
   shaped like the vault — `id`, frontmatter, body — not provider-
   specific structures. An agent that can read a markdown file can
   consume a Orchext response.
3. **Read-only unless the user is watching.** Writes go through
   `context.propose` — proposals land in a review queue and never
   mutate the vault until a session-authed user approves them. Read
   tools work for any token; propose requires `mode: read+propose`.
4. **No provider lock-in for the user.** Any MCP-capable agent works
   the same way. Orchext never shapes its responses for a specific model.

---

## 2. Transports

The same tool surface is exposed over two transports.

| Transport   | Used by                        | Status                                              |
|-------------|--------------------------------|-----------------------------------------------------|
| `stdio`     | Local agents on the same host. | Ships in v1. Spawned as a child process.            |
| `http+sse`  | Remote agents against a server.| Phase 2b. Served by `orchext-server` over TLS, OAuth 2.1 bearer auth. |

In both cases the wire protocol is JSON-RPC 2.0 as defined by the
MCP specification. Orchext does not extend or reinterpret the protocol;
everything below describes the payloads.

### 2.1 Stdio launch

Agents launch the server with:

```
orchext mcp serve --token <token>
```

The token is **not** read from the environment or the filesystem by
default. The agent must pass it explicitly. This makes token leakage
through a shared shell history or dotfile much less likely.

### 2.2 HTTP+SSE endpoint

For remote use, the relay exposes:

```
POST  https://relay.orchext.app/v1/mcp            (JSON-RPC requests)
GET   https://relay.orchext.app/v1/mcp/events     (SSE stream)
```

Both require:

- `Authorization: Bearer <token>`
- `Orchext-Vault-Id: <vault-uuid>`
- TLS 1.3.

The relay never has plaintext access to vault contents. It only
routes an authenticated JSON-RPC stream between the agent and the
user's desktop app (or their self-hosted instance).

---

## 3. Authentication

### 3.1 Tokens

A token is an opaque string with the shape `ocx_<32+ random chars>`.
The server stores only an Argon2id hash. Tokens carry, in the server's
record:

| Attribute        | Purpose                                                        |
|------------------|----------------------------------------------------------------|
| `label`          | Human-readable ("Claude — work laptop"). Shown in the UI.      |
| `scope`          | Set of `visibility` labels this token may read. Non-empty.     |
| `mode`           | `read` or `read+propose`. `propose` mode is required to call `context.propose`. |
| `expires_at`     | Required. Default 90 days. Max 365 days.                       |
| `limits.max_docs`| Max documents returned per request. Default 20. Hard cap 100.  |
| `limits.max_bytes`| Max total body bytes per request. Default 64 KiB. Hard cap 1 MiB. |
| `created_at`     | Set by the server.                                             |
| `last_used`      | Updated on every successful request.                           |
| `id`             | Short opaque token ID used in audit logs. Not the secret.      |

Tokens are revocable from the UI. A revoked token returns
`-32001 / token_revoked` on next use.

### 3.2 Scope semantics

Scope is the set of `visibility` labels a token may read. Given a
token with `scope = ["work", "public"]`:

- `context.list()` returns only documents whose `visibility` is in
  that set.
- `context.get(id)` on a `personal` document returns
  `-32002 / not_authorized`, identically to a non-existent document,
  so that scope cannot be used for enumeration.
- A `scope` argument passed to a tool may only **narrow** the token's
  scope, never widen it.

**The `private` floor.** A token's scope must contain the literal
string `private` to access any `private`-labelled document. There is
no implicit promotion. Out-of-scope `private` documents return the
same `-32002 / not_authorized` error as any other out-of-scope
document and are absent from all listings. The desktop UI warns the
user distinctly when a grant including `private` is approved.

### 3.3 Principal

Every request is attributed to a principal (the vault owner) and a
token ID. Both are recorded in the audit log. Tools never return the
principal in responses.

---

## 4. Initialization

On `initialize`, the server advertises:

```json
{
  "protocolVersion": "2025-06-18",
  "capabilities": {
    "tools": { "listChanged": true },
    "resources": { "listChanged": true, "subscribe": true }
  },
  "serverInfo": {
    "name": "orchext",
    "version": "0.1.0"
  }
}
```

- `tools.listChanged` fires when the user grants or revokes scopes
  that change which tools are meaningful.
- `resources.listChanged` and `resources.subscribe` fire when vault
  contents visible to the token change.

No prompts are exposed in v1.

---

## 5. Tools

All tools live under the `context.` namespace.

### 5.1 `context.search`

Full-text and (when enabled) semantic search over the caller's
in-scope documents.

**Input**

```json
{
  "query": "manager communication style",
  "scope": ["work"],
  "types": ["relationships", "preferences"],
  "tags": ["acme"],
  "limit": 20
}
```

| Field    | Type                | Required | Notes                                              |
|----------|---------------------|----------|----------------------------------------------------|
| `query`  | string              | yes      | 1–512 chars.                                       |
| `scope`  | array of string     | no       | Narrows the token's scope. Must be a subset.       |
| `types`  | array of string     | no       | Filters by document `type`.                        |
| `tags`   | array of string     | no       | Matches any document carrying any of these tags.   |
| `limit`  | integer             | no       | 1–100. Default 20.                                 |

**Output**

```json
{
  "results": [
    {
      "id": "rel-jane-smith",
      "type": "relationship",
      "title": "Jane Smith",
      "snippet": "My manager at Acme. Prefers concise written updates…",
      "score": 0.81,
      "visibility": "work",
      "tags": ["manager", "acme"],
      "updated": "2026-04-18"
    }
  ],
  "truncated": false
}
```

`snippet` is best-effort context around the match. `title` is the
first H1 in the body, or the `id` as a fallback. Full document
retrieval is a separate `context.get` call, to keep `search`
responses small.

Every result carries **provenance**: `id`, `visibility`, `updated`,
and (if set) `source` from the document's frontmatter. Agents should
treat the `snippet` and any body text as untrusted input and use the
provenance to decide how much weight to give it.

Retrieval volume is capped by the token's `limits.max_docs` and
`limits.max_bytes`. A `context.search` call that would exceed either
sets `truncated: true` and returns only what fits.

### 5.2 `context.get`

Fetch a single document by `id` or `alias`.

**Input**

```json
{ "id": "rel-jane-smith" }
```

**Output**

```json
{
  "id": "rel-jane-smith",
  "type": "relationship",
  "frontmatter": {
    "id": "rel-jane-smith",
    "type": "relationship",
    "visibility": "work",
    "tags": ["manager", "acme"],
    "links": ["goal-q2-launch"],
    "created": "2026-04-18",
    "updated": "2026-04-18"
  },
  "body": "# Jane Smith\n\nMy manager at Acme. …",
  "version": "sha256:3f1c…"
}
```

- `version` is the SHA-256 of the serialized document and is used as
  the optimistic-concurrency token for future `context.propose` calls.
- Out-of-scope or nonexistent documents both return
  `-32002 / not_authorized`.

### 5.3 `context.list`

Enumerate documents.

**Input**

```json
{
  "type": "relationships",
  "tags": ["acme"],
  "updated_since": "2026-01-01",
  "cursor": null,
  "limit": 50
}
```

All fields optional. `cursor` is an opaque string returned by a prior
page; `null` starts at the beginning.

**Output**

```json
{
  "items": [
    {
      "id": "rel-jane-smith",
      "type": "relationship",
      "title": "Jane Smith",
      "visibility": "work",
      "tags": ["manager", "acme"],
      "updated": "2026-04-18"
    }
  ],
  "next_cursor": "eyJvZmZzZXQiOjUwfQ=="
}
```

Listing never returns bodies; it is a cheap index lookup. Agents
should follow up with `context.get` for any document they need in
full.

### 5.4 `context.propose`

Submit a change to a document for user review.

**Input**

```json
{
  "id": "rel-jane-smith",
  "base_version": "sha256:3f1c…",
  "patch": {
    "frontmatter": { "tags": ["manager", "acme", "mentor"] },
    "body_append": "\n\nAsked me to mentor a new hire on 2026-04-18."
  },
  "reason": "Observed during our weekly 1:1."
}
```

- `base_version` must match the current `version` of the document, or
  the server returns `-32003 / version_conflict`.
- `patch` supports `frontmatter` (merge), `body_replace` (full
  replacement), and `body_append` (string append). Exactly one of
  `body_replace` / `body_append` may be set.
- For stdio MCP against a local vault, proposals land in
  `<vault>/.orchext/proposals/<proposal_id>.json`. For HTTP MCP
  against `orchext-server`, proposals land in the server's
  `proposals` table. Both desktop and web clients surface them in a
  review queue.

**Output**

```json
{
  "proposal_id": "prop-2026-04-18-abc123",
  "status": "pending"
}
```

A proposal never mutates the vault. Agents that need to know the
outcome should subscribe to the relevant resource (§6) and watch for
an `updated` event after user approval.

---

## 6. Resources

The vault is exposed as MCP resources in addition to tools, so agents
that prefer browsing to searching can walk the tree.

### 6.1 URI scheme

```
orchext://vault/<type>/<id>
orchext://vault/<type>/
orchext://vault/
```

- `orchext://vault/` — lists visible types.
- `orchext://vault/<type>/` — lists visible documents of that type.
- `orchext://vault/<type>/<id>` — returns the document's contents.

Only resources whose document `visibility` is within the token's
scope are advertised. Hidden resources are not listed, and direct
URIs to them return `-32002 / not_authorized`.

### 6.2 Content shape

`resources/read` returns the document as two content items:

- A `text/yaml` item containing the frontmatter (canonicalized).
- A `text/markdown` item containing the body.

This matches how the document is stored on disk and keeps parsing
trivial for the agent.

### 6.3 Subscriptions

`resources/subscribe` accepts any advertised URI. The server emits
`notifications/resources/updated` when:

- The underlying file is written.
- The document's `visibility` changes such that it enters or leaves
  the token's scope (the notification carries the new accessibility).

The subscription drops silently if the token is revoked.

---

## 7. Error model

JSON-RPC errors use the standard envelope with a Orchext-specific code
in `error.code` and a short machine-readable tag in `error.data.tag`.

| Code     | Tag                 | Meaning                                             |
|----------|---------------------|-----------------------------------------------------|
| `-32000` | `server_error`      | Unexpected. Retry with backoff.                     |
| `-32001` | `token_revoked`     | Token was revoked. Request a new one.               |
| `-32002` | `not_authorized`    | Out of scope, missing, or revoked. Indistinguishable by design. |
| `-32003` | `version_conflict`  | `base_version` did not match current.               |
| `-32004` | `invalid_argument`  | Input did not match the tool's schema.              |
| `-32005` | `rate_limited`      | See `error.data.retry_after_ms`.                    |
| `-32006` | `vault_locked`      | Vault is encrypted and the user has not unlocked it.|
| `-32007` | `proposals_disabled`| `context.propose` called on a server that doesn't support writes yet. |

`-32002 / not_authorized` is deliberately ambiguous: it covers
out-of-scope documents, nonexistent documents, and revoked direct
access, so that the error itself cannot be used to enumerate content.

---

## 8. Rate limiting

The local stdio server applies a light rate limit (default 60
requests per 10 seconds per token) mostly to protect the indexer from
runaway loops. The cloud relay applies a stricter limit, configurable
per plan. Limits are reported via standard `error.data.retry_after_ms`
and via a `Orchext-RateLimit-*` header set on HTTP responses.

Limits are per **token**, not per vault, so a misbehaving agent cannot
lock out a well-behaved one.

---

## 9. Audit

Every successful or denied request appends an entry to the vault's
audit log (see `ARCHITECTURE.md §5.7`). Entries include the token ID,
tool name, document ID (if any), scope in effect, and outcome.

The raw request payload is not logged. Query strings from
`context.search` are hashed before storage, so a stolen audit log
does not reveal what the agent was looking for.

---

## 10. Versioning

The MCP surface is versioned alongside the vault format (see
`FORMAT.md §8`). The server advertises the version in `serverInfo.version`.

- **Patch:** new optional fields on existing tools. Clients ignore
  unknown fields.
- **Minor:** new tools or resources. Clients that don't know about
  them simply don't call them.
- **Major:** breaking changes to existing tools. Reserved and
  avoided. Any major bump ships with a deprecation period of at
  least one minor release.

The server returns the full list of supported tools via
`tools/list`; clients should prefer that over hardcoded assumptions.

---

## 11. What's in v1 vs later

**v1 (ships with the desktop app, local stdio only)**

- `initialize`, `tools/list`, `resources/list`, `resources/read`,
  `resources/subscribe`.
- Tools: `context.search`, `context.get`, `context.list`.
- Stdio transport.
- Compatibility targets: Claude Desktop (primary — polish, docs,
  error messages written against it) and Cursor (guaranteed
  secondary — required pass in the v1 test matrix, no UX
  optimization).

**Phase 2b** (with `orchext-server`)

- HTTP+SSE transport mounted on `orchext-server`. Same JSON-RPC
  surface. OAuth 2.1 bearer auth replaces opaque tokens for remote
  callers; opaque tokens remain valid for local stdio.
- `context.propose` lands. Proposals flow into the workspace's
  proposal queue; desktop and web clients surface them for review.
- Server-side session-bound decryption (ARCH §3.4, reconciled
  Q3) — the server can answer MCP requests only while a client
  device has published a session key.

**Phase 2c** (teams)

- All MCP calls on a team workspace are workspace-scoped by the
  bearer token's audience. Resource URIs and tool results are
  filtered by role-derived default scopes plus any per-token
  narrowing.
- New built-in visibility `org` enters scope negotiation. Like
  `private`, it is a hard label.
- `context.propose` is the member-writes-to-`org/*` path; admin
  approval merges.

**Later**

- Streaming search results.
- Per-type custom schemas exposed to clients for richer UI hints.
- Write-through tools for the in-app onboarding agent (distinct
  surface, internal-only).

---

## 12. Example session

A Claude Desktop instance connects over stdio with a token scoped to
`["work", "public"]`.

```
→ initialize
← { protocolVersion, capabilities, serverInfo }

→ tools/list
← [ context.search, context.get, context.list ]

→ tools/call context.search
   { "query": "how does the user prefer to receive updates" }
← { "results": [
     { "id": "pref-comms", "title": "Communication style",
       "snippet": "Prefer written over spoken…", "score": 0.74 }
   ]}

→ tools/call context.get
   { "id": "pref-comms" }
← { "id": "pref-comms", "frontmatter": {...}, "body": "# Communication…" }
```

All three calls are audited. The token's `last_used` is updated. No
`personal` document is ever visible or listable to this agent.
