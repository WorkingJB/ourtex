import { PendingSignup } from "./api";

/// Gate shown to users who have no org membership but at least one
/// pending_signups row. Phase 3 platform D17d: every connection is
/// admin-approved at launch, so this is the steady-state UX for any
/// account between signup and approval.
///
/// Personal vault is intentionally NOT reachable from this state —
/// "connecting to the server" implies "connecting to the org", and
/// the gate exists to keep unapproved accounts from navigating
/// anywhere until an admin lets them in. Once approved, the rail-
/// based UI takes over.
export function AwaitingApprovalView({
  pending,
  email,
  onSignOut,
}: {
  pending: PendingSignup[];
  email: string;
  onSignOut: () => void;
}) {
  const single = pending.length === 1 ? pending[0] : null;
  return (
    <div className="h-full flex flex-col items-center justify-center bg-neutral-50 px-6">
      <div className="max-w-md w-full bg-white border border-neutral-200 rounded-lg p-6 shadow-sm">
        <h2 className="text-lg font-semibold mb-2">Awaiting approval</h2>
        <p className="text-sm text-neutral-600 mb-4">
          {single ? (
            <>
              Your request to join <strong>{single.org_name}</strong> is
              pending review. An admin will approve or deny it shortly;
              you&apos;ll have access as soon as that happens.
            </>
          ) : (
            <>
              Your requests to join the following organizations are
              pending review.
            </>
          )}
        </p>
        {!single && (
          <ul className="text-sm text-neutral-700 space-y-1 mb-4">
            {pending.map((p) => (
              <li key={p.id}>
                • <strong>{p.org_name}</strong>{" "}
                <span className="text-xs text-neutral-500">
                  · requested {new Date(p.requested_at).toLocaleDateString()}
                </span>
              </li>
            ))}
          </ul>
        )}
        <p className="text-xs text-neutral-500 mb-4">
          Signed in as {email}.
        </p>
        <button
          onClick={onSignOut}
          className="text-sm text-neutral-500 hover:text-neutral-900"
        >
          Sign out
        </button>
      </div>
    </div>
  );
}
