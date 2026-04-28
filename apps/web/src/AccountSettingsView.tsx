import { FormEvent, useState } from "react";
import { api, ApiFailure } from "./api";
import { SessionProfile } from "./session";

/// Per-account settings — display name + change password. Lives under
/// the personal-workspace Settings tab because the "where do I edit
/// my own profile" instinct points there; the underlying endpoints
/// (`PATCH /v1/auth/account`, `POST /v1/auth/password`) are scoped to
/// the session, not the active tenant.
export function AccountSettingsView({
  profile,
  onProfileUpdated,
}: {
  profile: SessionProfile;
  /// Called after a successful display-name update so the rail and
  /// header avatar reflect the new name without a refetch.
  onProfileUpdated: (profile: SessionProfile) => void;
}) {
  return (
    <div className="p-6 max-w-xl space-y-6">
      <h2 className="text-lg font-semibold">Account</h2>
      <DisplayNameForm
        profile={profile}
        onProfileUpdated={onProfileUpdated}
      />
      <hr className="border-neutral-200" />
      <ChangePasswordForm />
    </div>
  );
}

function DisplayNameForm({
  profile,
  onProfileUpdated,
}: {
  profile: SessionProfile;
  onProfileUpdated: (profile: SessionProfile) => void;
}) {
  const [name, setName] = useState(profile.displayName);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState<number | null>(null);

  const dirty = name.trim() !== profile.displayName && name.trim().length > 0;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!dirty) return;
    setBusy(true);
    setError(null);
    try {
      const updated = await api.accountUpdate(name.trim());
      onProfileUpdated({ ...profile, displayName: updated.display_name });
      setSavedAt(Date.now());
    } catch (e) {
      setError(e instanceof ApiFailure ? e.message : String(e));
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

function ChangePasswordForm() {
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
      await api.passwordChange(current, next);
      setCurrent("");
      setNext("");
      setConfirm("");
      setSavedAt(Date.now());
    } catch (e) {
      setError(e instanceof ApiFailure ? e.message : String(e));
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
