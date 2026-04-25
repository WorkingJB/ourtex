-- Phase 2b.3: at-rest encryption scaffolding.
--
-- Per-tenant crypto material (KDF salt + wrapped content key) lives on
-- `tenants`. A client that knows the passphrase can re-derive the
-- master key, unwrap, and publish the content key to the server's
-- in-memory store (see `session_keys.rs`). Encryption is opt-in per
-- tenant: NULL `kdf_salt` means crypto hasn't been seeded and the
-- tenant stays plaintext (matches 2b.2 behaviour).
--
-- `documents.body` becomes nullable; writes after crypto is seeded
-- store the ciphertext in `body_ciphertext` instead. Existing
-- plaintext rows from 2b.2 are untouched — they keep `body` set and
-- `body_ciphertext IS NULL`. The CHECK constraint pins the
-- invariant that at least one of the two is populated so queries
-- can never see a torn write.

ALTER TABLE tenants
    ADD COLUMN kdf_salt            TEXT,
    ADD COLUMN wrapped_content_key TEXT,
    ADD COLUMN key_version         INT;

ALTER TABLE documents
    ALTER COLUMN body DROP NOT NULL,
    ADD COLUMN body_ciphertext TEXT,
    ADD COLUMN key_version     INT;

ALTER TABLE documents
    ADD CONSTRAINT documents_body_or_ciphertext
    CHECK (
        (body IS NOT NULL AND body_ciphertext IS NULL)
        OR (body IS NULL AND body_ciphertext IS NOT NULL)
    );

-- The FTS tsvector column was generated from `title || ' ' || body`
-- in 0002. Now that `body` can be NULL we re-express it via
-- `coalesce(body,'')` so encrypted rows produce a well-defined empty
-- vector instead of a NULL one. `to_tsvector('english', '')` yields
-- an empty tsv that simply never matches — encrypted content is
-- invisible to server-side FTS while the workspace is locked. A 2b.3+
-- follow-up could rebuild the tsv from plaintext during writes while
-- the session key is live.
ALTER TABLE documents DROP COLUMN tsv;
ALTER TABLE documents ADD COLUMN tsv tsvector
    GENERATED ALWAYS AS (
        to_tsvector('english', coalesce(title, '') || ' ' || coalesce(body, ''))
    ) STORED;
CREATE INDEX documents_tsv_idx ON documents USING GIN (tsv);
