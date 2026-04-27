import { useEffect, useState } from "react";
import { api, ApiFailure, CryptoState, Membership } from "./api";
import { SessionProfile } from "./session";
import { LoginView } from "./LoginView";
import { TenantPicker } from "./TenantPicker";
import { DocumentsView } from "./DocumentsView";
import { TokensView } from "./TokensView";
import { AuditView } from "./AuditView";
import { ProposalsView } from "./ProposalsView";
import { UnlockView } from "./UnlockView";
import { ConsentView } from "./ConsentView";
import { Heartbeat, startHeartbeat } from "./heartbeat";

type View = "documents" | "proposals" | "tokens" | "audit";

// Top-level auth state. `bootstrapping` is the brief window between
// app load and the `/v1/auth/me` probe completing — don't render
// LoginView until we know the cookie is no good, or we'll flash the
// form on every reload.
type AuthState =
  | { kind: "bootstrapping" }
  | { kind: "anonymous" }
  | { kind: "authenticated"; profile: SessionProfile };

// Tri-state: what does this browser hold for the current tenant?
//   "checking"  — waiting on /vault/crypto
//   "ready"     — plaintext tenant OR we unlocked locally (contentKey held)
//   "locked"    — seeded tenant, no local key; UnlockView next
type WorkspaceState =
  | { kind: "checking" }
  | { kind: "ready"; contentKey: string | null }
  | { kind: "locked" };

// OAuth consent surface lives at its own path. We detect it once at
// mount and short-circuit the normal app shell so the user lands on
// the consent prompt without seeing the tenant picker / docs view.
const IS_CONSENT_ROUTE = window.location.pathname === "/oauth/authorize";

export default function App() {
  if (IS_CONSENT_ROUTE) return <ConsentView />;
  return <MainApp />;
}

function MainApp() {
  const [auth, setAuth] = useState<AuthState>({ kind: "bootstrapping" });
  const [tenant, setTenant] = useState<Membership | null>(null);
  const [workspace, setWorkspace] = useState<WorkspaceState>({
    kind: "checking",
  });
  const [heartbeat, setHeartbeatHandle] = useState<Heartbeat | null>(null);
  const [view, setView] = useState<View>("documents");

  // Probe the cookie-backed session on mount. 200 ⇒ authenticated;
  // anything else ⇒ no session, fall through to login.
  useEffect(() => {
    let cancelled = false;
    api
      .me()
      .then((resp) => {
        if (cancelled) return;
        setAuth({
          kind: "authenticated",
          profile: {
            accountId: resp.account.id,
            email: resp.account.email,
            displayName: resp.account.display_name,
          },
        });
      })
      .catch(() => {
        if (!cancelled) setAuth({ kind: "anonymous" });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Hop back to documents when the tenant changes so a "tokens" or
  // "audit" selection from a previous tenant doesn't stick.
  useEffect(() => {
    setView("documents");
  }, [tenant?.tenant_id]);

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
    setAuth({ kind: "anonymous" });
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

  if (auth.kind === "bootstrapping") {
    return (
      <div className="h-full flex items-center justify-center text-neutral-500">
        Loading…
      </div>
    );
  }
  if (auth.kind === "anonymous") {
    return (
      <LoginView
        onAuthenticated={(profile) =>
          setAuth({ kind: "authenticated", profile })
        }
      />
    );
  }
  if (!tenant) {
    return <TenantPicker onPicked={setTenant} />;
  }

  return (
    <div className="h-full flex flex-col">
      <header className="border-b border-neutral-200 bg-white px-4 h-12 flex items-center gap-3">
        <span className="font-semibold">Orchext</span>
        <span className="text-neutral-400">·</span>
        <button
          onClick={switchTenant}
          className="text-sm text-neutral-700 hover:text-neutral-900"
        >
          {tenant.name}
        </button>
        <div className="ml-auto flex items-center gap-3 text-sm text-neutral-600">
          <span>{auth.profile.displayName}</span>
          <button
            onClick={logout}
            className="text-neutral-500 hover:text-neutral-900"
          >
            Sign out
          </button>
        </div>
      </header>
      <div className="flex flex-1 min-h-0">
        {workspace.kind === "ready" && (
          <nav className="w-44 border-r border-neutral-200 bg-white p-2 flex flex-col gap-1">
            <NavBtn
              label="Documents"
              active={view === "documents"}
              onClick={() => setView("documents")}
            />
            <NavBtn
              label="Proposals"
              active={view === "proposals"}
              onClick={() => setView("proposals")}
            />
            <NavBtn
              label="Tokens"
              active={view === "tokens"}
              onClick={() => setView("tokens")}
            />
            <NavBtn
              label="Audit"
              active={view === "audit"}
              onClick={() => setView("audit")}
            />
          </nav>
        )}
        <main className="flex-1 min-w-0 bg-neutral-50">
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
          {workspace.kind === "ready" && view === "documents" && (
            <DocumentsView tenant={tenant} />
          )}
          {workspace.kind === "ready" && view === "proposals" && (
            <ProposalsView tenant={tenant} />
          )}
          {workspace.kind === "ready" && view === "tokens" && (
            <TokensView tenant={tenant} />
          )}
          {workspace.kind === "ready" && view === "audit" && (
            <AuditView tenant={tenant} />
          )}
        </main>
      </div>
    </div>
  );
}

function NavBtn({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={
        "text-left px-3 py-2 rounded-md text-sm transition " +
        (active
          ? "bg-brand-50 text-brand-700 font-medium"
          : "text-neutral-700 hover:bg-neutral-100")
      }
    >
      {label}
    </button>
  );
}
