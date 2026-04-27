import { useEffect, useMemo, useState } from "react";
import {
  api,
  ApiFailure,
  ListEntry,
  Membership,
  ORG_VISIBILITIES,
  PERSONAL_VISIBILITIES,
  SEED_TYPES,
} from "./api";
import { buildSource, DocDetail, parseSource } from "./docSource";

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

export function DocumentsView({ tenant }: { tenant: Membership }) {
  const isOrg = tenant.kind === "org";
  const [entries, setEntries] = useState<Load<ListEntry[]>>({ state: "loading" });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [typeFilter, setTypeFilter] = useState<string | null>(null);
  const [section, setSection] = useState<Section>("all");
  const [detail, setDetail] = useState<Load<DocDetail> | null>(null);
  const [creating, setCreating] = useState(false);

  async function refreshList() {
    try {
      const list = await api.docList(tenant.tenant_id);
      setEntries({ state: "ready", data: list.entries });
    } catch (e) {
      setEntries({ state: "error", message: errMessage(e) });
    }
  }

  useEffect(() => {
    setEntries({ state: "loading" });
    setSelectedId(null);
    setDetail(null);
    setCreating(false);
    void refreshList();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tenant.tenant_id]);

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
              }}
            />
            <SectionBtn
              label={tenant.name}
              active={section === "org"}
              count={allItems.filter((i) => i.visibility === "org").length}
              onClick={() => {
                setSection("org");
                setTypeFilter(null);
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
          <DocEditor
            key={`${detail.data.id}@${detail.data.version}`}
            tenantId={tenant.tenant_id}
            tenantName={tenant.name}
            tenantKind={tenant.kind}
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
  // Visibility set per context (Phase 3 platform 4-layer model). The
  // create form only offers what makes sense for the current context;
  // the editor for an existing doc keeps the doc's current visibility
  // available even if it's outside the new set (legacy doc, custom
  // label, etc.) so the value isn't silently dropped.
  const allowedVisibilities: readonly string[] = isOrg
    ? ORG_VISIBILITIES
    : PERSONAL_VISIBILITIES;
  const isNew = initial === null;
  const [id, setId] = useState(initial?.id ?? "");
  const [type, setType] = useState(
    initial?.type ?? defaultType ?? "relationships"
  );
  const [visibility, setVisibility] = useState(
    initial?.visibility ?? defaultVisibility ?? "private"
  );
  const [tags, setTags] = useState((initial?.tags ?? []).join(", "));
  const [sourceField, setSourceField] = useState(initial?.source ?? "");
  const [body, setBody] = useState(initial?.body ?? "# New document\n\n");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState<number | null>(null);

  // The visibility dropdown unions the allowed-for-context set with
  // the current value (so legacy values render rather than vanish).
  const visibilityOptions = useMemo(() => {
    const set = new Set<string>(allowedVisibilities);
    if (visibility) set.add(visibility);
    return Array.from(set);
  }, [allowedVisibilities, visibility]);

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
      const provenance = sourceField.trim() || null;
      const tagList = tags
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean);

      const canonical = buildSource({
        id: trimmedId,
        type: trimmedType,
        visibility,
        tags: tagList,
        links: initial?.links ?? [],
        aliases: initial?.aliases ?? [],
        source: provenance,
        body,
      });

      const resp = await api.docWrite(
        tenantId,
        trimmedId,
        canonical,
        isNew ? null : initial!.version
      );

      const saved: DocDetail = {
        id: resp.doc_id,
        type: resp.type_,
        visibility: resp.visibility,
        tags: tagList,
        links: initial?.links ?? [],
        aliases: initial?.aliases ?? [],
        source: provenance,
        created: initial?.created ?? null,
        updated: initial?.updated ?? null,
        body,
        version: resp.version,
        updated_at: resp.updated_at,
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

      <div className="grid grid-cols-2 gap-3 mb-4">
        <Field label="ID">
          <input
            value={id}
            onChange={(e) => setId(e.target.value)}
            disabled={!isNew}
            placeholder="e.g. rel-jane-smith"
            className="w-full px-3 py-1.5 border border-neutral-300 rounded text-sm font-mono disabled:bg-neutral-100"
          />
        </Field>
        <Field label="Type">
          <input
            value={type}
            onChange={(e) => setType(e.target.value)}
            list="type-options"
            className="w-full px-3 py-1.5 border border-neutral-300 rounded text-sm"
          />
          <datalist id="type-options">
            {SEED_TYPES.map((t) => (
              <option key={t} value={t} />
            ))}
          </datalist>
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
            {audienceCopy(visibility, isOrg, tenantName)}
          </p>
        </Field>
        <Field label="Source (provenance)">
          <input
            value={sourceField}
            onChange={(e) => setSourceField(e.target.value)}
            placeholder="optional — e.g. onboarding-2026-04"
            className="w-full px-3 py-1.5 border border-neutral-300 rounded text-sm"
          />
        </Field>
        <Field label="Tags (comma-separated)" full>
          <input
            value={tags}
            onChange={(e) => setTags(e.target.value)}
            placeholder="manager, acme"
            className="w-full px-3 py-1.5 border border-neutral-300 rounded text-sm"
          />
        </Field>
      </div>

      <Field label="Body (markdown)">
        <textarea
          value={body}
          onChange={(e) => setBody(e.target.value)}
          rows={20}
          className="w-full px-3 py-2 border border-neutral-300 rounded text-sm font-mono leading-relaxed"
        />
      </Field>

      {!isNew && initial && (
        <div className="mt-4 pt-4 border-t border-neutral-200 text-xs text-neutral-500 font-mono">
          {initial.version}
          {initial.updated_at && ` · updated ${initial.updated_at}`}
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

/// Inline copy under the visibility selector. Tells the user who will
/// see the doc — the most-asked question of the create form.
function audienceCopy(visibility: string, isOrg: boolean, tenantName: string): string {
  switch (visibility) {
    case "private":
      return isOrg
        ? `Only you, scoped to ${tenantName}.`
        : "Only you. Stays in your personal vault.";
    case "org":
      return `All members of ${tenantName} can read this.`;
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
