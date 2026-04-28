import { useEffect, useMemo, useState } from "react";
import {
  api,
  ApiFailure,
  ListEntry,
  Membership,
  ORG_VISIBILITIES,
  PERSONAL_VISIBILITIES,
  SEED_TYPES,
  TeamSummary,
} from "./api";
import { Context } from "./OrgRail";
import { buildSource, DocDetail, parseSource } from "./docSource";
import { RichTextEditor } from "./RichTextEditor";

type Load<T> =
  | { state: "loading" }
  | { state: "error"; message: string }
  | { state: "ready"; data: T };

/// Section toggle in the org workspace's Documents pane:
///   "mine"  → visibility=private docs (My notes for [Org])
///   "org"   → visibility=org docs (the org's shared context)
///   "all"   → both, default
type Section = "all" | "mine" | "org";

function errMessage(e: unknown): string {
  return e instanceof ApiFailure ? e.message : String(e);
}

export function DocumentsView({
  tenant,
  ctx,
  onSwitchToProposals,
}: {
  tenant: Membership;
  /// Active rail context. We only need it to derive the org_id for
  /// the teams fetch — kept optional so personal vault callers can
  /// continue to omit it without churn.
  ctx?: Context;
  /// Hook for the inline "N pending proposals → Review" banner. App
  /// uses this to flip the view to Proposals and pre-focus the
  /// filter on the doc the user came from.
  onSwitchToProposals?: (docId: string) => void;
}) {
  const isOrg = tenant.kind === "org";
  const [entries, setEntries] = useState<Load<ListEntry[]>>({ state: "loading" });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [typeFilter, setTypeFilter] = useState<string | null>(null);
  const [section, setSection] = useState<Section>("all");
  /// Team filter applied server-side via `?team_id=…`. When set, the
  /// section filter is forced to "all" — `mine`/`org` filter on
  /// visibility, and team docs are neither, so applying both would
  /// always render an empty list.
  const [teamFilter, setTeamFilter] = useState<string | null>(null);
  const [detail, setDetail] = useState<Load<DocDetail> | null>(null);
  const [creating, setCreating] = useState(false);
  // Teams visible to the viewer in this org context. Drives the
  // visibility=team option in the doc editor (only shown if the
  // viewer can write to at least one team) and the team picker
  // dropdown when the user picks `team` visibility.
  const [teams, setTeams] = useState<TeamSummary[]>([]);
  /// Pending-proposal count keyed by doc_id, for the inline banner.
  /// Refreshed alongside the doc list so approvals from a Proposals
  /// session reflect immediately on return.
  const [pendingByDoc, setPendingByDoc] = useState<Record<string, number>>({});

  async function refreshProposalCounts() {
    try {
      const resp = await api.proposalsList(tenant.tenant_id, "pending");
      const counts: Record<string, number> = {};
      for (const p of resp.proposals) {
        counts[p.doc_id] = (counts[p.doc_id] ?? 0) + 1;
      }
      setPendingByDoc(counts);
    } catch {
      // Best-effort. Don't fail the docs view on a proposals fetch error.
    }
  }

  async function refreshList() {
    try {
      const list = await api.docList(tenant.tenant_id, { teamId: teamFilter });
      setEntries({ state: "ready", data: list.entries });
    } catch (e) {
      setEntries({ state: "error", message: errMessage(e) });
    }
    void refreshProposalCounts();
  }

  useEffect(() => {
    setEntries({ state: "loading" });
    setSelectedId(null);
    setDetail(null);
    setCreating(false);
    setTeamFilter(null);
    setSection("all");
    void refreshList();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tenant.tenant_id]);

  // teamFilter changes alone re-fetch but don't reset selection state,
  // so the user can switch teams without losing the selected section.
  useEffect(() => {
    void refreshList();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [teamFilter]);

  // Fetch teams for the active org. Used by the doc editor — the
  // visibility=team option and team picker depend on this list.
  useEffect(() => {
    if (!isOrg || !ctx || ctx.kind !== "org") {
      setTeams([]);
      return;
    }
    let cancelled = false;
    api
      .teamsList(ctx.orgId)
      .then((r) => {
        if (!cancelled) setTeams(r.teams);
      })
      .catch(() => {
        if (!cancelled) setTeams([]);
      });
    return () => {
      cancelled = true;
    };
  }, [isOrg, ctx]);

  useEffect(() => {
    if (!selectedId) {
      setDetail(null);
      return;
    }
    let cancelled = false;
    setDetail({ state: "loading" });
    api
      .docRead(tenant.tenant_id, selectedId)
      .then((d) => {
        if (cancelled) return;
        try {
          const parsed = parseSource(d.source, {
            version: d.version,
            updated_at: d.updated_at,
          });
          parsed.team_id = d.team_id ?? null;
          setDetail({ state: "ready", data: parsed });
        } catch (e) {
          setDetail({ state: "error", message: errMessage(e) });
        }
      })
      .catch((e) => {
        if (!cancelled) setDetail({ state: "error", message: errMessage(e) });
      });
    return () => {
      cancelled = true;
    };
  }, [selectedId, tenant.tenant_id]);

  const allItems = entries.state === "ready" ? entries.data : [];
  // Section pre-filter applies before type filter so the "Types" sidebar
  // counts reflect only docs in the active section.
  const items = useMemo(() => {
    if (!isOrg || section === "all") return allItems;
    if (section === "mine") return allItems.filter((i) => i.visibility === "private");
    if (section === "org") return allItems.filter((i) => i.visibility === "org");
    return allItems;
  }, [allItems, isOrg, section]);
  const types = useMemo(() => {
    const present = new Set<string>(items.map((i) => i.type_));
    for (const t of SEED_TYPES) present.add(t);
    return Array.from(present).sort();
  }, [items]);

  const visible = typeFilter ? items.filter((i) => i.type_ === typeFilter) : items;

  // Default visibility for a "+ New" doc, computed from the active
  // section so the user isn't fighting the form. In the org-context
  // section, we assume the user is creating shared org content; in
  // "My context" or personal vault, default to private.
  const defaultVisibilityForNew: string =
    isOrg && section === "org" ? "org" : "private";

  return (
    <div className="flex h-full min-h-0">
      {/* Section sidebar — only in org workspace. Personal vault has
          one effective section, so no nav needed. Types moved to a
          dropdown in the doc list header (one filter per layer). */}
      {isOrg && (
        <aside className="w-44 border-r border-neutral-200 bg-white overflow-y-auto">
          <div className="p-2">
            <div className="text-xs uppercase tracking-wider text-neutral-500 mb-1 px-1">
              Section
            </div>
            <SectionBtn
              label="All"
              active={section === "all"}
              count={allItems.length}
              onClick={() => {
                setSection("all");
                setTypeFilter(null);
              }}
            />
            <SectionBtn
              label="My context"
              active={section === "mine"}
              count={allItems.filter((i) => i.visibility === "private").length}
              onClick={() => {
                setSection("mine");
                setTypeFilter(null);
                setTeamFilter(null);
              }}
            />
            <SectionBtn
              label={tenant.name}
              active={section === "org"}
              count={allItems.filter((i) => i.visibility === "org").length}
              onClick={() => {
                setSection("org");
                setTypeFilter(null);
                setTeamFilter(null);
              }}
            />
          </div>
        </aside>
      )}

      {/* Doc list */}
      <section className="w-80 border-r border-neutral-200 bg-white overflow-y-auto">
        <div className="p-2 border-b border-neutral-200 space-y-2">
          <div className="flex items-center justify-between">
            <div className="text-sm text-neutral-600">
              {entries.state === "loading"
                ? "Loading…"
                : `${visible.length} document${visible.length === 1 ? "" : "s"}`}
            </div>
            <button
              onClick={() => {
                setSelectedId(null);
                setCreating(true);
              }}
              className="text-sm text-brand-600 hover:text-brand-700"
            >
              + New
            </button>
          </div>
          <select
            value={typeFilter ?? ""}
            onChange={(e) => setTypeFilter(e.target.value || null)}
            className="w-full px-2 py-1 border border-neutral-300 rounded text-xs bg-white"
          >
            <option value="">All types ({items.length})</option>
            {types.map((t) => {
              const count = items.filter((i) => i.type_ === t).length;
              return (
                <option key={t} value={t}>
                  {t} ({count})
                </option>
              );
            })}
          </select>
          {isOrg && teams.length > 0 && (
            <select
              value={teamFilter ?? ""}
              onChange={(e) => {
                const next = e.target.value || null;
                setTeamFilter(next);
                // Section filters on visibility (private/org); team
                // docs are neither, so combining them with mine/org
                // would always render an empty list.
                if (next) setSection("all");
              }}
              className="w-full px-2 py-1 border border-neutral-300 rounded text-xs bg-white"
            >
              <option value="">All teams</option>
              {teams.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.name}
                </option>
              ))}
            </select>
          )}
        </div>
        {entries.state === "error" && (
          <div className="p-4 text-sm text-red-600">{entries.message}</div>
        )}
        {entries.state === "ready" && visible.length === 0 && (
          <div className="p-6 text-sm text-neutral-500 text-center">
            No documents yet. Click{" "}
            <span className="text-brand-600">+ New</span> to create one.
          </div>
        )}
        {visible.map((e) => (
          <button
            key={e.doc_id}
            onClick={() => {
              setSelectedId(e.doc_id);
              setCreating(false);
            }}
            className={
              "block w-full text-left px-3 py-2 border-b border-neutral-100 " +
              (selectedId === e.doc_id ? "bg-brand-50" : "hover:bg-neutral-50")
            }
          >
            <div className="flex items-center gap-2 mb-0.5">
              <span className="text-sm font-medium text-neutral-900 truncate">
                {e.title || e.doc_id}
              </span>
            </div>
            <div className="flex items-center gap-2 text-xs text-neutral-500">
              <span className="font-mono truncate">{e.doc_id}</span>
              <VisibilityChip v={e.visibility} />
            </div>
          </button>
        ))}
      </section>

      {/* Detail */}
      <section className="flex-1 min-w-0 overflow-y-auto">
        {creating && (
          <DocEditor
            key={`__new__:${typeFilter ?? ""}:${defaultVisibilityForNew}`}
            tenantId={tenant.tenant_id}
            tenantName={tenant.name}
            tenantKind={tenant.kind}
            tenantRole={tenant.role}
            teams={teams}
            initial={null}
            defaultType={typeFilter ?? undefined}
            defaultVisibility={defaultVisibilityForNew}
            onSaved={async (d) => {
              await refreshList();
              setCreating(false);
              setSelectedId(d.id);
            }}
            onCancel={() => setCreating(false)}
          />
        )}
        {!creating && detail?.state === "loading" && (
          <div className="p-6 text-sm text-neutral-500">Loading…</div>
        )}
        {!creating && detail?.state === "error" && (
          <div className="m-4 p-3 bg-red-50 text-red-700 text-sm rounded-lg border border-red-200">
            {detail.message}
          </div>
        )}
        {!creating && detail?.state === "ready" && (
          <>
            {pendingByDoc[detail.data.id] > 0 && onSwitchToProposals && (
              <ProposalBanner
                count={pendingByDoc[detail.data.id]}
                onReview={() => onSwitchToProposals(detail.data.id)}
              />
            )}
            <DocEditor
              key={`${detail.data.id}@${detail.data.version}`}
              tenantId={tenant.tenant_id}
              tenantName={tenant.name}
              tenantKind={tenant.kind}
              tenantRole={tenant.role}
              teams={teams}
              initial={detail.data}
              onSaved={async (d) => {
                await refreshList();
                setDetail({ state: "ready", data: d });
              }}
              onDeleted={async () => {
                await refreshList();
                setSelectedId(null);
                setDetail(null);
              }}
            />
          </>
        )}
        {!creating && !detail && (
          <div className="h-full flex items-center justify-center text-neutral-400 text-sm">
            Select a document or create a new one.
          </div>
        )}
      </section>
    </div>
  );
}

function ProposalBanner({
  count,
  onReview,
}: {
  count: number;
  onReview: () => void;
}) {
  return (
    <div className="mx-6 mt-6 mb-0 px-4 py-3 bg-amber-50 border border-amber-200 rounded-md flex items-center justify-between gap-3">
      <div className="text-sm text-amber-900">
        <strong>
          {count} pending proposal{count === 1 ? "" : "s"}
        </strong>{" "}
        against this document.
      </div>
      <button
        onClick={onReview}
        className="text-xs px-3 py-1.5 rounded bg-amber-600 text-white hover:bg-amber-700"
      >
        Review →
      </button>
    </div>
  );
}

function SectionBtn({
  label,
  active,
  count,
  onClick,
}: {
  label: string;
  active: boolean;
  count: number;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={
        "w-full flex items-center justify-between text-left text-sm px-3 py-1.5 rounded " +
        (active
          ? "bg-brand-50 text-brand-700 font-medium"
          : "text-neutral-700 hover:bg-neutral-100")
      }
    >
      <span className="truncate">{label}</span>
      <span className="text-xs text-neutral-400 ml-2">{count}</span>
    </button>
  );
}

function VisibilityChip({ v }: { v: string }) {
  const color =
    v === "private"
      ? "bg-red-100 text-red-700"
      : v === "personal"
      ? "bg-amber-100 text-amber-700"
      : v === "work"
      ? "bg-blue-100 text-blue-700"
      : v === "public"
      ? "bg-green-100 text-green-700"
      : v === "org"
      ? "bg-violet-100 text-violet-700"
      : v === "team"
      ? "bg-indigo-100 text-indigo-700"
      : "bg-neutral-100 text-neutral-700";
  return (
    <span className={`inline-block px-1.5 py-0.5 rounded text-[10px] ${color}`}>
      {v}
    </span>
  );
}

function DocEditor({
  tenantId,
  tenantName,
  tenantKind,
  tenantRole,
  teams,
  initial,
  defaultType,
  defaultVisibility,
  onSaved,
  onDeleted,
  onCancel,
}: {
  tenantId: string;
  tenantName: string;
  tenantKind: string;
  tenantRole: string;
  /// Teams the viewer can see in this org (empty for personal vaults).
  /// Drives the visibility=team option and the team-picker dropdown.
  teams: TeamSummary[];
  initial: DocDetail | null;
  /// When creating a new doc, pre-fill the type field with this
  /// (typically the active type-filter in the parent list view, so a
  /// user clicking "+ New" while filtered to "relationships" lands
  /// in a new-doc form already typed as "relationships"). Ignored
  /// when editing an existing doc.
  defaultType?: string;
  /// When creating a new doc, pre-fill the visibility field. Comes
  /// from the parent's active section (org section → "org", everywhere
  /// else → "private") so users don't have to flip the dropdown after
  /// every "+ New". Ignored when editing an existing doc.
  defaultVisibility?: string;
  onSaved: (d: DocDetail) => Promise<void> | void;
  onDeleted?: () => Promise<void> | void;
  onCancel?: () => void;
}) {
  const isOrg = tenantKind === "org";
  const isOrgAdmin = isOrg && (tenantRole === "owner" || tenantRole === "admin");
  // Teams the viewer can WRITE to: org admins/owners can write to any
  // team; others must hold a manager role. Only when this set is
  // non-empty does the visibility=team option appear in the dropdown
  // for new docs.
  const writableTeams = useMemo(() => {
    if (!isOrg) return [];
    if (isOrgAdmin) return teams;
    return teams.filter((t) => t.viewer_role === "manager");
  }, [isOrg, isOrgAdmin, teams]);
  // Visibility set per context (Phase 3 platform 4-layer model). The
  // create form only offers what makes sense for the current context;
  // the editor for an existing doc keeps the doc's current visibility
  // available even if it's outside the new set (legacy doc, custom
  // label, etc.) so the value isn't silently dropped.
  //
  // `team` is gated on `writableTeams.length > 0` — a member who's not
  // a manager of any team can read team docs but not author them.
  const allowedVisibilities: readonly string[] = isOrg
    ? writableTeams.length > 0
      ? ORG_VISIBILITIES
      : ORG_VISIBILITIES.filter((v) => v !== "team")
    : PERSONAL_VISIBILITIES;
  const isNew = initial === null;
  // Split the stored markdown body into a `title` (the leading `# H1`)
  // and the rest. Lets the editor expose a plain Title field +
  // free-text Content area instead of asking users to write `# Title`
  // syntax themselves. On save we recombine — round-trips byte-for-
  // byte for docs that already had a leading H1, and gains one for
  // docs that didn't.
  const split = useMemo(
    () => splitTitleAndBody(initial?.body ?? ""),
    [initial?.body]
  );
  const [id, setId] = useState(initial?.id ?? "");
  // For new docs without an active type filter, we don't pre-pick a
  // type — the select shows a "Please select…" placeholder until the
  // user chooses. Saves are gated on a non-empty type.
  const [type, setType] = useState(
    initial?.type ?? defaultType ?? ""
  );
  const [visibility, setVisibility] = useState(
    initial?.visibility ?? defaultVisibility ?? "private"
  );
  // Team binding for visibility=team docs. Defaults to the doc's
  // existing team (when editing) or the viewer's first writable team
  // (when creating). When the user flips visibility off/on team, we
  // re-default rather than holding a stale value.
  const [teamId, setTeamId] = useState<string | null>(
    initial?.team_id ?? (writableTeams[0]?.id ?? null)
  );
  const [tags, setTags] = useState((initial?.tags ?? []).join(", "));
  const [title, setTitle] = useState(isNew ? "" : split.title);
  const [body, setBody] = useState(isNew ? "" : split.body);
  const [busy, setBusy] = useState(false);
  // Track whether the user has hand-edited the ID. We auto-derive the
  // id from the title for new docs until that happens — once the user
  // touches the id field, we stop syncing so we don't clobber their
  // intentional value.
  const [idTouched, setIdTouched] = useState(!isNew);
  const [err, setErr] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState<number | null>(null);

  // The visibility dropdown unions the allowed-for-context set with
  // the current value (so legacy values render rather than vanish).
  const visibilityOptions = useMemo(() => {
    const set = new Set<string>(allowedVisibilities);
    if (visibility) set.add(visibility);
    return Array.from(set);
  }, [allowedVisibilities, visibility]);

  // Type dropdown: the seed types plus the doc's current type if it's
  // a custom value (so editing a doc with a non-seed type doesn't
  // silently change it on save).
  const typeOptions = useMemo(() => {
    const set = new Set<string>(SEED_TYPES);
    if (type) set.add(type);
    return Array.from(set).sort();
  }, [type]);

  // Auto-derive the doc id from the title for new docs until the
  // user manually edits the id field. Slugifies + clamps to the
  // 64-char limit `orchext_vault::DocumentId` enforces.
  useEffect(() => {
    if (!isNew || idTouched) return;
    setId(slugify(title));
  }, [title, isNew, idTouched]);

  // Keep team_id consistent with visibility — clear it when the user
  // moves away from `team`, default it on the way in.
  useEffect(() => {
    if (visibility === "team") {
      if (teamId === null && writableTeams.length > 0) {
        setTeamId(writableTeams[0].id);
      }
    } else if (teamId !== null) {
      setTeamId(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visibility]);

  useEffect(() => {
    if (savedAt === null) return;
    const t = setTimeout(() => setSavedAt(null), 1800);
    return () => clearTimeout(t);
  }, [savedAt]);

  async function save() {
    setErr(null);
    setBusy(true);
    try {
      const trimmedId = id.trim();
      const trimmedType = type.trim();
      const tagList = tags
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean);

      const combinedBody = combineTitleAndBody(title, body);
      const canonical = buildSource({
        id: trimmedId,
        type: trimmedType,
        visibility,
        tags: tagList,
        links: initial?.links ?? [],
        aliases: initial?.aliases ?? [],
        // Preserve any existing provenance value on edit (the field
        // is no longer surfaced in the form, but we don't want to
        // silently strip it from docs that already have one).
        source: initial?.source ?? null,
        body: combinedBody,
      });

      if (visibility === "team" && !teamId) {
        throw new Error("Pick a team for visibility=team docs.");
      }
      const resp = await api.docWrite(
        tenantId,
        trimmedId,
        canonical,
        isNew ? null : initial!.version,
        visibility === "team" ? teamId : null
      );

      const saved: DocDetail = {
        id: resp.doc_id,
        type: resp.type_,
        visibility: resp.visibility,
        tags: tagList,
        links: initial?.links ?? [],
        aliases: initial?.aliases ?? [],
        source: initial?.source ?? null,
        created: initial?.created ?? null,
        updated: initial?.updated ?? null,
        body: combinedBody,
        version: resp.version,
        updated_at: resp.updated_at,
        team_id: resp.team_id ?? null,
      };
      setSavedAt(Date.now());
      await onSaved(saved);
    } catch (e) {
      setErr(errMessage(e));
    } finally {
      setBusy(false);
    }
  }

  async function del() {
    if (!initial || !onDeleted) return;
    if (!confirm(`Delete ${initial.id}? This cannot be undone.`)) return;
    setErr(null);
    setBusy(true);
    try {
      await api.docDelete(tenantId, initial.id, initial.version);
      await onDeleted();
    } catch (e) {
      setErr(errMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="p-6 max-w-3xl mx-auto">
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-lg font-semibold">
          {isNew ? "New document" : initial?.id}
        </h2>
        <div className="flex gap-2">
          {onCancel && (
            <button
              onClick={onCancel}
              className="px-3 py-1.5 text-sm text-neutral-600 hover:bg-neutral-100 rounded"
            >
              Cancel
            </button>
          )}
          {!isNew && onDeleted && (
            <button
              onClick={del}
              disabled={busy}
              className="px-3 py-1.5 text-sm text-red-600 hover:bg-red-50 rounded disabled:opacity-50"
            >
              Delete
            </button>
          )}
          <button
            onClick={save}
            disabled={busy || !id.trim() || !type.trim()}
            className="px-3 py-1.5 text-sm bg-brand-600 text-white rounded hover:bg-brand-700 disabled:opacity-50"
          >
            {busy ? "Saving…" : "Save"}
          </button>
          {savedAt !== null && (
            <span
              role="status"
              aria-live="polite"
              className="inline-flex items-center gap-1 px-2 py-1 text-xs text-green-700 bg-green-50 border border-green-200 rounded"
            >
              <span aria-hidden="true">✓</span>
              <span>Saved</span>
            </span>
          )}
        </div>
      </div>

      <div className="mb-4">
        <Field label="Title">
          <input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="A short, human-readable title"
            className="w-full px-3 py-1.5 border border-neutral-300 rounded text-base"
          />
        </Field>
      </div>

      <div className="grid grid-cols-2 gap-3 mb-4">
        <Field label="ID">
          <input
            value={id}
            onChange={(e) => {
              setIdTouched(true);
              setId(e.target.value);
            }}
            disabled={!isNew}
            placeholder="auto-derived from title"
            className="w-full px-3 py-1.5 border border-neutral-300 rounded text-sm font-mono disabled:bg-neutral-100"
          />
        </Field>
        <Field label="Type">
          <select
            value={type}
            onChange={(e) => setType(e.target.value)}
            className="w-full px-3 py-1.5 border border-neutral-300 rounded text-sm bg-white"
          >
            {!type && (
              <option value="" disabled>
                Please select…
              </option>
            )}
            {typeOptions.map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Visibility">
          <select
            value={visibility}
            onChange={(e) => setVisibility(e.target.value)}
            className="w-full px-3 py-1.5 border border-neutral-300 rounded text-sm"
          >
            {visibilityOptions.map((v) => (
              <option key={v} value={v}>
                {v}
              </option>
            ))}
          </select>
          <p className="text-xs text-neutral-500 mt-1">
            {audienceCopy(visibility, isOrg, tenantName, teamId, teams)}
          </p>
        </Field>
        {visibility === "team" && (
          <Field label="Team">
            <select
              value={teamId ?? ""}
              onChange={(e) => setTeamId(e.target.value || null)}
              className="w-full px-3 py-1.5 border border-neutral-300 rounded text-sm"
            >
              {!teamId && (
                <option value="" disabled>
                  Select a team…
                </option>
              )}
              {/* For new docs, only writable teams. For existing team
                  docs that the viewer can read but not write to, keep
                  the current team_id in the list so it doesn't vanish. */}
              {(isNew
                ? writableTeams
                : teams.filter(
                    (t) =>
                      isOrgAdmin ||
                      t.viewer_role === "manager" ||
                      t.id === teamId
                  )
              ).map((t) => (
                <option key={t.id} value={t.id}>
                  {t.name}
                </option>
              ))}
            </select>
          </Field>
        )}
        <Field label="Tags (comma-separated)">
          <input
            value={tags}
            onChange={(e) => setTags(e.target.value)}
            placeholder="manager, acme"
            className="w-full px-3 py-1.5 border border-neutral-300 rounded text-sm"
          />
        </Field>
      </div>

      <Field label="Content">
        <RichTextEditor
          value={body}
          onChange={setBody}
          placeholder="Just write — apply formatting from the toolbar above. Switch to Advanced to edit raw markdown."
        />
      </Field>

      {err && (
        <div className="mt-4 p-3 bg-red-50 text-red-700 text-sm rounded-lg border border-red-200">
          {err}
        </div>
      )}
    </div>
  );
}

/// Build a vault doc id from a free-text title. Lowercase ASCII +
/// digits + dashes (matches `orchext_vault::DocumentId::is_valid`).
/// Trims to 64 chars and drops any leading dashes that fall out
/// after non-ASCII characters get squashed.
function slugify(title: string): string {
  const lowered = title.toLowerCase();
  let out = "";
  for (const ch of lowered) {
    if ((ch >= "a" && ch <= "z") || (ch >= "0" && ch <= "9")) {
      out += ch;
    } else if (out.length > 0 && !out.endsWith("-")) {
      out += "-";
    }
  }
  out = out.replace(/-+$/, "");
  if (out.length > 64) out = out.slice(0, 64).replace(/-+$/, "");
  return out;
}

/// Split a stored markdown body into a leading H1 (the doc's title)
/// and the rest of the content. If the body doesn't start with `# X`,
/// returns an empty title and the whole string as the body.
///
/// Pairs with `combineTitleAndBody` — round-trips cleanly for docs
/// whose body already had a leading H1.
function splitTitleAndBody(source: string): { title: string; body: string } {
  if (!source) return { title: "", body: "" };
  const lines = source.split("\n");
  const first = lines[0] ?? "";
  const m = first.match(/^# (.+)$/);
  if (!m) return { title: "", body: source };
  // Drop the H1 and a single immediately-following blank line if
  // present, so the body the user sees starts at the actual content.
  let bodyStart = 1;
  if (lines[bodyStart] === "") bodyStart += 1;
  return { title: m[1].trim(), body: lines.slice(bodyStart).join("\n") };
}

/// Reassemble a markdown body from a Title field + free-text body.
/// If `title` is empty, the body is stored as-is (no H1 added).
function combineTitleAndBody(title: string, body: string): string {
  const t = title.trim();
  const b = body.replace(/^\n+/, "").replace(/\s+$/, "");
  if (!t) return b;
  if (!b) return `# ${t}\n`;
  return `# ${t}\n\n${b}\n`;
}

/// Inline copy under the visibility selector. Tells the user who will
/// see the doc — the most-asked question of the create form.
function audienceCopy(
  visibility: string,
  isOrg: boolean,
  tenantName: string,
  teamId: string | null,
  teams: TeamSummary[]
): string {
  switch (visibility) {
    case "private":
      return isOrg
        ? `Only you, scoped to ${tenantName}.`
        : "Only you. Stays in your personal vault.";
    case "org":
      return `All members of ${tenantName} can read this.`;
    case "team": {
      const t = teams.find((t) => t.id === teamId);
      return t
        ? `Members of the ${t.name} team (and ${tenantName} admins) can read this.`
        : "Pick a team — only its members can read.";
    }
    case "personal":
      return "Only you. Tagged as personal-life context.";
    case "work":
      return "Only you. Tagged as work context.";
    case "public":
      return "Anyone with vault access can read this.";
    default:
      return "Custom visibility — scope is whatever your token grants.";
  }
}

function Field({
  label,
  full,
  children,
}: {
  label: string;
  full?: boolean;
  children: React.ReactNode;
}) {
  return (
    <label className={full ? "col-span-2" : ""}>
      <span className="block text-xs text-neutral-600 mb-1">{label}</span>
      {children}
    </label>
  );
}
