import { useEffect, useMemo, useState } from "react";
import {
  api,
  DocDetail,
  DocListItem,
  ORG_VISIBILITIES,
  PERSONAL_VISIBILITIES,
  Proposal,
  SEED_TYPES,
  TeamSummary,
} from "./api";
import { Context } from "./OrgRail";
import { RichTextEditor } from "./RichTextEditor";

/// Section toggle in the org-context Documents pane (mirrors web):
///   "mine" → visibility=private docs (My notes for [Org])
///   "org"  → visibility=org docs (the org's shared context)
///   "all"  → both, default
type Section = "all" | "mine" | "org";

export function DocumentsView({
  ctx,
  onMutated,
  onSwitchToProposals,
}: {
  ctx: Context;
  onMutated?: () => void | Promise<void>;
  onSwitchToProposals?: (docId: string) => void;
}) {
  const isOrg = ctx.kind === "org";
  const [items, setItems] = useState<DocListItem[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [typeFilter, setTypeFilter] = useState<string | null>(null);
  const [section, setSection] = useState<Section>("all");
  /// Team filter applied server-side via `?team_id=…` (org workspaces
  /// only — the local vault has no team concept). Forces section to
  /// "all" when set: section filters on visibility, team docs are
  /// neither private nor org, so combining the filters always renders
  /// an empty list.
  const [teamFilter, setTeamFilter] = useState<string | null>(null);
  const [detail, setDetail] = useState<DocDetail | null>(null);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Teams visible to the viewer in this org (empty otherwise). Drives
  // the visibility=team option and team picker in the doc editor.
  const [teams, setTeams] = useState<TeamSummary[]>([]);
  /// Pending-proposal count keyed by doc_id for the inline banner.
  /// Refreshed alongside the doc list so approvals from a Proposals
  /// session reflect immediately on return.
  const [pendingByDoc, setPendingByDoc] = useState<Record<string, number>>({});

  async function refreshProposalCounts() {
    try {
      const list: Proposal[] = await api.proposalList("pending");
      const counts: Record<string, number> = {};
      for (const p of list) {
        counts[p.doc_id] = (counts[p.doc_id] ?? 0) + 1;
      }
      setPendingByDoc(counts);
    } catch {
      // Best-effort; don't fail the docs view on a proposals fetch error.
    }
  }

  async function refreshList() {
    try {
      const list = await api.docList({ teamId: teamFilter });
      setItems(list);
      await onMutated?.();
    } catch (e) {
      setError(String(e));
    }
    void refreshProposalCounts();
  }

  useEffect(() => {
    setItems([]);
    setSelectedId(null);
    setDetail(null);
    setCreating(false);
    setSection("all");
    setTypeFilter(null);
    setTeamFilter(null);
    void refreshList();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ctx.workspaceId]);

  // teamFilter changes alone re-fetch but don't reset selection state.
  useEffect(() => {
    void refreshList();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [teamFilter]);

  useEffect(() => {
    if (ctx.kind !== "org") {
      setTeams([]);
      return;
    }
    let cancelled = false;
    api
      .teamsList(ctx.workspaceId, ctx.orgId)
      .then((r) => {
        if (!cancelled) setTeams(r.teams);
      })
      .catch(() => {
        if (!cancelled) setTeams([]);
      });
    return () => {
      cancelled = true;
    };
  }, [ctx]);

  // Refresh the list whenever the watcher (local vault) sees a change
  // under the vault root. Remote workspaces still get explicit refresh
  // on mutations + a fresh fetch on context switch.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    api
      .onVaultChanged(() => {
        void refreshList();
      })
      .then((fn) => {
        unlisten = fn;
      });
    return () => {
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!selectedId) {
      setDetail(null);
      return;
    }
    api
      .docRead(selectedId)
      .then((d) => setDetail(d))
      .catch((e) => setError(String(e)));
  }, [selectedId]);

  // Section pre-filter applies before type filter so the type counts
  // reflect only docs in the active section.
  const sectionItems = useMemo(() => {
    if (!isOrg || section === "all") return items;
    if (section === "mine") return items.filter((i) => i.visibility === "private");
    if (section === "org") return items.filter((i) => i.visibility === "org");
    return items;
  }, [items, isOrg, section]);

  const types = useMemo(() => {
    const present = new Set<string>(sectionItems.map((i) => i.type));
    for (const t of SEED_TYPES) present.add(t);
    return Array.from(present).sort();
  }, [sectionItems]);

  const visible = typeFilter
    ? sectionItems.filter((i) => i.type === typeFilter)
    : sectionItems;

  // Default visibility for a "+ New" doc, computed from the active
  // section so the user isn't fighting the form. In the org-context
  // section, assume the user is creating shared org content; elsewhere,
  // default to private.
  const defaultVisibilityForNew: string =
    isOrg && section === "org" ? "org" : "private";

  const ctxName = ctx.kind === "org" ? ctx.name : ctxLabel(ctx);

  return (
    <div className="flex h-full min-h-0">
      {/* Section sidebar — only in org workspace. */}
      {isOrg && (
        <aside className="w-44 border-r border-neutral-200 bg-white overflow-y-auto">
          <div className="p-2">
            <div className="text-xs uppercase tracking-wider text-neutral-500 mb-1 px-1">
              Section
            </div>
            <SectionBtn
              label="All"
              active={section === "all"}
              count={items.length}
              onClick={() => {
                setSection("all");
                setTypeFilter(null);
              }}
            />
            <SectionBtn
              label="My context"
              active={section === "mine"}
              count={items.filter((i) => i.visibility === "private").length}
              onClick={() => {
                setSection("mine");
                setTypeFilter(null);
                setTeamFilter(null);
              }}
            />
            <SectionBtn
              label={ctx.name}
              active={section === "org"}
              count={items.filter((i) => i.visibility === "org").length}
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
              {visible.length} document{visible.length === 1 ? "" : "s"}
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
            <option value="">All types ({sectionItems.length})</option>
            {types.map((t) => {
              const count = sectionItems.filter((i) => i.type === t).length;
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
        {visible.length === 0 && (
          <div className="p-6 text-sm text-neutral-500 text-center">
            No documents yet. Click <span className="text-brand-600">+ New</span>{" "}
            to create one.
          </div>
        )}
        {visible.map((item) => (
          <button
            key={item.id}
            onClick={() => {
              setSelectedId(item.id);
              setCreating(false);
            }}
            className={
              "block w-full text-left px-3 py-2 border-b border-neutral-100 " +
              (selectedId === item.id ? "bg-brand-50" : "hover:bg-neutral-50")
            }
          >
            <div className="flex items-center gap-2 mb-0.5">
              <span className="text-sm font-medium text-neutral-900 truncate">
                {item.title}
              </span>
            </div>
            <div className="flex items-center gap-2 text-xs text-neutral-500">
              <span className="font-mono">{item.id}</span>
              <VisibilityChip v={item.visibility} />
            </div>
          </button>
        ))}
      </section>

      {/* Detail */}
      <section className="flex-1 min-w-0 overflow-y-auto">
        {error && (
          <div className="m-4 p-3 bg-red-50 text-red-700 text-sm rounded-lg border border-red-200">
            {error}
          </div>
        )}
        {creating && (
          <DocEditor
            key={`__new__:${typeFilter ?? ""}:${defaultVisibilityForNew}`}
            ctxKind={ctx.kind}
            ctxName={ctxName}
            ctxRole={ctx.kind === "org" ? ctx.role : "owner"}
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
        {!creating && detail && (
          <>
            {pendingByDoc[detail.id] > 0 && onSwitchToProposals && (
              <ProposalBanner
                count={pendingByDoc[detail.id]}
                onReview={() => onSwitchToProposals(detail.id)}
              />
            )}
            <DocEditor
              // Keyed by id+version so switching docs remounts the form
              // (useState only reads initial props on mount), and saving
              // also remounts so the editor shows the post-save truth.
              key={`${detail.id}@${detail.version}`}
              ctxKind={ctx.kind}
              ctxName={ctxName}
              ctxRole={ctx.kind === "org" ? ctx.role : "owner"}
              teams={teams}
              initial={detail}
              onSaved={async (d) => {
                await refreshList();
                setDetail(d);
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

function ctxLabel(ctx: Context): string {
  if (ctx.kind === "personal") return "Personal";
  if (ctx.kind === "local") return ctx.name;
  return ctx.name;
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
  ctxKind,
  ctxName,
  ctxRole,
  teams,
  initial,
  defaultType,
  defaultVisibility,
  onSaved,
  onDeleted,
  onCancel,
}: {
  ctxKind: Context["kind"];
  ctxName: string;
  ctxRole: string;
  teams: TeamSummary[];
  initial: DocDetail | null;
  /// When creating a new doc, pre-fill the type field with this
  /// (typically the active type filter — so a user clicking "+ New"
  /// while filtered to "relationships" lands typed as relationships).
  defaultType?: string;
  /// When creating a new doc, pre-fill the visibility field. Comes
  /// from the parent's active section. Ignored when editing.
  defaultVisibility?: string;
  onSaved: (d: DocDetail) => Promise<void> | void;
  onDeleted?: () => Promise<void> | void;
  onCancel?: () => void;
}) {
  const isOrg = ctxKind === "org";
  const isOrgAdmin = isOrg && (ctxRole === "owner" || ctxRole === "admin");
  /// Teams the viewer can WRITE to. Org admins can write any; other
  /// roles must be team managers. Drives whether `team` shows up in
  /// the visibility dropdown for new docs.
  const writableTeams = useMemo(() => {
    if (!isOrg) return [];
    if (isOrgAdmin) return teams;
    return teams.filter((t) => t.viewer_role === "manager");
  }, [isOrg, isOrgAdmin, teams]);
  /// Visibility set per context (Phase 3 platform 4-layer model).
  /// Local + personal vaults offer the personal set; org workspaces
  /// offer org+private (+team when the viewer can write a team doc).
  const allowedVisibilities: readonly string[] = isOrg
    ? writableTeams.length > 0
      ? ORG_VISIBILITIES
      : ORG_VISIBILITIES.filter((v) => v !== "team")
    : PERSONAL_VISIBILITIES;
  const isNew = initial === null;
  // Split the stored body into a leading H1 (the doc's title) and the
  // rest. Lets the editor expose a plain Title field + free-text
  // Content area instead of asking users to write `# Title` syntax.
  const split = useMemo(
    () => splitTitleAndBody(initial?.body ?? ""),
    [initial?.body]
  );
  const [id, setId] = useState(initial?.id ?? "");
  // For new docs without an active type filter, leave type empty — the
  // select shows "Please select…" until the user chooses. Saves are
  // gated on a non-empty type.
  const [type, setType] = useState(initial?.type ?? defaultType ?? "");
  const [visibility, setVisibility] = useState(
    initial?.visibility ?? defaultVisibility ?? "private"
  );
  const [teamId, setTeamId] = useState<string | null>(
    initial?.team_id ?? (writableTeams[0]?.id ?? null)
  );
  const [tags, setTags] = useState((initial?.tags ?? []).join(", "));
  const [title, setTitle] = useState(isNew ? "" : split.title);
  const [body, setBody] = useState(isNew ? "" : split.body);
  const [busy, setBusy] = useState(false);
  // Track whether the user has hand-edited the ID. We auto-derive the
  // id from the title for new docs until that happens.
  const [idTouched, setIdTouched] = useState(!isNew);
  const [err, setErr] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState<number | null>(null);

  const visibilityOptions = useMemo(() => {
    const set = new Set<string>(allowedVisibilities);
    if (visibility) set.add(visibility);
    return Array.from(set);
  }, [allowedVisibilities, visibility]);

  // Type dropdown: seed types plus the doc's current type if it's a
  // custom value (so editing a non-seed-typed doc doesn't silently
  // change it on save).
  const typeOptions = useMemo(() => {
    const set = new Set<string>(SEED_TYPES);
    if (type) set.add(type);
    return Array.from(set).sort();
  }, [type]);

  // Auto-derive doc id from title for new docs until the user touches
  // the id field. Slugify + clamp to 64 chars (DocumentId::is_valid).
  useEffect(() => {
    if (!isNew || idTouched) return;
    setId(slugify(title));
  }, [title, isNew, idTouched]);

  // Keep team_id in sync with visibility — clear when leaving `team`,
  // default to the first writable team when entering it.
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
      if (visibility === "team" && !teamId) {
        throw new Error("Pick a team for visibility=team docs.");
      }
      const saved = await api.docWrite({
        id: trimmedId,
        type: trimmedType,
        visibility,
        tags: tagList,
        // Preserve any existing provenance value on edit (the field
        // is no longer surfaced; don't silently strip it).
        source: initial?.source ?? null,
        body: combinedBody,
        team_id: visibility === "team" ? teamId : null,
      });
      setSavedAt(Date.now());
      await onSaved(saved);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function del() {
    if (!initial || !onDeleted) return;
    if (!confirm(`Delete ${initial.id}? This cannot be undone.`)) return;
    setBusy(true);
    try {
      await api.docDelete(initial.id);
      await onDeleted();
    } catch (e) {
      setErr(String(e));
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
            {audienceCopy(visibility, isOrg, ctxName, teamId, teams)}
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

      {!isNew && initial && (
        <div className="mt-4 pt-4 border-t border-neutral-200 text-xs text-neutral-500 font-mono">
          {initial.version}
          {initial.updated && ` · updated ${initial.updated}`}
        </div>
      )}

      {err && (
        <div className="mt-4 p-3 bg-red-50 text-red-700 text-sm rounded-lg border border-red-200">
          {err}
        </div>
      )}
    </div>
  );
}

/// Build a vault doc id from a free-text title. Lowercase ASCII +
/// digits + dashes; clamps to 64 chars; matches the regex
/// `orchext_vault::DocumentId::is_valid` enforces.
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
/// and the rest. If the body doesn't start with `# X`, returns an
/// empty title and the whole string as the body.
function splitTitleAndBody(source: string): { title: string; body: string } {
  if (!source) return { title: "", body: "" };
  const lines = source.split("\n");
  const first = lines[0] ?? "";
  const m = first.match(/^# (.+)$/);
  if (!m) return { title: "", body: source };
  let bodyStart = 1;
  if (lines[bodyStart] === "") bodyStart += 1;
  return { title: m[1].trim(), body: lines.slice(bodyStart).join("\n") };
}

/// Reassemble a markdown body from a Title field + free-text body.
/// Empty title → body stored as-is.
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
  ctxName: string,
  teamId: string | null,
  teams: TeamSummary[]
): string {
  switch (visibility) {
    case "private":
      return isOrg
        ? `Only you, scoped to ${ctxName}.`
        : "Only you. Stays in your personal vault.";
    case "org":
      return `All members of ${ctxName} can read this.`;
    case "team": {
      const t = teams.find((t) => t.id === teamId);
      return t
        ? `Members of the ${t.name} team (and ${ctxName} admins) can read this.`
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
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <label className="block">
      <span className="block text-xs text-neutral-600 mb-1">{label}</span>
      {children}
    </label>
  );
}
