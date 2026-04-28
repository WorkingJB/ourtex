import { useEffect, useMemo, useState } from "react";
import { Organization } from "./api";
import { Context } from "./OrgRail";
import { TokensView } from "./TokensView";
import { AuditView } from "./AuditView";
import { MembersView } from "./MembersView";
import { OrgSettingsView } from "./OrgSettingsView";
import { TeamsView } from "./TeamsView";
import { AccountSettingsView } from "./AccountSettingsView";

type Tab = "account" | "members" | "teams" | "org" | "tokens" | "audit";

/// Settings hub. Wraps the per-feature views under a single top-level
/// nav slot so the right-side nav stays sparse (Docs / Settings).
/// Tabs are role/context-gated:
///   * `account` — remote personal workspace only (per-account profile
///     + change password live there because that's where users
///     instinctively look; the underlying endpoints are
///     session-scoped, but local vaults have no remote account at all
///     so the tab is hidden there).
///   * `members`, `org` — admin/owner of an org context only
///   * `tokens`, `audit` — every workspace (including local)
/// Local vaults therefore only show Tokens + Audit.
export function SettingsView({
  ctx,
  onOrgUpdated,
  onMutated,
}: {
  ctx: Context;
  /// Called when the user saves changes from the Organization tab.
  /// Layout uses this to live-update the rail so the new name/logo
  /// shows immediately without a refetch.
  onOrgUpdated?: (org: Organization) => void;
  onMutated?: () => void | Promise<void>;
}) {
  const isOrg = ctx.kind === "org";
  const isPersonal = ctx.kind === "personal";
  const isAdmin =
    isOrg && (ctx.role === "owner" || ctx.role === "admin");

  const availableTabs = useMemo<Tab[]>(() => {
    const tabs: Tab[] = [];
    if (isPersonal) {
      tabs.push("account");
    }
    if (isOrg && isAdmin) {
      tabs.push("members");
    }
    if (isOrg) {
      tabs.push("teams");
    }
    if (isOrg && isAdmin) {
      tabs.push("org");
    }
    tabs.push("tokens");
    tabs.push("audit");
    return tabs;
  }, [isOrg, isPersonal, isAdmin]);

  const [tab, setTab] = useState<Tab>(availableTabs[0]);

  // If the active tab disappears (admin demoted, context switch), drop
  // back to the first available one.
  useEffect(() => {
    if (!availableTabs.includes(tab)) {
      setTab(availableTabs[0]);
    }
  }, [availableTabs, tab]);

  return (
    <div className="h-full flex flex-col min-h-0">
      <div className="border-b border-neutral-200 bg-white px-4 flex items-center gap-1">
        {availableTabs.map((t) => (
          <SubTab
            key={t}
            label={LABELS[t]}
            active={tab === t}
            onClick={() => setTab(t)}
          />
        ))}
      </div>
      <div className="flex-1 min-h-0 overflow-auto">
        {tab === "account" && isPersonal && (
          <AccountSettingsView workspaceId={ctx.workspaceId} />
        )}
        {tab === "members" && isOrg && (
          <MembersView ctx={ctx as Context & { kind: "org" }} />
        )}
        {tab === "teams" && isOrg && (
          <TeamsView ctx={ctx as Context & { kind: "org" }} />
        )}
        {tab === "org" && isOrg && (
          <OrgSettingsView
            ctx={ctx as Context & { kind: "org" }}
            onUpdated={(org) => onOrgUpdated?.(org)}
          />
        )}
        {tab === "tokens" && (
          <TokensView ctx={ctx} onMutated={onMutated} />
        )}
        {tab === "audit" && <AuditView />}
      </div>
    </div>
  );
}

const LABELS: Record<Tab, string> = {
  account: "Account",
  members: "Members",
  teams: "Teams",
  org: "Organization",
  tokens: "Tokens",
  audit: "Audit",
};

function SubTab({
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
        "px-3 py-2.5 text-sm border-b-2 transition -mb-px " +
        (active
          ? "border-brand-500 text-brand-700 font-medium"
          : "border-transparent text-neutral-600 hover:text-neutral-900")
      }
    >
      {label}
    </button>
  );
}
