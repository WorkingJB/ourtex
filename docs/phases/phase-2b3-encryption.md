# Phase 2b.3 — Encryption at rest + session-bound decryption (shipped)

Shipped 2026-04-19. New `ourtex-crypto` crate (Argon2id KDF +
XChaCha20-Poly1305 AEAD); server gains `/vault/crypto`,
`/vault/init-crypto`, and `/session-key` endpoints plus encrypted
`documents.body_ciphertext`; desktop gains unlock/lock commands with a
4-minute heartbeat. Forward-looking plan context in
[`phase-2-plan.md`](phase-2-plan.md); live status in
[`../implementation-status.md`](../implementation-status.md).

---

### `ourtex-crypto` — 2026-04-19 (new, Phase 2b.3)

Passphrase KDF + AEAD primitives. Intentionally minimal — the crate
exposes only what the client/server need to cooperate on
session-bound decryption.

**Public API:**

- `Salt::generate()` / `to_wire()` / `from_wire(&str)` — 16-byte
  KDF salt, base64url on the wire.
- `derive_master_key(passphrase, &Salt) -> MasterKey` — Argon2id
  (default profile) with an 8-char minimum passphrase check.
- `ContentKey::generate()` — fresh random 32-byte AEAD key per
  workspace. Zeroized on drop.
- `wrap_content_key(&ContentKey, &MasterKey) -> SealedBlob` and
  `unwrap_content_key(&SealedBlob, &MasterKey) -> Result<ContentKey>`
  — XChaCha20-Poly1305 wrap/unwrap of the 32 key bytes.
- `seal(plaintext, &[u8; 32]) -> SealedBlob` /
  `open(&SealedBlob, &[u8; 32]) -> Result<Vec<u8>>` — general AEAD;
  nonce is random per call and bundled into the sealed blob.
- `SealedBlob::to_wire() / from_wire(&str)` — base64url-nopad of
  `<24-byte nonce><ct+tag>`. Same format for wrapped keys and
  encrypted document bodies.

**Decisions recorded here:**

- **XChaCha20-Poly1305 over plain ChaCha20-Poly1305.** 192-bit nonce
  is long enough to pick at random per encryption without a counter
  table. Removes an entire class of operational footguns.
- **Argon2id `default()` profile.** Same as `ourtex-server`'s
  password hashing and `ourtex-auth`'s token hashing — one parameter
  set across the workspace, easy to bump in one place.
- **`CryptoError::Open` collapses every decryption failure.** Wrong
  key, tampered ciphertext, truncated nonce, and bad base64 all map
  to the same variant so error output can't be used as an oracle.
- **Zeroize on drop for `MasterKey` and `ContentKey`.** The inner
  `[u8; 32]` is scrubbed when the handle leaves scope. Not a
  defense against a compromised process — just reduces the window
  for opportunistic memory dumps.
- **No per-doc keys in this pass.** One content key per workspace;
  every document's body is sealed under it. Per-doc keys + key
  rotation are future work (touches the `key_version` column already
  present in the schema for exactly this reason).
- **WASM support landed in 2b.4 without a feature flag.** Swapped
  `rand::thread_rng()` for `rand::rngs::OsRng` (no thread-local state)
  and added a `cfg(target_arch = "wasm32")` dep on `getrandom` with
  the `js` feature. Argon2 and XChaCha20-Poly1305 are pure-CPU + alloc
  and need no extra gating. `cargo build -p ourtex-crypto --target
  wasm32-unknown-unknown` succeeds.

### `ourtex-server` — 2026-04-19 (Phase 2b.3 delta)

Adds at-rest encryption to the vault endpoints, server-side
session-key store, and four new control-plane routes. Encryption is
**opt-in per tenant**: an unseeded tenant keeps storing plaintext,
matching 2b.2's behaviour. New writes on a seeded tenant encrypt
server-side; reads decrypt if the session key is live, else
`423 Locked`.

**New schema (`migrations/0003_encryption.sql`):**

- `tenants`: `kdf_salt TEXT`, `wrapped_content_key TEXT`,
  `key_version INT` — all NULL when the tenant hasn't seeded crypto.
- `documents`: `body` becomes nullable; `body_ciphertext TEXT`,
  `key_version INT`. CHECK constraint pins the invariant that
  exactly one of `body` / `body_ciphertext` is populated.
- `tsv` column re-expressed as
  `to_tsvector('english', coalesce(title,'') || ' ' || coalesce(body,''))`
  so encrypted rows produce an empty tsvector (no FTS on encrypted
  content while locked).

**New routes (all tenant-scoped):**

| Method | Path                              | Purpose                                |
| ---    | ---                               | ---                                    |
| GET    | `/v1/t/:tid/vault/crypto`         | Fetch salt + wrapped content key + `unlocked` flag |
| POST   | `/v1/t/:tid/vault/init-crypto`    | First-time seed (admin-only, 409 if already seeded) |
| POST   | `/v1/t/:tid/session-key`          | Publish or refresh the live content key |
| DELETE | `/v1/t/:tid/session-key`          | Drop the live content key (lock)       |

**New server modules:**

- `session_keys.rs` — in-memory `SessionKeyStore` (mutex-guarded
  hashmap) keyed by tenant_id. 15-minute default TTL; entries
  self-evict on the read path when expired. Keys never persist —
  a process restart re-locks every tenant.
- `crypto_api.rs` — the four new endpoints above. `init-crypto`
  uses `UPDATE ... WHERE kdf_salt IS NULL` as a TOCTOU-free
  idempotent-forbidden guard (409 if already seeded).
- `documents.rs` — extended with `resolve_body` that picks plaintext
  or decrypts ciphertext via the live session key. Writes branch on
  whether the tenant is seeded: encrypt server-side if so, store
  plaintext otherwise. `vault_locked` surfaces when the tenant is
  seeded but no key is live.
- `error.rs` — new `ApiError::VaultLocked` variant, status `423`,
  tag `vault_locked`.

**Decisions recorded here:**

- **Server-side encryption, not end-to-end.** Matches ARCH.md §3.4
  / D9: the server holds a short-lived content key and does
  encrypt/decrypt in memory while a client is online. This lets
  hosted agents (future MCP HTTP) read context without per-agent
  key plumbing. Strict-E2EE is an explicit follow-up (no
  `e2ee_opt_out` flag in 2b.3).
- **`init-crypto` is admin-only, 409 on re-seed.** The passphrase
  becomes the canonical recovery secret for every document in the
  workspace — only an owner/admin can decide what it is. Re-seed
  is refused because it would orphan every existing ciphertext;
  key *rotation* is a future endpoint that re-wraps without
  invalidating rows.
- **`key_version` present but pinned to 1.** Rotation will advance
  it; the column is plumbed through inserts + storage now so 2b.3+
  can add versioning without a schema touch.
- **FTS off for encrypted rows.** `coalesce(body, '')` in the tsv
  expression means encrypted rows contribute nothing to search.
  While a session key is live the server *could* decrypt during
  write and materialize plaintext into a tsv, but that's a 2b.3+
  optimisation.
- **Session-key store is process-memory only.** Persisting would
  defeat the locked-after-restart posture. A multi-process
  deployment (Phase 2b.4+) either runs the store on one
  consistent-hash-picked node or promotes it to a shared Redis —
  TBD.

**Integration tests (`tests/crypto_flow.rs`):**

- `encrypted_round_trip` — seed, publish key, write, read; canonical
  source round-trips through the server's AEAD path.
- `vault_locked_without_key` — revoke the session key; subsequent
  read returns 423 `vault_locked`; write also returns 423.
- `wrong_passphrase_fails_to_unwrap` — client-side: fetching the
  crypto state and deriving a master key with the wrong passphrase
  cannot unwrap the content key.
- `init_crypto_is_idempotent_forbidden` — second `init-crypto` on
  the same tenant returns 409 `crypto_already_seeded`.
- `plaintext_legacy_rows_still_readable` — unseeded tenants continue
  to operate in plaintext mode; 2b.2 rows are unchanged.

### `ourtex-sync` — 2026-04-19 (Phase 2b.3 delta)

Adds control-plane wrappers for the four new server endpoints. No
data-path changes — reads/writes go through the existing
`RemoteVaultDriver` and the server handles encryption transparently
based on whether the session key is live.

- `RemoteClient::get_crypto_state() -> CryptoState { seeded,
  kdf_salt, wrapped_content_key, key_version, unlocked }`.
- `RemoteClient::init_crypto(&salt_wire, &wrapped_wire)`.
- `RemoteClient::publish_session_key(&key_wire)` — refreshes the
  TTL on every call; heartbeat-friendly.
- `RemoteClient::revoke_session_key()`.

New workspace dep: `ourtex-crypto`.

### `ourtex-desktop` — 2026-04-19 (Phase 2b.3 delta)

Unlock / lock flow for remote workspaces.

**New Tauri commands:**

- `workspace_unlock(passphrase)` — derives the master key via
  `ourtex-crypto::derive_master_key`, fetches the server's crypto
  state, and either (a) seeds crypto for a fresh tenant or (b)
  unwraps the stored content key with the master. Publishes the
  content key, spawns a heartbeat task, and runs a full
  `reindex_from` now that the server can decrypt.
- `workspace_lock()` — aborts the heartbeat task and calls
  `DELETE /session-key`.
- `workspace_crypto_state()` — reports `{ kind, seeded, unlocked }`
  so the UI can choose between "Connect", "Unlock", and "Lock"
  affordances without exposing any key material.

**State changes (`state.rs`):**

- `OpenVault` gains `remote_client: Option<Arc<RemoteClient>>` (so
  unlock can reach `/vault/crypto` without downcasting
  `Arc<dyn VaultDriver>`) and `heartbeat: Option<HeartbeatHandle>`
  (dropping the vault aborts the background task).
- `HeartbeatHandle::spawn(client, content_key_wire)` — republishes
  every 4 minutes (at ~1/4 of the server's 15-minute default TTL),
  cancelled on drop via `JoinHandle::abort`.
- `open_remote` now tolerates `vault_locked` on the initial reindex
  — a fresh remote workspace starts locked; the first successful
  `workspace_unlock` call runs the real reindex.

**Decisions recorded here:**

- **No OS keychain yet.** The master key lives in client memory for
  the duration of the workspace open. Locking or closing the app
  drops it; re-unlock requires the passphrase. `keyring`-based
  caching is explicitly a follow-up.
- **No auto-unlock prompt yet.** The backend is ready; the React
  modal that prompts the user at activate time and wires up the
  `workspace_unlock` invocation is a remaining UI task.
- **Heartbeat interval is 1/4 of server TTL.** One missed refresh
  does not lock the workspace; two in a row does. Conservative but
  cheap.

**Known gaps after Phase 2b.3:**

- **Unlock modal not wired in the React UI.** Backend commands
  (`workspace_unlock` / `workspace_lock` / `workspace_crypto_state`)
  compile and pass integration tests, but the desktop frontend
  doesn't yet surface an "Unlock" affordance. Follow-up.
- **Master key held only in client process memory.** Re-prompts
  for passphrase on app restart. OS keychain integration is the
  usual polish pass.
- **FTS on encrypted content.** Encrypted rows are invisible to
  server-side search. Re-populating tsv from plaintext during
  write (while a session key is live) would fix this.
- **No key rotation endpoint.** `key_version` column is ready;
  endpoint to roll the content key + re-encrypt in batches is
  follow-up.
- **Strict-E2EE opt-out.** A per-account flag to skip server-side
  decryption entirely (hosted agents see locked state for those
  users) is explicit future work from D9.
