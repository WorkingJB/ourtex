import { OrgMembership, OrgsListResponse, WorkspaceInfo } from "./api";

/// One entry in the desktop rail. Unlike the web — which lists tenants
/// at a single server — desktop's rail spans every workspace the user
/// has registered: local vaults plus any remote tenants (personal +
/// org) across one or more servers.
export type Context =
  | {
      kind: "local";
      workspaceId: string;
      name: string;
    }
  | {
      kind: "personal";
      workspaceId: string;
      tenantId: string;
      serverUrl: string;
      name: string;
    }
  | {
      kind: "org";
      workspaceId: string;
      tenantId: string;
      serverUrl: string;
      orgId: string;
      name: string;
      logoUrl: string | null;
      role: OrgMembership["role"];
    };

/// Per-server org list snapshot keyed by `server_url`, so the rail
/// builder can decide whether a remote workspace is a personal vault
/// or an org membership and pick up org metadata (org_id, logo, role).
export type OrgsByServer = Map<string, OrgsListResponse>;

/// Build the rail's `Context[]` from the workspace list plus any
/// per-server org lookups. Order: local vaults first (registry order),
/// then remote workspaces (registry order).
///
/// A remote workspace whose `tenant_id` matches an `OrgMembership.tenant_id`
/// returned by its server becomes an `org` context; the rest become
/// `personal` (the personal vault is implicitly every-non-org tenant
/// since `/v1/orgs` only returns org tenants).
///
/// Workspaces whose server is missing from `orgsByServer` (lookup
/// failed, session expired) gracefully degrade to `personal` so the
/// rail stays usable — the user can still open them; metadata
/// refresh happens next time the lookup succeeds.
export function buildContexts(
  workspaces: WorkspaceInfo[],
  orgsByServer: OrgsByServer
): Context[] {
  const result: Context[] = [];

  // Local vaults first.
  for (const w of workspaces) {
    if (w.kind !== "local") continue;
    result.push({ kind: "local", workspaceId: w.id, name: w.name });
  }

  // Remote workspaces: route by tenant_id through the org map.
  for (const w of workspaces) {
    if (w.kind !== "remote") continue;
    if (!w.server_url || !w.tenant_id) continue;
    const orgs = orgsByServer.get(w.server_url);
    const orgMembership = orgs?.memberships.find(
      (m) => m.tenant_id === w.tenant_id
    );
    if (orgMembership) {
      result.push({
        kind: "org",
        workspaceId: w.id,
        tenantId: w.tenant_id,
        serverUrl: w.server_url,
        orgId: orgMembership.org_id,
        name: orgMembership.name,
        logoUrl: orgMembership.logo_url,
        role: orgMembership.role,
      });
    } else {
      result.push({
        kind: "personal",
        workspaceId: w.id,
        tenantId: w.tenant_id,
        serverUrl: w.server_url,
        name: w.name,
      });
    }
  }

  return result;
}

/// Slack-style left rail. One badge per context. Active highlight on
/// the current selection. The "+ Add" affordance opens whatever
/// dialog the parent passes — desktop wires it to the existing
/// `VaultPicker` so the same affordance can either pick a local
/// folder or connect to a remote server (D17f).
export function OrgRail({
  contexts,
  activeWorkspaceId,
  onSelect,
  onAdd,
}: {
  contexts: Context[];
  activeWorkspaceId: string | null;
  onSelect: (ctx: Context) => void;
  onAdd: () => void;
}) {
  return (
    <nav
      aria-label="Workspace picker"
      className="w-14 shrink-0 border-r border-neutral-200 bg-white flex flex-col items-center gap-2 py-3"
    >
      {contexts.map((ctx) => (
        <ContextBadge
          key={ctx.workspaceId}
          ctx={ctx}
          active={ctx.workspaceId === activeWorkspaceId}
          onClick={() => onSelect(ctx)}
        />
      ))}
      <button
        type="button"
        onClick={onAdd}
        title="Add workspace"
        aria-label="Add workspace"
        className="w-9 h-9 rounded-lg flex items-center justify-center text-lg font-light text-neutral-500 border border-dashed border-neutral-300 hover:border-neutral-500 hover:text-neutral-900 transition mt-1"
      >
        +
      </button>
    </nav>
  );
}

function ContextBadge({
  ctx,
  active,
  onClick,
}: {
  ctx: Context;
  active: boolean;
  onClick: () => void;
}) {
  const initials = badgeInitials(ctx);
  const tooltip = badgeTooltip(ctx);
  return (
    <button
      type="button"
      onClick={onClick}
      title={tooltip}
      aria-label={tooltip}
      aria-current={active ? "true" : undefined}
      className={
        "w-9 h-9 rounded-lg flex items-center justify-center text-xs font-semibold transition " +
        (active
          ? "bg-brand-500 text-white ring-2 ring-offset-1 ring-brand-300"
          : "bg-neutral-100 text-neutral-700 hover:bg-neutral-200")
      }
    >
      {ctx.kind === "org" && ctx.logoUrl ? (
        <img
          src={ctx.logoUrl}
          alt=""
          className="w-7 h-7 rounded object-cover"
        />
      ) : (
        initials
      )}
    </button>
  );
}

function badgeTooltip(ctx: Context): string {
  switch (ctx.kind) {
    case "local":
      return `${ctx.name} (local)`;
    case "personal":
      return `Personal — ${hostOf(ctx.serverUrl)}`;
    case "org":
      return ctx.name;
  }
}

function badgeInitials(ctx: Context): string {
  if (ctx.kind === "personal") return "P";
  if (ctx.kind === "local") {
    return ctx.name.trim()[0]?.toUpperCase() ?? "L";
  }
  // First letter of the first two words, uppercase. "Acme Co" → "AC",
  // "Acme" → "A".
  const words = ctx.name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return "?";
  if (words.length === 1) return words[0][0]!.toUpperCase();
  return (words[0][0]! + words[1][0]!).toUpperCase();
}

function hostOf(serverUrl: string): string {
  try {
    return new URL(serverUrl).host;
  } catch {
    return serverUrl;
  }
}
