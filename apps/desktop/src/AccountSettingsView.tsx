import { FormEvent, useEffect, useState } from "react";
import { api, AccountInfo } from "./api";

/// Per-account settings — display name + change password — shown in
/// the personal-workspace Settings hub for a remote desktop
/// connection. The underlying endpoints (`PATCH /v1/auth/account`,
/// `POST /v1/auth/password`) live on the server, so this is a no-op
/// for local vaults; the parent view gates the Account tab on
/// `ctx.kind === "personal"`.
export function AccountSettingsView({ workspaceId }: { workspaceId: string }) {
  const [account, setAccount] = useState<AccountInfo | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setAccount(null);
    setLoadError(null);
    api
      .authMe(workspaceId)
      .then((r) => {
        if (!cancelled) setAccount(r.account);
      })
      .catch((e) => {
        if (!cancelled) setLoadError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [workspaceId]);

  return (
    <div className="p-6 max-w-xl space-y-6">
      <h2 className="text-lg font-semibold">Account</h2>
      {loadError && (
        <div className="text-sm text-red-600">{loadError}</div>
      )}
      {!account && !loadError && (
        <div className="text-sm text-neutral-500">Loading…</div>
      )}
      {account && (
        <>
          <DisplayNameForm
            workspaceId={workspaceId}
            account={account}
            onUpdated={setAccount}
          />
          <hr className="border-neutral-200" />
          <ChangePasswordForm workspaceId={workspaceId} />
        </>
      )}
    </div>
  );
}

function DisplayNameForm({
  workspaceId,
  account,
  onUpdated,
}: {
  workspaceId: string;
  account: AccountInfo;
  onUpdated: (account: AccountInfo) => void;
}) {
  const [name, setName] = useState(account.display_name);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState<number | null>(null);

  useEffect(() => {
    setName(account.display_name);
  }, [account.display_name]);

  const dirty =
    name.trim() !== account.display_name && name.trim().length > 0;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!dirty) return;
    setBusy(true);
    setError(null);
    try {
      const updated = await api.authAccountUpdate(workspaceId, name.trim());
      onUpdated(updated);
      setSavedAt(Date.now());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className="space-y-3">
      <div>
        <label className="block text-sm font-medium mb-1">Display name</label>
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          className="w-full border border-neutral-300 rounded px-3 py-2 text-sm"
          disabled={busy}
        />
        <p className="text-xs text-neutral-500 mt-1">
          Shown to teammates on shared docs, comments, and audit entries.
        </p>
      </div>
      {error && (
        <div className="text-sm text-red-600" role="alert">
          {error}
        </div>
      )}
      <div className="flex items-center gap-3">
        <button
          type="submit"
          disabled={!dirty || busy}
          className="bg-brand-600 hover:bg-brand-700 disabled:opacity-50 text-white text-sm font-medium px-3 py-1.5 rounded"
        >
          {busy ? "Saving…" : "Save"}
        </button>
        {savedAt && !dirty && (
          <span className="text-xs text-neutral-500">Saved.</span>
        )}
      </div>
    </form>
  );
}

function ChangePasswordForm({ workspaceId }: { workspaceId: string }) {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState<number | null>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    if (next !== confirm) {
      setError("New passwords do not match.");
      return;
    }
    setBusy(true);
    try {
      await api.authPasswordChange(workspaceId, current, next);
      setCurrent("");
      setNext("");
      setConfirm("");
      setSavedAt(Date.now());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className="space-y-3">
      <h3 className="text-sm font-medium">Change password</h3>
      <div>
        <label className="block text-sm mb-1">Current password</label>
        <input
          type="password"
          autoComplete="current-password"
          required
          value={current}
          onChange={(e) => setCurrent(e.target.value)}
          className="w-full border border-neutral-300 rounded px-3 py-2 text-sm"
          disabled={busy}
        />
      </div>
      <div>
        <label className="block text-sm mb-1">New password</label>
        <input
          type="password"
          autoComplete="new-password"
          required
          minLength={8}
          value={next}
          onChange={(e) => setNext(e.target.value)}
          className="w-full border border-neutral-300 rounded px-3 py-2 text-sm"
          disabled={busy}
        />
      </div>
      <div>
        <label className="block text-sm mb-1">Confirm new password</label>
        <input
          type="password"
          autoComplete="new-password"
          required
          minLength={8}
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
          className="w-full border border-neutral-300 rounded px-3 py-2 text-sm"
          disabled={busy}
        />
      </div>
      {error && (
        <div className="text-sm text-red-600" role="alert">
          {error}
        </div>
      )}
      <div className="flex items-center gap-3">
        <button
          type="submit"
          disabled={busy || !current || !next || !confirm}
          className="bg-brand-600 hover:bg-brand-700 disabled:opacity-50 text-white text-sm font-medium px-3 py-1.5 rounded"
        >
          {busy ? "Updating…" : "Change password"}
        </button>
        {savedAt && (
          <span className="text-xs text-neutral-500">Password updated.</span>
        )}
      </div>
    </form>
  );
}
