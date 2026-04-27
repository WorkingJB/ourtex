import { useEffect, useState } from "react";
import { api, Organization } from "./api";
import { Context } from "./OrgRail";

/// Org settings pane (Phase 3 platform Slice 1). Admin/owner only —
/// gated by Layout.
///
/// `allowed_domains` is rendered read-only with a "available when
/// email infra ships" note, per D17e: the column lands in the schema
/// now but the auto-join code path won't fire until SMTP + email
/// verification are wired.
export function OrgSettingsView({
  ctx,
  onUpdated,
}: {
  ctx: Context & { kind: "org" };
  onUpdated: (org: Organization) => void;
}) {
  const [org, setOrg] = useState<Organization | null>(null);
  const [name, setName] = useState("");
  const [logoUrl, setLogoUrl] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [savedAt, setSavedAt] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    setOrg(null);
    setError(null);
    api
      .orgGet(ctx.workspaceId, ctx.orgId)
      .then((o) => {
        if (cancelled) return;
        setOrg(o);
        setName(o.name);
        setLogoUrl(o.logo_url ?? "");
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [ctx.orgId, ctx.workspaceId]);

  async function save(e: React.FormEvent) {
    e.preventDefault();
    if (!org) return;
    const trimmedName = name.trim();
    if (trimmedName.length === 0) {
      setError("Name must not be empty.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const updated = await api.orgUpdate(ctx.workspaceId, org.id, {
        name: trimmedName,
        logo_url: logoUrl.trim() === "" ? null : logoUrl.trim(),
      });
      setOrg(updated);
      setSavedAt(Date.now());
      onUpdated(updated);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  if (!org && !error) {
    return (
      <div className="h-full flex items-center justify-center text-neutral-500">
        Loading settings…
      </div>
    );
  }

  return (
    <div className="h-full overflow-auto p-6">
      <div className="max-w-2xl mx-auto space-y-6">
        <header>
          <h1 className="text-xl font-semibold">Organization settings</h1>
        </header>

        {error && (
          <div className="bg-red-50 border border-red-200 text-red-700 text-sm rounded-md p-3">
            {error}
          </div>
        )}

        {org && (
          <form
            onSubmit={save}
            className="bg-white border border-neutral-200 rounded-md p-5 space-y-4"
          >
            <Field label="Name">
              <input
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                className="w-full border border-neutral-300 rounded px-3 py-2 text-sm"
                disabled={busy}
              />
            </Field>

            <Field label="Logo URL">
              <input
                type="url"
                value={logoUrl}
                onChange={(e) => setLogoUrl(e.target.value)}
                placeholder="https://example.com/logo.png"
                className="w-full border border-neutral-300 rounded px-3 py-2 text-sm"
                disabled={busy}
              />
              <p className="text-xs text-neutral-500 mt-1">
                Shown as the org&apos;s avatar in the left rail.
              </p>
            </Field>

            <Field label="Allowed domains">
              <input
                type="text"
                value={renderAllowedDomains(org.allowed_domains)}
                disabled
                className="w-full border border-neutral-200 rounded px-3 py-2 text-sm bg-neutral-50 text-neutral-500"
              />
              <p className="text-xs text-neutral-500 mt-1">
                Available when email infra ships — auto-join from a
                matching corporate email currently still goes through
                the approval queue.
              </p>
            </Field>

            <div className="flex items-center gap-3 pt-2">
              <button
                type="submit"
                disabled={busy}
                className="text-sm px-3 py-1.5 rounded bg-brand-500 text-white hover:bg-brand-600 disabled:opacity-50"
              >
                {busy ? "Saving…" : "Save changes"}
              </button>
              {savedAt !== null && !busy && (
                <span className="text-xs text-neutral-500">Saved.</span>
              )}
            </div>
          </form>
        )}
      </div>
    </div>
  );
}

function renderAllowedDomains(value: unknown): string {
  if (Array.isArray(value)) return value.join(", ");
  return "";
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <label className="block">
      <span className="block text-sm font-medium text-neutral-700 mb-1">
        {label}
      </span>
      {children}
    </label>
  );
}
