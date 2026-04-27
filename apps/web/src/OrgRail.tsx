import { Membership, OrgMembership } from "./api";

/// Active context shown in the rail. `personal` corresponds to the
/// caller's `kind='personal'` tenant; `org` is one of their org
/// memberships. We carry tenantId in both so the rest of the app can
/// continue to key off it without caring which kind it is.
export type Context =
  | {
      kind: "personal";
      tenantId: string;
      name: string;
    }
  | {
      kind: "org";
      tenantId: string;
      orgId: string;
      name: string;
      logoUrl: string | null;
      role: OrgMembership["role"];
    };

export function buildContexts(
  memberships: Membership[],
  orgs: OrgMembership[]
): Context[] {
  // Personal tenant first; the user's vault is the default landing
  // surface. Then orgs in the order the server returned them
  // (oldest-first by membership).
  const personal = memberships.find((m) => m.kind === "personal");
  const result: Context[] = [];
  if (personal) {
    result.push({
      kind: "personal",
      tenantId: personal.tenant_id,
      name: personal.name,
    });
  }
  for (const o of orgs) {
    result.push({
      kind: "org",
      tenantId: o.tenant_id,
      orgId: o.org_id,
      name: o.name,
      logoUrl: o.logo_url,
      role: o.role,
    });
  }
  return result;
}

/// Slack-style left rail. One circle per context the user can switch
/// to. Always visible while the app is authenticated, regardless of
/// whether the active tenant is locked or unlocked — switching to
/// another context is a way out of a locked vault.
export function OrgRail({
  contexts,
  activeTenantId,
  onSelect,
}: {
  contexts: Context[];
  activeTenantId: string | null;
  onSelect: (ctx: Context) => void;
}) {
  return (
    <nav
      aria-label="Organization picker"
      className="w-14 shrink-0 border-r border-neutral-200 bg-white flex flex-col items-center gap-2 py-3"
    >
      {contexts.map((ctx) => (
        <ContextBadge
          key={ctx.tenantId}
          ctx={ctx}
          active={ctx.tenantId === activeTenantId}
          onClick={() => onSelect(ctx)}
        />
      ))}
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
  const tooltip = ctx.kind === "personal" ? "Personal" : ctx.name;
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

function badgeInitials(ctx: Context): string {
  if (ctx.kind === "personal") return "P";
  // First letter of the first two words, uppercase. "Acme Co" → "AC",
  // "Acme" → "A".
  const words = ctx.name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return "?";
  if (words.length === 1) return words[0][0]!.toUpperCase();
  return (words[0][0]! + words[1][0]!).toUpperCase();
}
