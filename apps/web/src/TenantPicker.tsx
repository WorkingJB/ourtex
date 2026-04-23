import { useEffect, useState } from "react";
import { api, ApiFailure, Membership } from "./api";

export function TenantPicker({
  onPicked,
}: {
  onPicked: (m: Membership) => void;
}) {
  const [memberships, setMemberships] = useState<Membership[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .tenants()
      .then((r) => setMemberships(r.memberships))
      .catch((e) => setError(e instanceof ApiFailure ? e.message : String(e)));
  }, []);

  if (error) {
    return <Centered>{error}</Centered>;
  }
  if (!memberships) {
    return <Centered>Loading workspaces…</Centered>;
  }
  if (memberships.length === 0) {
    return <Centered>No workspaces yet.</Centered>;
  }
  if (memberships.length === 1) {
    // Auto-pick the single workspace — matches the personal-account
    // default. The UI only surfaces a chooser for multi-membership.
    onPicked(memberships[0]);
    return <Centered>Opening {memberships[0].name}…</Centered>;
  }

  return (
    <div className="h-full flex items-center justify-center p-6">
      <div className="w-full max-w-md">
        <h2 className="text-lg font-semibold mb-4">Pick a workspace</h2>
        <ul className="flex flex-col gap-2">
          {memberships.map((m) => (
            <li key={m.tenant_id}>
              <button
                onClick={() => onPicked(m)}
                className="w-full text-left bg-white border border-neutral-200 hover:border-brand-500 rounded-md px-4 py-3"
              >
                <div className="font-medium">{m.name}</div>
                <div className="text-xs text-neutral-500">
                  {m.kind} · {m.role}
                </div>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

function Centered({ children }: { children: React.ReactNode }) {
  return (
    <div className="h-full flex items-center justify-center text-neutral-500">
      {children}
    </div>
  );
}
