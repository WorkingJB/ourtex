-- Phase 2b.2 schema: vault documents, tag/link graph, audit chain, MCP tokens.
--
-- Same shape as the local stack (mytex-vault + mytex-index + mytex-audit +
-- mytex-auth) but rehomed in Postgres and keyed by tenant_id. Frontmatter
-- is preserved verbatim as JSONB so the round-trip contract (`FORMAT.md`
-- §3.4) survives a server trip — unknown / x-* fields come back intact.

CREATE TABLE documents (
    tenant_id   UUID        NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    doc_id      TEXT        NOT NULL,
    type_       TEXT        NOT NULL,
    visibility  TEXT        NOT NULL,
    title       TEXT        NOT NULL,
    frontmatter JSONB       NOT NULL,                         -- serde-serialized Frontmatter
    body        TEXT        NOT NULL,
    version     TEXT        NOT NULL,                         -- sha256:<hex>
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, doc_id)
);

CREATE INDEX documents_tenant_type_idx       ON documents (tenant_id, type_);
CREATE INDEX documents_tenant_visibility_idx ON documents (tenant_id, visibility);

-- Full-text column + GIN index, mirroring the FTS5 shape in mytex-index.
-- `title` gets the same weight as the body today; adjust with `setweight`
-- later if ranking turns out to need it.
ALTER TABLE documents ADD COLUMN tsv tsvector
    GENERATED ALWAYS AS (
        to_tsvector('english', coalesce(title, '') || ' ' || body)
    ) STORED;
CREATE INDEX documents_tsv_idx ON documents USING GIN (tsv);

CREATE TABLE doc_tags (
    tenant_id UUID NOT NULL,
    doc_id    TEXT NOT NULL,
    tag       TEXT NOT NULL,
    PRIMARY KEY (tenant_id, doc_id, tag),
    FOREIGN KEY (tenant_id, doc_id) REFERENCES documents(tenant_id, doc_id) ON DELETE CASCADE
);
CREATE INDEX doc_tags_tag_idx ON doc_tags (tenant_id, tag);

CREATE TABLE doc_links (
    tenant_id UUID NOT NULL,
    source    TEXT NOT NULL,
    target    TEXT NOT NULL,
    PRIMARY KEY (tenant_id, source, target),
    FOREIGN KEY (tenant_id, source) REFERENCES documents(tenant_id, doc_id) ON DELETE CASCADE
);
CREATE INDEX doc_links_target_idx ON doc_links (tenant_id, target);

-- Per-tenant audit chain. Matches `mytex-audit` field-for-field so the
-- same hash helper can verify the chain later; the only expansion is that
-- `actor` accepts `account:<uuid>` in addition to `owner` / `tok:<id>`.
CREATE TABLE audit_entries (
    tenant_id   UUID        NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    seq         BIGINT      NOT NULL,
    ts          TIMESTAMPTZ NOT NULL DEFAULT now(),
    actor       TEXT        NOT NULL,
    action      TEXT        NOT NULL,
    document_id TEXT,
    scope_used  TEXT[]      NOT NULL DEFAULT '{}',
    outcome     TEXT        NOT NULL CHECK (outcome IN ('ok','denied','error')),
    prev_hash   TEXT        NOT NULL,
    hash        TEXT        NOT NULL,
    PRIMARY KEY (tenant_id, seq)
);

-- MCP tokens scoped to a tenant. Shape mirrors `mytex-auth::StoredToken`,
-- but the secret is Argon2id-hashed at rest and looked up by the same
-- prefix trick used for user sessions.
CREATE TABLE mcp_tokens (
    id            TEXT        PRIMARY KEY,
    tenant_id     UUID        NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    issued_by     UUID        NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    label         TEXT        NOT NULL,
    token_prefix  TEXT        NOT NULL UNIQUE,
    token_hash    TEXT        NOT NULL,
    scope         TEXT[]      NOT NULL,
    mode          TEXT        NOT NULL CHECK (mode IN ('read','read_propose')),
    max_docs      INT         NOT NULL DEFAULT 20,
    max_bytes     BIGINT      NOT NULL DEFAULT 65536,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at    TIMESTAMPTZ NOT NULL,
    last_used_at  TIMESTAMPTZ,
    revoked_at    TIMESTAMPTZ
);
CREATE INDEX mcp_tokens_tenant_idx ON mcp_tokens (tenant_id);
