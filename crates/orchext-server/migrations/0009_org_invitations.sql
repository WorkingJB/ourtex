-- Phase 3 platform Slice 1 follow-up: email-pre-approval invitations.
--
-- An admin/owner adds an email + role to an org's invitation list.
-- When that email signs up, the signup flow detects the open
-- invitation, materializes the membership directly, and marks the
-- invitation redeemed — no awaiting-approval gate, no email
-- delivery. (D17a's full join-code redemption flow remains a
-- separate piece of work for when there's a driver beyond
-- in-house pre-approval.)
--
-- Email matching is case-insensitive — consistent with how
-- `accounts.email` is normalized to lowercase at signup. The
-- partial UNIQUE prevents duplicate open invites for the same
-- (org, email) pair while leaving redeemed history intact.

CREATE TABLE org_invitations (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      UUID        NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    email       TEXT        NOT NULL,
    role        TEXT        NOT NULL DEFAULT 'member'
                            CHECK (role IN ('owner','admin','org_editor','member')),
    invited_by  UUID        NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    invited_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    redeemed_at TIMESTAMPTZ,
    redeemed_by UUID        REFERENCES accounts(id) ON DELETE SET NULL
);

-- One open invite per (org, email). Allow duplicates once the prior
-- one is redeemed (the partial predicate gates on `redeemed_at`).
CREATE UNIQUE INDEX org_invitations_open_unique
    ON org_invitations (org_id, lower(email))
    WHERE redeemed_at IS NULL;

-- Signup hook lookup: "open invitations for this email, any org".
CREATE INDEX org_invitations_email_open_idx
    ON org_invitations (lower(email))
    WHERE redeemed_at IS NULL;
