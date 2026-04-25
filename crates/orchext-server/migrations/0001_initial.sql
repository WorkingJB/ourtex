-- Phase 2b.1 schema: accounts + sessions + memberships.
--
-- `memberships` is unused until Phase 2c but lives here so we don't
-- run another migration the moment we need it. `tenant_id` columns
-- throughout are NOT NULL and currently always resolve to a single
-- implicit tenant per account; multi-tenancy enforcement lands in 2c.

CREATE TABLE accounts (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    email       TEXT        NOT NULL UNIQUE,
    password    TEXT        NOT NULL,            -- Argon2id PHC string
    display_name TEXT       NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX accounts_email_lower_idx ON accounts ((lower(email)));

-- Opaque session tokens. The raw `otx_*` secret is only visible to the
-- caller that just logged in; the server stores only an Argon2id hash
-- plus a short `token_prefix` for lookup (matches `ourtex-auth` pattern).
CREATE TABLE sessions (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id    UUID        NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    token_prefix  TEXT        NOT NULL UNIQUE,  -- first 8 chars of the secret, for O(1) lookup
    token_hash    TEXT        NOT NULL,         -- Argon2id hash of the full secret
    label         TEXT        NOT NULL DEFAULT 'web session',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at    TIMESTAMPTZ NOT NULL,
    last_used_at  TIMESTAMPTZ,
    revoked_at    TIMESTAMPTZ
);

CREATE INDEX sessions_account_idx ON sessions (account_id);
CREATE INDEX sessions_expires_idx ON sessions (expires_at);

-- Tenants / memberships, pre-built for Phase 2c. In 2b.1 we create a
-- personal tenant for every new account (owner role) so future
-- workspace endpoints can key off `tenant_id` without a migration.
CREATE TABLE tenants (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT        NOT NULL,
    kind        TEXT        NOT NULL CHECK (kind IN ('personal', 'team')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE memberships (
    tenant_id   UUID        NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    account_id  UUID        NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    role        TEXT        NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, account_id)
);

CREATE INDEX memberships_account_idx ON memberships (account_id);
