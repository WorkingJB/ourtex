# Phase 2b.4 — Web client + WASM crypto **[SHIPPED 2026-04-25]**

Opened 2026-04-22, closed 2026-04-25. Adds `apps/web` (Vite + React +
Tailwind) alongside `apps/desktop`, plus a browser-facing
`orchext-crypto-wasm` wrapper so the unlock flow runs entirely
client-side. Pulled ahead of 2b.5 (MCP HTTP/SSE + OAuth PKCE +
`context.propose`) so a shareable URL lands sooner; depended only on
2b.2's HTTP surface and 2b.3's crypto, both shipped.

Forward-looking plan context in [`phase-2-plan.md`](phase-2-plan.md);
live status in [`../implementation-status.md`](../implementation-status.md).

---

### `orchext-crypto` — 2026-04-22 (Phase 2b.4 delta)

No API change. The crate now compiles clean to `wasm32-unknown-unknown`
without a feature flag:

- `rand::thread_rng()` → `rand::rngs::OsRng` in `kdf.rs`,
  `content_key.rs`, `aead.rs`. `OsRng` has no thread-local state, so it
  works on wasm32 where `thread_rng` doesn't.
- `Cargo.toml` gains a target-gated dep on `getrandom` with the `js`
  feature, so `OsRng` routes through `crypto.getRandomValues` in the
  browser.

Argon2id and XChaCha20-Poly1305 are pure-CPU + alloc and need no
additional gating. Native builds and the 13-test unit suite are
unchanged.

### `orchext-crypto-wasm` — 2026-04-22 (new, Phase 2b.4)
*([Notion: WASM crypto wrapper](https://www.notion.so/34d47fdae49a810f8e65f18bb9667e21))*

Thin wasm-bindgen wrapper. Exists as its own crate rather than a
feature on `orchext-crypto` so wasm-bindgen's dependency tree doesn't
leak into native consumers (`orchext-server`, `orchext-sync`, desktop).

**Public API (JS surface, all take/return base64url-nopad strings):**

- `generateSalt() -> string` — fresh 16-byte KDF salt.
- `generateContentKey() -> string` — fresh 32-byte AEAD key.
- `wrapContentKey(contentWire, passphrase, saltWire) -> string` —
  derive master, wrap, return wrapped blob for `init-crypto`.
- `unwrapContentKey(wrappedWire, passphrase, saltWire) -> string` —
  derive master, unwrap, return raw content key ready for
  `/session-key`. Collapses every failure mode into one error.

**Build:** `crate-type = ["cdylib", "rlib"]`; the rlib half lets
`cargo check --workspace` validate the crate on native hosts without
the wasm32 target installed. `wasm-pack build --target web` emits an
ES-module bundle that Vite consumes directly.

**Decisions recorded here:**

- **Separate wrapper crate, not a feature on `orchext-crypto`.** The
  core crate stays free of `wasm-bindgen` and its `js-sys` /
  `wasm-bindgen-macro` toolchain, so server and desktop builds don't
  pay for machinery they never use. Cost: one extra crate in the
  tree and a `wasm-pack` build step in `apps/web`.
- **Four top-level functions, not a stateful class.** The JS side
  holds the passphrase briefly during a single unlock call, then
  drops it. A class would accumulate key state in the JS heap —
  neither more secure (we're inside the browser's process anyway)
  nor better ergonomically.
- **`JsError` for every failure.** `CryptoError::Display` is safe to
  surface (collapsed "decryption failed"); exposing it verbatim keeps
  the enumeration-resistance posture the Rust crate already has.

### `apps/web` — 2026-04-22 (new, Phase 2b.4)
*([Notion: web client (React + Vite + Tailwind)](https://www.notion.so/34b47fdae49a806b8e86fcfb24fcdc8d))*

Sibling to `apps/desktop`. Same toolchain (Vite + React 18 + TS +
Tailwind), no Tauri — hits `orchext-server` directly over HTTPS.

**What's wired (2026-04-22):**

- **Login / signup** — `LoginView.tsx` toggles between
  `POST /v1/auth/login` and `POST /v1/auth/signup`. Session token
  held in `localStorage` under `orchext.session.v1`; bearer attached
  by the shared `request()` helper in `api.ts`.
- **Tenant picker** — `TenantPicker.tsx` hits `GET /v1/tenants`.
  Single-membership accounts (default for personal signup) auto-pick
  and skip the chooser.
- **Unlock** — `UnlockView.tsx` branches on `vault/crypto.seeded`:
  fresh tenants get a "set passphrase" form that generates salt +
  content key in WASM, wraps, and `POST`s `vault/init-crypto`;
  seeded tenants get an "unlock" form that derives + unwraps
  locally. Both paths end by `POST`ing `/session-key`.
- **Heartbeat** — `heartbeat.ts` republishes the content key every
  4 minutes (1/4 of the server's 15-minute default TTL). Cancelled
  on tenant switch, sign out, or component unmount.
- **Documents (read-only)** — `DocumentsView.tsx` renders the list
  from `GET /v1/t/:tid/vault/docs` and the canonical source from
  `GET .../docs/:id` on click.
- **Session lifecycle** — `App.tsx` tri-state: `checking` →
  `locked` (seeded + no local content key) → `ready`. Sign out and
  tenant switch both call `DELETE /session-key` on the outgoing
  tenant before dropping the token.
- **Dev ergonomics** — Vite proxies `/v1/*` + `/healthz` to
  `http://localhost:8080` (override via `ORCHEXT_SERVER_URL`);
  `predev` and `prebuild` npm hooks run `wasm-pack build` so the
  WASM blob is always fresh.

**Decisions recorded here:**

- **Session token in `localStorage`, not httpOnly cookie.** Pragmatic
  first pass — same-origin, survives a reload, one source of truth
  for the bearer. XSS-vulnerable, so moving to an httpOnly cookie
  issued by the server is flagged as a 2b.5 follow-up when the auth
  surface is hardened end-to-end.
- **Per-tab unlock.** Even if `vault/crypto.unlocked = true` (another
  client is holding the key), the web client always requires its own
  local unlock so it has the content key in memory for heartbeat
  recovery. Cost: extra passphrase prompt in a second tab; benefit:
  a single predictable state machine in `App.tsx`.
- **No stdio MCP.** Browsers can't spawn processes. Hosted
  integrations will go through the server's HTTP/SSE MCP in 2b.5.
- **Session-key revocation on tenant switch / logout.** Best-effort
  `DELETE /session-key` before dropping local state. If the request
  fails the server TTL (15 min) will still clean up; the belt is
  cheap and avoids a leaked unlock between workspace hops.
- **wasm-pack output lives under `apps/web/src/wasm/`.** Generated
  files are gitignored by a wasm-pack-managed `.gitignore` inside
  that directory. CI / install should run `npm run build:wasm`
  (which the `predev` / `prebuild` hooks already do) before Vite.

**Bundle footprint (prod build, 2026-04-22):**

- `orchext_crypto_wasm_bg.wasm` — 82 KB
- `index.js` — 162 KB (52 KB gzipped)
- `index.css` — 8 KB (2 KB gzipped)

### Test coverage

No new Rust unit tests this phase — the four WASM-exposed functions
are thin passthroughs to `orchext-crypto` which already has 13/13
passing unit tests covering every code path. `apps/web` has no test
suite yet; the right time to add React/Vitest tests is when the
write path lands and starts accumulating non-trivial UI state
transitions.

### Writes — 2026-04-22
*([Notion: web document CRUD + editor](https://www.notion.so/34d47fdae49a8109a1c2f5728d76bfca))*

Web client now has create / edit / delete parity with desktop for the
doc list. `DocumentsView.tsx` drops the read-only `<pre>` rendering and
gains a `DocEditor` panel modelled on desktop's, plus a "+ New" entry
on the doc list. New helpers:

- `src/docSource.ts` — `buildSource` dumps the form state to YAML
  frontmatter + body, `parseSource` reads the server's canonical
  response back into structured fields. Uses `js-yaml` (~15 KB
  gzipped).
- `api.docWrite` — `PUT /v1/t/:tid/vault/docs/:doc_id` with the
  canonical source. Sends `base_version` on existing docs for
  optimistic concurrency; omits it on creates.
- `api.docDelete` — `DELETE /v1/t/:tid/vault/docs/:doc_id` with
  `?base_version=` as a precondition.

The editor is keyed by `${id}@${version}` so a successful save remounts
it with the post-save version stamp — same pattern desktop uses.

**Server note:** the PUT handler re-parses + re-canonicalizes via
`orchext_vault::Document`, so `buildSource`'s YAML doesn't have to be
byte-exact canonical form. It only needs to round-trip the frontmatter
fields — which the node-side `buildSource` → `parseSource` smoke test
confirms.

Bundle footprint after `js-yaml`:

- `index.js` — 210 KB (68 KB gzipped)
- `index.css` — 11 KB (3 KB gzipped)
- `orchext_crypto_wasm_bg.wasm` — 82 KB

### Tokens + audit views — 2026-04-22
*([Notion: web tokens + audit views](https://www.notion.so/34d47fdae49a81a9af78cb30a33c225b))*

Web client gets parity with desktop for the admin surfaces:

- `TokensView.tsx` — table of `/v1/t/:tid/tokens`; issue form
  (label, TTL days, scope checkboxes) → `POST`; one-time secret
  reveal with clipboard copy; revoke → `DELETE`. Mirrors desktop's
  `TokensView.tsx` but the server's `PublicToken` shape uses
  `last_used_at` / `revoked_at` (nullable timestamps) instead of
  desktop's `last_used` / `revoked: boolean`, so the component reads
  those fields directly.
- `AuditView.tsx` — table of `/v1/t/:tid/audit`. Server returns
  `entries[]` + `head_hash`; the header chip shows the first 12 chars
  of the head hash rather than desktop's `chain_valid`. Client-side
  rehashing would have to mirror the server's exact serde JSON
  encoding, which is brittle; a proper verify endpoint (or a wasm
  helper that uses the same `sha2` + serde_json wire shape) is a
  later-phase task.
- `App.tsx` gets a left-nav (Documents / Tokens / Audit) rendered
  only in the `ready` workspace state. Lock/checking states still
  take the full pane. View selection resets to Documents on tenant
  switch.

Bundle footprint after these views:

- `index.js` — 220 KB (70 KB gzipped)
- `index.css` — 13 KB (3 KB gzipped)
- `orchext_crypto_wasm_bg.wasm` — 82 KB

### Cuts at close (2026-04-25)

- **Graph view dropped** from both clients. Desktop's `GraphView.tsx`
  + `react-force-graph-2d` removed; web never adopted them. The
  view didn't carry its weight against the documents list.
- **Onboarding chat → Phase 3 platform.** Desktop's
  `OnboardingView.tsx` calls a Tauri-specific Anthropic-key-holding
  command. Web needs a server-mediated chat route, which fits
  better alongside the agent-observer plumbing; bundled into
  [`phase-3-platform.md`](phase-3-platform.md).
- **Session token hardening → 2b.5 opening slice.** Web kept the
  bearer in `localStorage` at 2b.4 close. The move to httpOnly
  cookie + CSRF + `/v1/auth/me`-based bootstrap is the first item
  in 2b.5.
- **OS keychain (desktop follow-up) → Phase 3 platform.** Still
  open from 2b.3; bundled with the team/onboarding work.

### Cuts already made

- **No `wasm` feature flag on `orchext-crypto`.** `OsRng` +
  target-gated `getrandom[js]` covers every wasm requirement
  without forking the native build path.
- **No wasm-opt.** `package.metadata.wasm-pack.profile.release`
  disables it; binary is 82 KB already and wasm-opt adds ~10 s to
  every build for a ~5 KB win.
- **No i18n / a11y polish.** Forms are semantic (`<label>` +
  `<input required>`), but there is no aria-live region for errors
  and no translation scaffolding. Defer to post-2b.
