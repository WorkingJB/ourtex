-- Phase 2b.5 schema: OAuth 2.1 authorization-code grant with PKCE.
--
-- The flow lets a logged-in user (desktop or web) authorize an agent
-- client to receive a tenant-scoped, audience-bound `mcp_tokens` row.
-- The opaque-token shape (D15) is unchanged — OAuth defines the
-- *issuance* flow, not the token encoding.
--
-- Codes are short-lived (~10 min) and single-use. The `used_at` column
-- nails replay; an attacker who steals a code mid-flight can only redeem
-- it once, and the rightful client's redemption either also fails or
-- already won. The `code_challenge` is held verbatim and verified via
-- SHA-256(verifier) on the token exchange.
--
-- We hash the code with Argon2id at rest like every other secret in
-- this codebase, and look it up by `code_prefix` (matches the same
-- trick used for sessions / mcp_tokens). PKCE is mandatory: only S256
-- is accepted (per OAuth 2.1 §7.5.2 — `plain` is forbidden).

CREATE TABLE oauth_authorization_codes (
    code_prefix             TEXT        PRIMARY KEY,
    code_hash               TEXT        NOT NULL,
    account_id              UUID        NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    tenant_id               UUID        NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    client_label            TEXT        NOT NULL,
    redirect_uri            TEXT        NOT NULL,
    scope                   TEXT[]      NOT NULL,
    mode                    TEXT        NOT NULL CHECK (mode IN ('read','read_propose')),
    max_docs                INT         NOT NULL DEFAULT 20,
    max_bytes               BIGINT      NOT NULL DEFAULT 65536,
    ttl_days                INT         NOT NULL DEFAULT 90,
    code_challenge          TEXT        NOT NULL,
    code_challenge_method   TEXT        NOT NULL CHECK (code_challenge_method = 'S256'),
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at              TIMESTAMPTZ NOT NULL,
    used_at                 TIMESTAMPTZ
);

CREATE INDEX oauth_authorization_codes_expires_idx
    ON oauth_authorization_codes (expires_at);
