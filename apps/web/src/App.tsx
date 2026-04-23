import { useEffect, useState } from "react";
import { api, ApiFailure, CryptoState, Membership } from "./api";
import { clearSession, loadSession, StoredSession } from "./session";
import { LoginView } from "./LoginView";
import { TenantPicker } from "./TenantPicker";
import { DocumentsView } from "./DocumentsView";
import { UnlockView } from "./UnlockView";
import { Heartbeat, startHeartbeat } from "./heartbeat";

// Tri-state: what does this browser hold for the current tenant?
//   "checking"  — waiting on /vault/crypto
//   "ready"     — plaintext tenant OR we unlocked locally (contentKey held)
//   "locked"    — seeded tenant, no local key; UnlockView next
type WorkspaceState =
  | { kind: "checking" }
  | { kind: "ready"; contentKey: string | null }
  | { kind: "locked" };

export default function App() {
  const [session, setSession] = useState<StoredSession | null>(() =>
    loadSession()
  );
  const [tenant, setTenant] = useState<Membership | null>(null);
  const [workspace, setWorkspace] = useState<WorkspaceState>({
    kind: "checking",
  });
  const [heartbeat, setHeartbeatHandle] = useState<Heartbeat | null>(null);

  // Classify the tenant whenever the caller flips to a new one. Seeded
  // tenants without a local content key land in UnlockView; plaintext
  // and already-keyed tenants go straight through.
  useEffect(() => {
    if (!tenant) return;
    let cancelled = false;
    setWorkspace({ kind: "checking" });
    api
      .cryptoState(tenant.tenant_id)
      .then((state: CryptoState) => {
        if (cancelled) return;
        setWorkspace(
          state.seeded ? { kind: "locked" } : { kind: "ready", contentKey: null }
        );
      })
      .catch(() => {
        if (!cancelled) setWorkspace({ kind: "locked" });
      });
    return () => {
      cancelled = true;
    };
  }, [tenant]);

  // Heartbeat lifecycle: one handle per unlocked workspace. Cancelled
  // whenever the tenant or session changes, or on teardown.
  useEffect(() => {
    if (workspace.kind !== "ready" || !workspace.contentKey || !tenant) {
      return;
    }
    const hb = startHeartbeat(tenant.tenant_id, workspace.contentKey);
    setHeartbeatHandle(hb);
    return () => {
      hb.stop();
      setHeartbeatHandle(null);
    };
  }, [workspace, tenant]);

  async function logout() {
    heartbeat?.stop();
    try {
      if (tenant) await api.revokeSessionKey(tenant.tenant_id);
    } catch {
      // best-effort; session is about to be revoked anyway
    }
    try {
      await api.logout();
    } catch (e) {
      if (!(e instanceof ApiFailure) || e.status !== 401) {
        console.warn("logout failed", e);
      }
    }
    clearSession();
    setSession(null);
    setTenant(null);
    setWorkspace({ kind: "checking" });
  }

  function switchTenant() {
    heartbeat?.stop();
    if (tenant) {
      api.revokeSessionKey(tenant.tenant_id).catch(() => undefined);
    }
    setTenant(null);
    setWorkspace({ kind: "checking" });
  }

  if (!session) {
    return <LoginView onAuthenticated={setSession} />;
  }
  if (!tenant) {
    return <TenantPicker onPicked={setTenant} />;
  }

  return (
    <div className="h-full flex flex-col">
      <header className="border-b border-neutral-200 bg-white px-4 h-12 flex items-center gap-3">
        <span className="font-semibold">Ourtex</span>
        <span className="text-neutral-400">·</span>
        <button
          onClick={switchTenant}
          className="text-sm text-neutral-700 hover:text-neutral-900"
        >
          {tenant.name}
        </button>
        <div className="ml-auto flex items-center gap-3 text-sm text-neutral-600">
          <span>{session.displayName}</span>
          <button
            onClick={logout}
            className="text-neutral-500 hover:text-neutral-900"
          >
            Sign out
          </button>
        </div>
      </header>
      <main className="flex-1 min-h-0">
        {workspace.kind === "checking" && (
          <div className="h-full flex items-center justify-center text-neutral-500">
            Checking workspace…
          </div>
        )}
        {workspace.kind === "locked" && (
          <UnlockView
            tenant={tenant}
            onUnlocked={(contentKey) =>
              setWorkspace({ kind: "ready", contentKey })
            }
          />
        )}
        {workspace.kind === "ready" && <DocumentsView tenant={tenant} />}
      </main>
    </div>
  );
}
