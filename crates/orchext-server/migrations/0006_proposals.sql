-- Phase 2b.5 schema: agent-issued change proposals (`context.propose`).
--
-- An agent with `mode = 'read_propose'` calls `context_propose` and the
-- patch lands here in `pending`. Nothing in `documents` moves until a
-- session-authed reviewer (the issuing user, or a workspace admin) hits
-- the approve endpoint, which applies the patch under the same base-
-- version optimistic-concurrency check `documents.write` already uses.
--
-- `patch` is stored as JSONB so the apply path can deserialise into the
-- same `Patch` enum the MCP layer accepts; `base_version` is the doc
-- version the agent saw at propose time. The `actor_*` columns let the
-- review UI show "from agent X (issued by Y)" without joining to
-- `mcp_tokens` (whose row may have been revoked since).

CREATE TABLE proposals (
    id                  TEXT        PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    doc_id              TEXT        NOT NULL,
    base_version        TEXT        NOT NULL,
    patch               JSONB       NOT NULL,
    reason              TEXT,
    status              TEXT        NOT NULL CHECK (status IN ('pending','approved','rejected'))
                                    DEFAULT 'pending',
    actor_token_id      TEXT        REFERENCES mcp_tokens(id) ON DELETE SET NULL,
    actor_token_label   TEXT        NOT NULL,
    actor_account_id    UUID        REFERENCES accounts(id) ON DELETE SET NULL,
    decided_by          UUID        REFERENCES accounts(id) ON DELETE SET NULL,
    decided_at          TIMESTAMPTZ,
    decision_note       TEXT,
    applied_version     TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Reviewer queries: "all pending proposals for this tenant, newest first".
CREATE INDEX proposals_tenant_status_idx
    ON proposals (tenant_id, status, created_at DESC);

-- Doc-scoped queries (e.g. "history for this doc").
CREATE INDEX proposals_tenant_doc_idx
    ON proposals (tenant_id, doc_id, created_at DESC);
