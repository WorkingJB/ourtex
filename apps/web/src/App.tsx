import { useEffect, useState } from "react";
import {
  api,
  ApiFailure,
  CryptoState,
  Membership,
  OrgsResponse,
  PendingSignup,
} from "./api";
import { SessionProfile } from "./session";
import { LoginView } from "./LoginView";
import { DocumentsView } from "./DocumentsView";
import { TokensView } from "./TokensView";
import { AuditView } from "./AuditView";
import { ProposalsView } from "./ProposalsView";
import { UnlockView } from "./UnlockView";
import { ConsentView } from "./ConsentView";
import { Heartbeat, startHeartbeat } from "./heartbeat";
import { buildContexts, Context, OrgRail } from "./OrgRail";
import { AwaitingApprovalView } from "./AwaitingApprovalView";
import { MembersView } from "./MembersView";
import { OrgSettingsView } from "./OrgSettingsView";

type View = "documents" | "proposals" | "tokens" | "audit" | "members" | "settings";

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

// Contexts state — drives the org rail. `loading` covers the initial
// fetch; `ready` is the resolved list. The rail is always rendered in
// `ready`, even when the list is empty (the awaiting-approval gate
// covers that branch in a follow-up commit).
type ContextsState =
  | { kind: "loading" }
  | { kind: "ready"; contexts: Context[]; pending: PendingSignup[] };

// OAuth consent surface lives at its own path. We detect it once at
// mount and short-circuit the normal app shell so the user lands on
// the consent prompt without seeing the rail / docs view.
const IS_CONSENT_ROUTE = window.location.pathname === "/oauth/authorize";

export default function App() {
  if (IS_CONSENT_ROUTE) return <ConsentView />;
  return <MainApp />;
}

function MainApp() {
  const [auth, setAuth] = useState<AuthState>({ kind: "bootstrapping" });
  const [contexts, setContexts] = useState<ContextsState>({ kind: "loading" });
  const [active, setActive] = useState<Context | null>(null);
  const [workspace, setWorkspace] = useState<WorkspaceState>({
    kind: "checking",
  });
  const [heartbeat, setHeartbeatHandle] = useState<Heartbeat | null>(null);
  const [view, setView] = useState<View>("documents");
  /// Optional doc-id filter for the Proposals view. Set when the
  /// user clicks "Review" on the inline banner in the Documents view
  /// so they land on that doc's pending proposals; cleared when the
  /// active context changes or they navigate elsewhere.
  const [proposalsFocus, setProposalsFocus] = useState<string | null>(null);

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

  // Once authenticated, fetch the rail data: tenants() for the
  // personal vault, orgsList() for any org memberships + pending
  // signup count. Run in parallel — both are cookie-authed reads.
  useEffect(() => {
    if (auth.kind !== "authenticated") return;
    let cancelled = false;
    setContexts({ kind: "loading" });
    Promise.all([api.tenants(), api.orgsList()])
      .then(([t, o]: [{ memberships: Membership[] }, OrgsResponse]) => {
        if (cancelled) return;
        const ctxs = buildContexts(t.memberships, o.memberships);
        setContexts({
          kind: "ready",
          contexts: ctxs,
          pending: o.pending,
        });
        // Default-select first context (personal first by buildContexts
        // ordering) once on first load.
        if (ctxs.length > 0) setActive((prev) => prev ?? ctxs[0]);
      })
      .catch(() => {
        if (!cancelled) {
          setContexts({ kind: "ready", contexts: [], pending: [] });
        }
      });
    return () => {
      cancelled = true;
    };
  }, [auth.kind]);

  // Hop back to documents when the active context changes so a
  // "tokens" or "audit" selection from a previous one doesn't stick.
  // Also drop any proposals-focus from the previous context.
  useEffect(() => {
    setView("documents");
    setProposalsFocus(null);
  }, [active?.tenantId]);

  // Clear the proposals doc-focus whenever the user manually
  // navigates away from Proposals — keeps it from hanging around
  // for the next deep-link.
  useEffect(() => {
    if (view !== "proposals") setProposalsFocus(null);
  }, [view]);

  // Classify the tenant whenever the caller flips to a new one. Seeded
  // tenants without a local content key land in UnlockView; plaintext
  // and already-keyed tenants go straight through.
  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    setWorkspace({ kind: "checking" });
    api
      .cryptoState(active.tenantId)
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
  }, [active]);

  // Heartbeat lifecycle: one handle per unlocked workspace. Cancelled
  // whenever the active context or session changes, or on teardown.
  useEffect(() => {
    if (workspace.kind !== "ready" || !workspace.contentKey || !active) {
      return;
    }
    const hb = startHeartbeat(active.tenantId, workspace.contentKey);
    setHeartbeatHandle(hb);
    return () => {
      hb.stop();
      setHeartbeatHandle(null);
    };
  }, [workspace, active]);

  async function logout() {
    heartbeat?.stop();
    try {
      if (active) await api.revokeSessionKey(active.tenantId);
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
    setActive(null);
    setContexts({ kind: "loading" });
    setWorkspace({ kind: "checking" });
  }

  function selectContext(ctx: Context) {
    if (ctx.tenantId === active?.tenantId) return;
    heartbeat?.stop();
    if (active) {
      api.revokeSessionKey(active.tenantId).catch(() => undefined);
    }
    setActive(ctx);
    setWorkspace({ kind: "checking" });
  }

  /// "+ Add organization" affordance on the rail (D17f). Prompts for
  /// a name, calls POST /v1/orgs (caller becomes owner), then re-
  /// fetches the rail data so the new org appears and switches into
  /// it. Errors fall back to a window.alert — the rail's tight space
  /// doesn't have a great surface for inline error rendering yet.
  async function createNewOrg() {
    const raw = window.prompt("Name your new organization:");
    if (raw === null) return;
    const name = raw.trim();
    if (!name) return;
    try {
      const created = await api.orgCreate(name);
      const o = await api.orgsList();
      const t = await api.tenants();
      const ctxs = buildContexts(t.memberships, o.memberships);
      setContexts({
        kind: "ready",
        contexts: ctxs,
        pending: o.pending,
      });
      const newCtx = ctxs.find(
        (c) => c.kind === "org" && c.orgId === created.id
      );
      if (newCtx) {
        heartbeat?.stop();
        if (active) {
          api.revokeSessionKey(active.tenantId).catch(() => undefined);
        }
        setActive(newCtx);
        setWorkspace({ kind: "checking" });
      }
    } catch (e) {
      const msg = e instanceof ApiFailure ? e.message : String(e);
      window.alert(`Could not create organization: ${msg}`);
    }
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
  if (contexts.kind === "loading") {
    return (
      <div className="h-full flex items-center justify-center text-neutral-500">
        Loading workspaces…
      </div>
    );
  }
  // Awaiting-approval gate (D17d): account has no org membership +
  // a pending row. Personal vault is intentionally not surfaced —
  // "connecting to the server" implies "connecting to the org" until
  // an admin approves.
  const orgContexts = contexts.contexts.filter((c) => c.kind === "org");
  if (orgContexts.length === 0 && contexts.pending.length > 0) {
    return (
      <AwaitingApprovalView
        pending={contexts.pending}
        email={auth.profile.email}
        onSignOut={logout}
      />
    );
  }
  if (contexts.contexts.length === 0 || !active) {
    // Edge state: no orgs, no pending. Most likely a self-hosted
    // server that has no first-user yet, or an admin-removed account
    // with no re-application — log out so the user can re-sign-up.
    return (
      <div className="h-full flex flex-col items-center justify-center text-neutral-500 gap-3">
        <span>No workspaces available.</span>
        <button
          onClick={logout}
          className="text-sm text-neutral-500 hover:text-neutral-900 underline"
        >
          Sign out
        </button>
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col">
      <header className="border-b border-neutral-200 bg-white px-4 h-12 flex items-center gap-3">
        <span className="font-semibold">Orchext</span>
        <span className="text-neutral-400">·</span>
        <span className="text-sm text-neutral-700">
          {active.kind === "personal" ? "Personal" : active.name}
        </span>
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
        <OrgRail
          contexts={contexts.contexts}
          activeTenantId={active.tenantId}
          onSelect={selectContext}
          onCreateOrg={createNewOrg}
        />
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
            {/* Admin/owner-only org-management views. Only mounted
                when the active context is an org tenant — personal
                vault has no members or org-settings concept. */}
            {active.kind === "org" &&
              (active.role === "owner" || active.role === "admin") && (
                <>
                  <div className="mt-3 mb-1 text-[10px] uppercase tracking-wider text-neutral-400 px-3">
                    Admin
                  </div>
                  <NavBtn
                    label="Members"
                    active={view === "members"}
                    onClick={() => setView("members")}
                  />
                  <NavBtn
                    label="Settings"
                    active={view === "settings"}
                    onClick={() => setView("settings")}
                  />
                </>
              )}
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
              tenant={contextToMembership(active)}
              onUnlocked={(contentKey) =>
                setWorkspace({ kind: "ready", contentKey })
              }
            />
          )}
          {workspace.kind === "ready" && view === "documents" && (
            <DocumentsView
              tenant={contextToMembership(active)}
              onSwitchToProposals={(docId) => {
                setProposalsFocus(docId);
                setView("proposals");
              }}
            />
          )}
          {workspace.kind === "ready" && view === "proposals" && (
            <ProposalsView
              tenant={contextToMembership(active)}
              focusDocId={proposalsFocus}
            />
          )}
          {workspace.kind === "ready" && view === "tokens" && (
            <TokensView tenant={contextToMembership(active)} />
          )}
          {workspace.kind === "ready" && view === "audit" && (
            <AuditView tenant={contextToMembership(active)} />
          )}
          {workspace.kind === "ready" &&
            view === "members" &&
            active.kind === "org" && <MembersView ctx={active} />}
          {workspace.kind === "ready" &&
            view === "settings" &&
            active.kind === "org" && (
              <OrgSettingsView
                ctx={active}
                onUpdated={(updated) => {
                  // Live-update the rail entry so the new name/logo
                  // shows immediately without refetching.
                  setContexts((prev) => {
                    if (prev.kind !== "ready") return prev;
                    const next = prev.contexts.map((c) =>
                      c.kind === "org" && c.orgId === updated.id
                        ? {
                            ...c,
                            name: updated.name,
                            logoUrl: updated.logo_url,
                          }
                        : c
                    );
                    return { ...prev, contexts: next };
                  });
                  setActive((prev) =>
                    prev && prev.kind === "org" && prev.orgId === updated.id
                      ? {
                          ...prev,
                          name: updated.name,
                          logoUrl: updated.logo_url,
                        }
                      : prev
                  );
                }}
              />
            )}
        </main>
      </div>
    </div>
  );
}

/// Existing views (DocumentsView, TokensView, etc.) accept a
/// `Membership`. The rail-based shell keeps a richer `Context`, so
/// adapt back at the boundary. Once the views move to `Context`
/// natively this can go.
function contextToMembership(ctx: Context): Membership {
  return {
    tenant_id: ctx.tenantId,
    name: ctx.kind === "personal" ? ctx.name : ctx.name,
    kind: ctx.kind === "personal" ? "personal" : "org",
    role: ctx.kind === "personal" ? "owner" : ctx.role,
    created_at: "",
  };
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
