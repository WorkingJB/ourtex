import { FormEvent, useState } from "react";
import { api, ApiFailure } from "./api";
import { saveSession, StoredSession } from "./session";

type Mode = "login" | "signup";

export function LoginView({
  onAuthenticated,
}: {
  onAuthenticated: (s: StoredSession) => void;
}) {
  const [mode, setMode] = useState<Mode>("login");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      const resp =
        mode === "login"
          ? await api.login(email, password)
          : await api.signup(email, password, displayName || undefined);
      const session: StoredSession = {
        token: resp.session.secret,
        accountId: resp.account.id,
        email: resp.account.email,
        displayName: resp.account.display_name,
        expiresAt: resp.session.expires_at,
      };
      saveSession(session);
      onAuthenticated(session);
    } catch (e) {
      setError(e instanceof ApiFailure ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="h-full flex items-center justify-center p-6">
      <form
        onSubmit={submit}
        className="w-full max-w-sm bg-white border border-neutral-200 rounded-lg p-6 shadow-sm"
      >
        <h1 className="text-xl font-semibold mb-1">Ourtex</h1>
        <p className="text-sm text-neutral-500 mb-5">
          {mode === "login" ? "Sign in to your workspace." : "Create an account."}
        </p>

        <label className="block text-sm mb-1">Email</label>
        <input
          type="email"
          autoComplete="email"
          required
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          className="w-full border border-neutral-300 rounded-md px-3 py-2 mb-3 text-sm"
        />

        <label className="block text-sm mb-1">Password</label>
        <input
          type="password"
          autoComplete={mode === "login" ? "current-password" : "new-password"}
          required
          minLength={mode === "signup" ? 12 : undefined}
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          className="w-full border border-neutral-300 rounded-md px-3 py-2 mb-3 text-sm"
        />

        {mode === "signup" && (
          <>
            <label className="block text-sm mb-1">
              Display name <span className="text-neutral-400">(optional)</span>
            </label>
            <input
              type="text"
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
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
          {busy
            ? "Working…"
            : mode === "login"
              ? "Sign in"
              : "Create account"}
        </button>

        <button
          type="button"
          onClick={() => {
            setMode(mode === "login" ? "signup" : "login");
            setError(null);
          }}
          className="w-full text-sm text-neutral-600 hover:text-neutral-900 mt-3"
        >
          {mode === "login"
            ? "Need an account? Sign up"
            : "Already have an account? Sign in"}
        </button>
      </form>
    </div>
  );
}
