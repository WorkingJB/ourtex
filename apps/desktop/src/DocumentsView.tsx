import { useEffect, useMemo, useState } from "react";
import {
  api,
  DocDetail,
  DocListItem,
  SEED_TYPES,
  VISIBILITIES,
} from "./api";

export function DocumentsView({
  onMutated,
}: {
  onMutated?: () => void | Promise<void>;
}) {
  const [items, setItems] = useState<DocListItem[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [typeFilter, setTypeFilter] = useState<string | null>(null);
  const [detail, setDetail] = useState<DocDetail | null>(null);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function refreshList() {
    try {
      const list = await api.docList();
      setItems(list);
      await onMutated?.();
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    void refreshList();
  }, []);

  // Refresh the list whenever the watcher sees a change under the vault
  // root (edits from another editor, `git pull`, agent writes, etc.).
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

  const types = useMemo(() => {
    const present = new Set<string>(items.map((i) => i.type));
    for (const t of SEED_TYPES) {
      present.add(t);
    }
    return Array.from(present).sort();
  }, [items]);

  const visible = typeFilter ? items.filter((i) => i.type === typeFilter) : items;

  return (
    <div className="h-full flex min-w-0">
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
          const count = items.filter((i) => i.type === t).length;
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
        {visible.length === 0 && (
          <div className="p-6 text-sm text-neutral-500 text-center">
            No documents yet. Click <span className="text-brand-600">+ New</span> to create one.
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
              (selectedId === item.id
                ? "bg-brand-50"
                : "hover:bg-neutral-50")
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
            key={`__new__:${typeFilter ?? ""}`}
            initial={null}
            defaultType={typeFilter ?? undefined}
            onSaved={async (d) => {
              await refreshList();
              setCreating(false);
              setSelectedId(d.id);
            }}
            onCancel={() => setCreating(false)}
          />
        )}
        {!creating && detail && (
          <DocEditor
            // Keyed by id+version so switching docs remounts the form
            // (useState only reads initial props on mount), and saving a
            // doc also remounts so the editor shows the post-save truth
            // (updated stamp, canonical body after round-trip).
            key={`${detail.id}@${detail.version}`}
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
  initial,
  defaultType,
  onSaved,
  onDeleted,
  onCancel,
}: {
  initial: DocDetail | null;
  /// When creating a new doc, pre-fill the type field with this
  /// (typically the active type filter in the list view).
  defaultType?: string;
  onSaved: (d: DocDetail) => Promise<void> | void;
  onDeleted?: () => Promise<void> | void;
  onCancel?: () => void;
}) {
  const [id, setId] = useState(initial?.id ?? "");
  const [type, setType] = useState(
    initial?.type ?? defaultType ?? "relationships"
  );
  const [visibility, setVisibility] = useState(initial?.visibility ?? "work");
  const [tags, setTags] = useState((initial?.tags ?? []).join(", "));
  const [source, setSource] = useState(initial?.source ?? "");
  const [body, setBody] = useState(
    initial?.body ?? "# New document\n\n"
  );
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState<number | null>(null);
  const isNew = initial === null;

  useEffect(() => {
    if (savedAt === null) return;
    const t = setTimeout(() => setSavedAt(null), 1800);
    return () => clearTimeout(t);
  }, [savedAt]);

  async function save() {
    setErr(null);
    setBusy(true);
    try {
      const saved = await api.docWrite({
        id: id.trim(),
        type: type.trim(),
        visibility,
        tags: tags
          .split(",")
          .map((t) => t.trim())
          .filter(Boolean),
        source: source.trim() || null,
        body,
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
              className="inline-flex items-center gap-1 px-2 py-1 text-xs text-green-700 bg-green-50 border border-green-200 rounded transition-opacity"
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
            value={source}
            onChange={(e) => setSource(e.target.value)}
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
