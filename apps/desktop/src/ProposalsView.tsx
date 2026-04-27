import { useEffect, useState } from "react";
import { api, Proposal } from "./api";

type FilterStatus = "pending" | "approved" | "rejected" | "all";

export function ProposalsView({ onMutated }: { onMutated?: () => void }) {
  const [filter, setFilter] = useState<FilterStatus>("pending");
  const [proposals, setProposals] = useState<Proposal[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  async function refresh() {
    try {
      setError(null);
      setProposals(await api.proposalList(filter));
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    setProposals(null);
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filter]);

  async function approve(p: Proposal) {
    setBusy(p.id);
    try {
      await api.proposalApprove(p.id);
      onMutated?.();
      await refresh();
    } catch (e) {
      setError(`approve ${p.id}: ${String(e)}`);
    } finally {
      setBusy(null);
    }
  }

  async function reject(p: Proposal) {
    setBusy(p.id);
    try {
      await api.proposalReject(p.id);
      await refresh();
    } catch (e) {
      setError(`reject ${p.id}: ${String(e)}`);
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="p-6 max-w-5xl mx-auto">
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-lg font-semibold">Proposals</h2>
        <div className="flex items-center gap-2">
          <FilterTabs value={filter} onChange={setFilter} />
          <button
            onClick={() => void refresh()}
            className="text-xs text-neutral-500 hover:text-neutral-900"
          >
            Refresh
          </button>
        </div>
      </div>

      {error && (
        <div className="mb-4 p-3 bg-red-50 text-red-700 text-sm rounded-lg border border-red-200">
          {error}
        </div>
      )}

      {proposals && proposals.length === 0 && (
        <div className="bg-white border border-neutral-200 rounded-lg p-8 text-center text-neutral-500 text-sm">
          {filter === "pending"
            ? "No proposals waiting for review. Agents holding a `read+propose` token can submit changes here."
            : `No ${filter} proposals.`}
        </div>
      )}

      <div className="space-y-3">
        {proposals?.map((p) => (
          <ProposalCard
            key={p.id}
            proposal={p}
            busy={busy === p.id}
            onApprove={() => approve(p)}
            onReject={() => reject(p)}
          />
        ))}
      </div>
    </div>
  );
}

function FilterTabs({
  value,
  onChange,
}: {
  value: FilterStatus;
  onChange: (s: FilterStatus) => void;
}) {
  const opts: { id: FilterStatus; label: string }[] = [
    { id: "pending", label: "Pending" },
    { id: "approved", label: "Approved" },
    { id: "rejected", label: "Rejected" },
    { id: "all", label: "All" },
  ];
  return (
    <div className="flex rounded-md border border-neutral-200 bg-white text-xs overflow-hidden">
      {opts.map((o) => (
        <button
          key={o.id}
          onClick={() => onChange(o.id)}
          className={
            "px-2.5 py-1 transition " +
            (value === o.id
              ? "bg-brand-50 text-brand-700 font-medium"
              : "text-neutral-600 hover:bg-neutral-50")
          }
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function ProposalCard({
  proposal,
  busy,
  onApprove,
  onReject,
}: {
  proposal: Proposal;
  busy: boolean;
  onApprove: () => void;
  onReject: () => void;
}) {
  const ageMs = Date.now() - new Date(proposal.created_at).getTime();
  return (
    <div className="bg-white border border-neutral-200 rounded-lg p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="font-mono text-sm text-neutral-800 truncate">
              {proposal.doc_id}
            </span>
            <StatusBadge status={proposal.status} />
            <span className="text-xs text-neutral-500">
              {formatAge(ageMs)} ago
            </span>
          </div>
          <div className="mt-1 text-xs text-neutral-500">
            from{" "}
            <span className="font-mono">{proposal.actor_token_label}</span>
            {proposal.actor_token_id && (
              <span className="text-neutral-400">
                {" "}
                ({proposal.actor_token_id})
              </span>
            )}
          </div>
          {proposal.reason && (
            <p className="mt-2 text-sm text-neutral-700 italic">
              "{proposal.reason}"
            </p>
          )}
        </div>
        {proposal.status === "pending" && (
          <div className="flex flex-col gap-1.5 shrink-0">
            <button
              disabled={busy}
              onClick={onApprove}
              className="px-3 py-1 text-xs rounded bg-brand-600 text-white hover:bg-brand-700 disabled:opacity-50"
            >
              {busy ? "…" : "Approve"}
            </button>
            <button
              disabled={busy}
              onClick={onReject}
              className="px-3 py-1 text-xs rounded border border-neutral-300 text-neutral-700 hover:bg-neutral-50 disabled:opacity-50"
            >
              Reject
            </button>
          </div>
        )}
      </div>

      <PatchPreview proposal={proposal} />

      {proposal.status !== "pending" && (
        <div className="mt-3 pt-3 border-t border-neutral-100 text-xs text-neutral-500">
          {proposal.status === "approved" ? "Approved" : "Rejected"}{" "}
          {proposal.decided_at &&
            new Date(proposal.decided_at).toLocaleString()}
          {proposal.applied_version && (
            <>
              {" · new version "}
              <span className="font-mono">
                {proposal.applied_version.slice(0, 16)}…
              </span>
            </>
          )}
          {proposal.decision_note && (
            <p className="mt-1 italic">{proposal.decision_note}</p>
          )}
        </div>
      )}
    </div>
  );
}

function StatusBadge({ status }: { status: Proposal["status"] }) {
  const cls =
    status === "pending"
      ? "bg-amber-100 text-amber-700"
      : status === "approved"
      ? "bg-green-100 text-green-700"
      : "bg-neutral-100 text-neutral-600";
  return (
    <span className={`text-[10px] px-1.5 py-0.5 rounded ${cls}`}>
      {status}
    </span>
  );
}

function PatchPreview({ proposal }: { proposal: Proposal }) {
  const { patch } = proposal;
  const hasFm = patch.frontmatter && Object.keys(patch.frontmatter).length > 0;
  const bodyOp =
    patch.body_replace != null
      ? { kind: "replace" as const, text: patch.body_replace }
      : patch.body_append != null
      ? { kind: "append" as const, text: patch.body_append }
      : null;

  if (!hasFm && !bodyOp) {
    return (
      <p className="mt-3 text-xs text-neutral-500 italic">
        Empty patch (no frontmatter or body changes).
      </p>
    );
  }
  return (
    <div className="mt-3 space-y-2">
      {hasFm && (
        <div>
          <div className="text-[10px] uppercase tracking-wider text-neutral-500 mb-1">
            Frontmatter merge
          </div>
          <pre className="text-xs bg-neutral-50 border border-neutral-200 rounded p-2 overflow-x-auto font-mono">
            {JSON.stringify(patch.frontmatter, null, 2)}
          </pre>
        </div>
      )}
      {bodyOp && (
        <div>
          <div className="text-[10px] uppercase tracking-wider text-neutral-500 mb-1">
            Body {bodyOp.kind === "replace" ? "replace" : "append"}
          </div>
          <pre className="text-xs bg-neutral-50 border border-neutral-200 rounded p-2 overflow-x-auto whitespace-pre-wrap">
            {bodyOp.text}
          </pre>
        </div>
      )}
    </div>
  );
}

function formatAge(ms: number): string {
  const sec = Math.round(ms / 1000);
  if (sec < 60) return `${sec}s`;
  const min = Math.round(sec / 60);
  if (min < 60) return `${min}m`;
  const hr = Math.round(min / 60);
  if (hr < 48) return `${hr}h`;
  const d = Math.round(hr / 24);
  return `${d}d`;
}
