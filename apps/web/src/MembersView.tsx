import { useEffect, useState } from "react";
import {
  api,
  ApiFailure,
  Invitation,
  MemberDetail,
  PendingDetail,
} from "./api";
import { Context } from "./OrgRail";

const ROLES = ["owner", "admin", "org_editor", "member"] as const;
type Role = (typeof ROLES)[number];

/// Members + pending-signups admin pane (Phase 3 platform Slice 1).
/// Only mounted when the active context is an org tenant AND the
/// caller's role is admin/owner — App.tsx gates the nav button.
///
/// One pane covers two server resources (`/orgs/:id/members` and
/// `/orgs/:id/pending`) because operationally they're the same job:
/// keep the org's roster in shape.
export function MembersView({ ctx }: { ctx: Context & { kind: "org" } }) {
  const [members, setMembers] = useState<MemberDetail[] | null>(null);
  const [pending, setPending] = useState<PendingDetail[] | null>(null);
  const [invitations, setInvitations] = useState<Invitation[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [showInvite, setShowInvite] = useState(false);
  const [inviteEmail, setInviteEmail] = useState("");
  const [inviteRole, setInviteRole] = useState<Role>("member");

  async function reload() {
    setError(null);
    try {
      const [m, p, i] = await Promise.all([
        api.orgMembers(ctx.orgId),
        api.orgPending(ctx.orgId, "pending"),
        api.orgInvitations(ctx.orgId, "open"),
      ]);
      setMembers(m.members);
      setPending(p.pending);
      setInvitations(i.invitations);
    } catch (e) {
      setError(e instanceof ApiFailure ? e.message : String(e));
    }
  }

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ctx.orgId]);

  async function changeRole(memberId: string, newRole: Role) {
    setBusy(memberId);
    setError(null);
    try {
      await api.orgMemberUpdate(ctx.orgId, memberId, newRole);
      await reload();
    } catch (e) {
      setError(e instanceof ApiFailure ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function remove(memberId: string) {
    if (!confirm("Remove this member from the organization?")) return;
    setBusy(memberId);
    setError(null);
    try {
      await api.orgMemberRemove(ctx.orgId, memberId);
      await reload();
    } catch (e) {
      setError(e instanceof ApiFailure ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function approve(accountId: string, role: Role) {
    setBusy(accountId);
    setError(null);
    try {
      await api.orgPendingApprove(ctx.orgId, accountId, role);
      await reload();
    } catch (e) {
      setError(e instanceof ApiFailure ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function reject(accountId: string) {
    setBusy(accountId);
    setError(null);
    try {
      await api.orgPendingReject(ctx.orgId, accountId);
      await reload();
    } catch (e) {
      setError(e instanceof ApiFailure ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function submitInvite() {
    const email = inviteEmail.trim();
    if (!email) return;
    setBusy(email);
    setError(null);
    try {
      await api.orgInvite(ctx.orgId, email, inviteRole);
      setInviteEmail("");
      setInviteRole("member");
      setShowInvite(false);
      await reload();
    } catch (e) {
      setError(e instanceof ApiFailure ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function revokeInvitation(id: string) {
    setBusy(id);
    setError(null);
    try {
      await api.orgInvitationDelete(ctx.orgId, id);
      await reload();
    } catch (e) {
      setError(e instanceof ApiFailure ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  const callerIsOwner = ctx.role === "owner";

  return (
    <div className="h-full overflow-auto p-6">
      <div className="max-w-4xl mx-auto space-y-8">
        <header className="flex items-center justify-between">
          <h1 className="text-xl font-semibold">Members of {ctx.name}</h1>
          <div className="flex items-center gap-3">
            <button
              onClick={() => setShowInvite((v) => !v)}
              className="text-sm px-3 py-1.5 rounded bg-brand-500 text-white hover:bg-brand-600"
            >
              {showInvite ? "Cancel" : "+ Add member"}
            </button>
            <button
              onClick={() => void reload()}
              className="text-xs text-neutral-500 hover:text-neutral-900"
            >
              Refresh
            </button>
          </div>
        </header>

        {showInvite && (
          <div className="bg-white border border-neutral-200 rounded-md p-4 space-y-3">
            <h2 className="text-sm font-medium text-neutral-700">
              Add member by email
            </h2>
            <p className="text-xs text-neutral-500">
              They&apos;ll join automatically when they sign up with this
              email — no email is sent. Until they sign up, you can revoke
              the invitation below.
            </p>
            <div className="flex gap-2">
              <input
                type="email"
                value={inviteEmail}
                onChange={(e) => setInviteEmail(e.target.value)}
                placeholder="alice@example.com"
                className="flex-1 px-3 py-1.5 border border-neutral-300 rounded text-sm"
              />
              <select
                value={inviteRole}
                onChange={(e) => setInviteRole(e.target.value as Role)}
                className="px-2 py-1.5 border border-neutral-300 rounded text-sm bg-white"
              >
                {ROLES.filter((r) => callerIsOwner || r !== "owner").map(
                  (r) => (
                    <option key={r} value={r}>
                      {r}
                    </option>
                  )
                )}
              </select>
              <button
                onClick={() => void submitInvite()}
                disabled={busy !== null || !inviteEmail.trim()}
                className="text-sm px-3 py-1.5 rounded bg-brand-500 text-white hover:bg-brand-600 disabled:opacity-50"
              >
                Invite
              </button>
            </div>
          </div>
        )}

        {error && (
          <div className="bg-red-50 border border-red-200 text-red-700 text-sm rounded-md p-3">
            {error}
          </div>
        )}

        <section>
          <h2 className="text-sm font-medium text-neutral-700 mb-2">
            Pending requests
            {pending && pending.length > 0 && (
              <span className="ml-2 text-xs bg-amber-100 text-amber-800 rounded-full px-2 py-0.5">
                {pending.length}
              </span>
            )}
          </h2>
          {pending === null ? (
            <p className="text-sm text-neutral-500">Loading…</p>
          ) : pending.length === 0 ? (
            <p className="text-sm text-neutral-500">
              No pending requests.
            </p>
          ) : (
            <ul className="bg-white border border-neutral-200 rounded-md divide-y divide-neutral-100">
              {pending.map((p) => (
                <PendingRow
                  key={p.id}
                  row={p}
                  busy={busy === p.account_id}
                  callerIsOwner={callerIsOwner}
                  onApprove={(role) => approve(p.account_id, role)}
                  onReject={() => reject(p.account_id)}
                />
              ))}
            </ul>
          )}
        </section>

        <section>
          <h2 className="text-sm font-medium text-neutral-700 mb-2">
            Open invitations
            {invitations && invitations.length > 0 && (
              <span className="ml-2 text-xs bg-blue-100 text-blue-800 rounded-full px-2 py-0.5">
                {invitations.length}
              </span>
            )}
          </h2>
          {invitations === null ? (
            <p className="text-sm text-neutral-500">Loading…</p>
          ) : invitations.length === 0 ? (
            <p className="text-sm text-neutral-500">
              No open invitations.
            </p>
          ) : (
            <ul className="bg-white border border-neutral-200 rounded-md divide-y divide-neutral-100">
              {invitations.map((inv) => (
                <li
                  key={inv.id}
                  className="flex items-center gap-3 px-4 py-3"
                >
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium truncate">
                      {inv.email}
                    </div>
                    <div className="text-xs text-neutral-500 truncate">
                      Will join as <strong>{inv.role}</strong> · invited{" "}
                      {new Date(inv.invited_at).toLocaleDateString()}
                    </div>
                  </div>
                  <button
                    onClick={() => void revokeInvitation(inv.id)}
                    disabled={busy === inv.id}
                    className="text-xs px-2 py-1 rounded border border-neutral-300 text-neutral-700 hover:bg-red-50 hover:text-red-700 hover:border-red-300 disabled:opacity-50"
                  >
                    Revoke
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section>
          <h2 className="text-sm font-medium text-neutral-700 mb-2">
            Members
          </h2>
          {members === null ? (
            <p className="text-sm text-neutral-500">Loading…</p>
          ) : (
            <ul className="bg-white border border-neutral-200 rounded-md divide-y divide-neutral-100">
              {members.map((m) => (
                <MemberRow
                  key={m.account_id}
                  row={m}
                  busy={busy === m.account_id}
                  callerIsOwner={callerIsOwner}
                  onChangeRole={(role) => changeRole(m.account_id, role)}
                  onRemove={() => remove(m.account_id)}
                />
              ))}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}

function PendingRow({
  row,
  busy,
  callerIsOwner,
  onApprove,
  onReject,
}: {
  row: PendingDetail;
  busy: boolean;
  callerIsOwner: boolean;
  onApprove: (role: Role) => void;
  onReject: () => void;
}) {
  const [role, setRole] = useState<Role>("member");
  return (
    <li className="flex items-center gap-3 px-4 py-3">
      <div className="flex-1 min-w-0">
        <div className="text-sm font-medium truncate">{row.display_name}</div>
        <div className="text-xs text-neutral-500 truncate">
          {row.email} · requested{" "}
          {new Date(row.requested_at).toLocaleDateString()}
        </div>
      </div>
      <RoleSelect
        value={role}
        onChange={setRole}
        callerIsOwner={callerIsOwner}
        disabled={busy}
      />
      <button
        onClick={() => onApprove(role)}
        disabled={busy}
        className="text-xs px-2 py-1 rounded bg-brand-500 text-white hover:bg-brand-600 disabled:opacity-50"
      >
        Approve
      </button>
      <button
        onClick={onReject}
        disabled={busy}
        className="text-xs px-2 py-1 rounded border border-neutral-300 text-neutral-700 hover:bg-neutral-50 disabled:opacity-50"
      >
        Reject
      </button>
    </li>
  );
}

function MemberRow({
  row,
  busy,
  callerIsOwner,
  onChangeRole,
  onRemove,
}: {
  row: MemberDetail;
  busy: boolean;
  callerIsOwner: boolean;
  onChangeRole: (role: Role) => void;
  onRemove: () => void;
}) {
  return (
    <li className="flex items-center gap-3 px-4 py-3">
      <div className="flex-1 min-w-0">
        <div className="text-sm font-medium truncate">{row.display_name}</div>
        <div className="text-xs text-neutral-500 truncate">{row.email}</div>
      </div>
      <RoleSelect
        value={row.role}
        onChange={onChangeRole}
        callerIsOwner={callerIsOwner}
        disabled={busy}
      />
      <button
        onClick={onRemove}
        disabled={busy}
        className="text-xs px-2 py-1 rounded border border-neutral-300 text-neutral-700 hover:bg-red-50 hover:text-red-700 hover:border-red-300 disabled:opacity-50"
      >
        Remove
      </button>
    </li>
  );
}

/// Role selector. `owner` is hidden unless the caller is themselves
/// an owner — server-side D11 says only owners can promote to or
/// demote from owner. The UI mirrors that to avoid surfacing a
/// guaranteed-403 control.
function RoleSelect({
  value,
  onChange,
  callerIsOwner,
  disabled,
}: {
  value: Role | string;
  onChange: (r: Role) => void;
  callerIsOwner: boolean;
  disabled: boolean;
}) {
  const visible: Role[] = callerIsOwner
    ? [...ROLES]
    : ROLES.filter((r) => r !== "owner");
  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value as Role)}
      disabled={disabled}
      className="text-xs border border-neutral-300 rounded px-2 py-1 bg-white"
    >
      {visible.map((r) => (
        <option key={r} value={r}>
          {r}
        </option>
      ))}
      {/* If the row's current role isn't in `visible` (e.g. a non-owner
          looking at an owner row), include it as a disabled option so
          the select doesn't collapse to the wrong default. */}
      {!visible.includes(value as Role) && (
        <option value={value} disabled>
          {value}
        </option>
      )}
    </select>
  );
}
