import { FormEvent, useEffect, useState } from "react";
import { api, ApiFailure, CryptoState, Membership } from "./api";
import { crypto } from "./crypto";

// Seeding (fresh tenant) and unlock (seeded tenant) share enough form
// state to stay in one component. The server decides which path the
// caller is authorized for — seed is owner/admin-only (401 otherwise).

export function UnlockView({
  tenant,
  onUnlocked,
}: {
  tenant: Membership;
  onUnlocked: (contentKeyWire: string) => void;
}) {
  const [state, setState] = useState<CryptoState | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [passphrase, setPassphrase] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setState(null);
    setLoadError(null);
    api
      .cryptoState(tenant.tenant_id)
      .then(setState)
      .catch((e) =>
        setLoadError(e instanceof ApiFailure ? e.message : String(e))
      );
  }, [tenant.tenant_id]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!state) return;
    setError(null);
    setBusy(true);
    try {
      let contentKeyWire: string;
      if (state.seeded) {
        if (!state.kdf_salt || !state.wrapped_content_key) {
          throw new Error("server reported seeded without salt/wrapped key");
        }
        contentKeyWire = await crypto.unwrapContentKey(
          state.wrapped_content_key,
          passphrase,
          state.kdf_salt
        );
      } else {
        if (passphrase !== confirm) {
          throw new Error("passphrases do not match");
        }
        const saltWire = await crypto.generateSalt();
        contentKeyWire = await crypto.generateContentKey();
        const wrappedWire = await crypto.wrapContentKey(
          contentKeyWire,
          passphrase,
          saltWire
        );
        await api.initCrypto(tenant.tenant_id, saltWire, wrappedWire);
      }
      await api.publishSessionKey(tenant.tenant_id, contentKeyWire);
      onUnlocked(contentKeyWire);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  if (loadError) {
    return <Centered>{loadError}</Centered>;
  }
  if (!state) {
    return <Centered>Checking workspace…</Centered>;
  }

  const seeding = !state.seeded;

  return (
    <div className="h-full flex items-center justify-center p-6">
      <form
        onSubmit={submit}
        className="w-full max-w-sm bg-white border border-neutral-200 rounded-lg p-6 shadow-sm"
      >
        <h2 className="text-lg font-semibold mb-1">
          {seeding ? "Set a workspace passphrase" : "Unlock workspace"}
        </h2>
        <p className="text-sm text-neutral-500 mb-4">
          {seeding
            ? "This passphrase encrypts every document in the workspace. It can't be recovered if lost."
            : `Enter the passphrase for ${tenant.name}.`}
        </p>

        <label className="block text-sm mb-1">Passphrase</label>
        <input
          type="password"
          required
          minLength={8}
          autoFocus
          value={passphrase}
          onChange={(e) => setPassphrase(e.target.value)}
          className="w-full border border-neutral-300 rounded-md px-3 py-2 mb-3 text-sm"
        />

        {seeding && (
          <>
            <label className="block text-sm mb-1">Confirm passphrase</label>
            <input
              type="password"
              required
              minLength={8}
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
              className="w-full border border-neutral-300 rounded-md px-3 py-2 mb-3 text-sm"
            />
          </>
        )}

        {error && (
          <div className="text-sm text-red-600 mb-3" role="alert">
            {error}
          </div>
        )}

        <button
          type="submit"
          disabled={busy}
          className="w-full bg-brand-600 hover:bg-brand-700 disabled:opacity-60 text-white text-sm font-medium py-2 rounded-md"
        >
          {busy ? "Working…" : seeding ? "Seed workspace" : "Unlock"}
        </button>
      </form>
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
