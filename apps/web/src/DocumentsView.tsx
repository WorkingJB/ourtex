import { useEffect, useMemo, useState } from "react";
import {
  api,
  ApiFailure,
  ListEntry,
  Membership,
  SEED_TYPES,
  VISIBILITIES,
} from "./api";
import { buildSource, DocDetail, parseSource } from "./docSource";

type Load<T> =
  | { state: "loading" }
  | { state: "error"; message: string }
  | { state: "ready"; data: T };

function errMessage(e: unknown): string {
  return e instanceof ApiFailure ? e.message : String(e);
}

export function DocumentsView({ tenant }: { tenant: Membership }) {
  const [entries, setEntries] = useState<Load<ListEntry[]>>({ state: "loading" });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [typeFilter, setTypeFilter] = useState<string | null>(null);
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

  const items = entries.state === "ready" ? entries.data : [];
  const types = useMemo(() => {
    const present = new Set<string>(items.map((i) => i.type_));
    for (const t of SEED_TYPES) present.add(t);
    return Array.from(present).sort();
  }, [items]);

  const visible = typeFilter ? items.filter((i) => i.type_ === typeFilter) : items;

  return (
    <div className="flex h-full min-h-0">
      {/* Types sidebar */}
      <aside className="w-48 border-r border-neutral-200 bg-white overflow-y-auto">
        <div className="p-2">
          <button
            onClick={() => setTypeFilter(null)}
            className={
              "w-full text-left text-sm px-3 py-1.5 rounded " +
              (typeFilter === null
                ? "bg-brand-50 text-brand-700 font-medium"
                : "text-neutral-700 hover:bg-neutral-100")
            }
          >
            All ({items.length})
          </button>
        </div>
        <div className="px-2 pb-2 text-xs uppercase tracking-wider text-neutral-500 mt-2">
          Types
        </div>
        {types.map((t) => {
          const count = items.filter((i) => i.type_ === t).length;
          return (
            <button
              key={t}
              onClick={() => setTypeFilter(t)}
              className={
                "w-full text-left text-sm px-3 py-1.5 " +
                (typeFilter === t
                  ? "bg-brand-50 text-brand-700 font-medium"
                  : "text-neutral-700 hover:bg-neutral-100")
              }
            >
              {t}{" "}
              <span className="text-neutral-400 text-xs">({count})</span>
            </button>
          );
        })}
      </aside>

      {/* Doc list */}
      <section className="w-80 border-r border-neutral-200 bg-white overflow-y-auto">
        <div className="p-2 border-b border-neutral-200 flex items-center justify-between">
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
            key="__new__"
            tenantId={tenant.tenant_id}
            initial={null}
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
      : "bg-neutral-100 text-neutral-700";
  return (
    <span className={`inline-block px-1.5 py-0.5 rounded text-[10px] ${color}`}>
      {v}
    </span>
  );
}

function DocEditor({
  tenantId,
  initial,
  onSaved,
  onDeleted,
  onCancel,
}: {
  tenantId: string;
  initial: DocDetail | null;
  onSaved: (d: DocDetail) => Promise<void> | void;
  onDeleted?: () => Promise<void> | void;
  onCancel?: () => void;
}) {
  const isNew = initial === null;
  const [id, setId] = useState(initial?.id ?? "");
  const [type, setType] = useState(initial?.type ?? "relationships");
  const [visibility, setVisibility] = useState(initial?.visibility ?? "work");
  const [tags, setTags] = useState((initial?.tags ?? []).join(", "));
  const [sourceField, setSourceField] = useState(initial?.source ?? "");
  const [body, setBody] = useState(initial?.body ?? "# New document\n\n");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState<number | null>(null);

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
            {VISIBILITIES.map((v) => (
              <option key={v} value={v}>
                {v}
              </option>
            ))}
          </select>
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
