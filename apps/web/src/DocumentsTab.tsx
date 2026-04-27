import { useEffect, useState } from "react";
import { Membership } from "./api";
import { DocumentsView } from "./DocumentsView";
import { ProposalsView } from "./ProposalsView";

type SubTab = "documents" | "proposals";

/// Wraps DocumentsView + ProposalsView with a top sub-nav so they
/// live as siblings under a single top-level "Documents" slot.
/// Proposals against context docs feels naturally close to the docs
/// themselves; the standalone tab survived only as a triage queue.
///
/// `proposalsFocus` lets the inline "Review →" banner on a doc deep-
/// link into the Proposals tab pre-filtered to that doc's pending
/// proposals.
export function DocumentsTab({
  tenant,
  proposalsFocus,
  onSetProposalsFocus,
}: {
  tenant: Membership;
  proposalsFocus: string | null;
  onSetProposalsFocus: (docId: string | null) => void;
}) {
  const [subtab, setSubtab] = useState<SubTab>("documents");

  // Hop to the Proposals subtab whenever a focus is set externally
  // (the banner click), and back to Documents when it clears.
  useEffect(() => {
    if (proposalsFocus) setSubtab("proposals");
  }, [proposalsFocus]);

  // Reset subtab on context switch so a per-tenant proposals view
  // doesn't bleed into a different one.
  useEffect(() => {
    setSubtab("documents");
  }, [tenant.tenant_id]);

  return (
    <div className="h-full flex flex-col min-h-0">
      <div className="border-b border-neutral-200 bg-white px-4 flex items-center gap-1">
        <SubTabBtn
          label="Documents"
          active={subtab === "documents"}
          onClick={() => setSubtab("documents")}
        />
        <SubTabBtn
          label="Proposals"
          active={subtab === "proposals"}
          onClick={() => {
            setSubtab("proposals");
            // Clearing the focus when the user clicks the tab
            // header lets them browse all pending proposals from
            // here.
            onSetProposalsFocus(null);
          }}
        />
      </div>
      <div className="flex-1 min-h-0">
        {subtab === "documents" && (
          <DocumentsView
            tenant={tenant}
            onSwitchToProposals={(docId) => {
              onSetProposalsFocus(docId);
            }}
          />
        )}
        {subtab === "proposals" && (
          <ProposalsView tenant={tenant} focusDocId={proposalsFocus} />
        )}
      </div>
    </div>
  );
}

function SubTabBtn({
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
