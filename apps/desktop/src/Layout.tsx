import { useCallback, useEffect, useState } from "react";
import { api, VaultInfo } from "./api";
import { DocumentsView } from "./DocumentsView";
import { OnboardingView } from "./OnboardingView";
import { TokensView } from "./TokensView";
import { AuditView } from "./AuditView";
import { ProposalsView } from "./ProposalsView";
import { WorkspaceSwitcher } from "./WorkspaceSwitcher";

type View = "documents" | "onboarding" | "proposals" | "tokens" | "audit";

type Counts = {
  documents: number;
  proposals: number;
  tokens: number;
  audit: number;
};

export function Layout({
  vault,
  onSwitched,
}: {
  vault: VaultInfo;
  onSwitched: (v: VaultInfo) => void;
}) {
  // Auto-open onboarding on first-run (empty vault).
  const [view, setView] = useState<View>(
    vault.document_count === 0 ? "onboarding" : "documents"
  );
  const [counts, setCounts] = useState<Counts>({
    documents: vault.document_count,
    proposals: 0,
    tokens: 0,
    audit: 0,
  });

  const refreshCounts = useCallback(async () => {
    const [docs, proposals, tokens, audit] = await Promise.all([
      api.docList().then((l) => l.length),
      api
        .proposalList("pending")
        .then((l) => l.length)
        .catch(() => 0),
      api
        .tokenList()
        .then((l) => l.length)
        .catch(() => 0),
      api
        .auditList(1)
        .then((p) => p.total)
        .catch(() => 0),
    ]);
    setCounts({ documents: docs, proposals, tokens, audit });
  }, []);

  useEffect(() => {
    // Refresh counts whenever the view changes or on mount — cheap, and
    // keeps the sidebar honest after edits in any tab.
    void refreshCounts();
  }, [view, refreshCounts]);

  // When the workspace switches, hop to Documents and refresh counts
  // against the newly-active vault.
  useEffect(() => {
    setView(vault.document_count === 0 ? "onboarding" : "documents");
    setCounts({
      documents: vault.document_count,
      proposals: 0,
      tokens: 0,
      audit: 0,
    });
    void refreshCounts();
  }, [vault.workspace_id, vault.document_count, refreshCounts]);

  return (
    <div className="h-full flex flex-col">
      <header className="border-b border-neutral-200 bg-white px-4 h-12 flex items-center gap-3">
        <span className="font-semibold">Orchext</span>
        <span className="text-neutral-400">·</span>
        <WorkspaceSwitcher active={vault} onSwitched={onSwitched} />
      </header>
      <div className="flex flex-1 min-h-0">
        <nav className="w-44 border-r border-neutral-200 bg-white p-2 flex flex-col gap-1">
          <NavBtn
            label="Documents"
            count={counts.documents}
            active={view === "documents"}
            onClick={() => setView("documents")}
          />
          <NavBtn
            label="Onboarding"
            count={0}
            active={view === "onboarding"}
            onClick={() => setView("onboarding")}
          />
          <NavBtn
            label="Proposals"
            count={counts.proposals}
            active={view === "proposals"}
            onClick={() => setView("proposals")}
          />
          <NavBtn
            label="Tokens"
            count={counts.tokens}
            active={view === "tokens"}
            onClick={() => setView("tokens")}
          />
          <NavBtn
            label="Audit"
            count={counts.audit}
            active={view === "audit"}
            onClick={() => setView("audit")}
          />
        </nav>
        <main key={vault.workspace_id} className="flex-1 min-w-0 bg-neutral-50">
          {view === "documents" && (
            <DocumentsView onMutated={refreshCounts} />
          )}
          {view === "onboarding" && (
            <OnboardingView
              onComplete={async () => {
                await refreshCounts();
                setView("documents");
              }}
            />
          )}
          {view === "proposals" && <ProposalsView onMutated={refreshCounts} />}
          {view === "tokens" && <TokensView onMutated={refreshCounts} />}
          {view === "audit" && <AuditView />}
        </main>
      </div>
    </div>
  );
}

function NavBtn({
  label,
  count,
  active,
  onClick,
}: {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={
        "flex items-center justify-between text-left px-3 py-2 rounded-md text-sm transition " +
        (active
          ? "bg-brand-50 text-brand-700 font-medium"
          : "text-neutral-700 hover:bg-neutral-100")
      }
    >
      <span>{label}</span>
      <span
        className={
          "text-xs px-1.5 py-0.5 rounded " +
          (active ? "bg-white text-brand-700" : "bg-neutral-100 text-neutral-600")
        }
      >
        {count}
      </span>
    </button>
  );
}
