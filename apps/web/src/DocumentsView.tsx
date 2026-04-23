import { useEffect, useState } from "react";
import { api, ApiFailure, DocResponse, ListEntry, Membership } from "./api";

type Load<T> =
  | { state: "loading" }
  | { state: "error"; message: string }
  | { state: "ready"; data: T };

export function DocumentsView({ tenant }: { tenant: Membership }) {
  const [entries, setEntries] = useState<Load<ListEntry[]>>({ state: "loading" });
  const [selected, setSelected] = useState<string | null>(null);
  const [detail, setDetail] = useState<Load<DocResponse> | null>(null);

  useEffect(() => {
    let cancelled = false;
    setEntries({ state: "loading" });
    setSelected(null);
    setDetail(null);
    api
      .docList(tenant.tenant_id)
      .then((list) => {
        if (!cancelled) setEntries({ state: "ready", data: list.entries });
      })
      .catch((e) => {
        if (cancelled) return;
        setEntries({
          state: "error",
          message: e instanceof ApiFailure ? e.message : String(e),
        });
      });
    return () => {
      cancelled = true;
    };
  }, [tenant.tenant_id]);

  useEffect(() => {
    if (!selected) {
      setDetail(null);
      return;
    }
    let cancelled = false;
    setDetail({ state: "loading" });
    api
      .docRead(tenant.tenant_id, selected)
      .then((d) => {
        if (!cancelled) setDetail({ state: "ready", data: d });
      })
      .catch((e) => {
        if (cancelled) return;
        setDetail({
          state: "error",
          message: e instanceof ApiFailure ? e.message : String(e),
        });
      });
    return () => {
      cancelled = true;
    };
  }, [selected, tenant.tenant_id]);

  return (
    <div className="flex h-full min-h-0">
      <aside className="w-72 border-r border-neutral-200 bg-white overflow-y-auto">
        {entries.state === "loading" && (
          <div className="p-4 text-sm text-neutral-500">Loading…</div>
        )}
        {entries.state === "error" && (
          <div className="p-4 text-sm text-red-600">{entries.message}</div>
        )}
        {entries.state === "ready" && entries.data.length === 0 && (
          <div className="p-4 text-sm text-neutral-500">
            No documents yet.
          </div>
        )}
        {entries.state === "ready" && entries.data.length > 0 && (
          <ul>
            {entries.data.map((e) => (
              <li key={e.doc_id}>
                <button
                  onClick={() => setSelected(e.doc_id)}
                  className={
                    "w-full text-left px-4 py-2 text-sm border-l-2 " +
                    (selected === e.doc_id
                      ? "border-brand-500 bg-brand-50"
                      : "border-transparent hover:bg-neutral-50")
                  }
                >
                  <div className="font-medium truncate">{e.title || e.doc_id}</div>
                  <div className="text-xs text-neutral-500 truncate">
                    {e.type_} · {e.visibility}
                  </div>
                </button>
              </li>
            ))}
          </ul>
        )}
      </aside>
      <section className="flex-1 min-w-0 overflow-y-auto">
        {!selected && (
          <div className="p-6 text-sm text-neutral-500">
            Select a document.
          </div>
        )}
        {detail?.state === "loading" && (
          <div className="p-6 text-sm text-neutral-500">Loading…</div>
        )}
        {detail?.state === "error" && (
          <div className="p-6 text-sm text-red-600">{detail.message}</div>
        )}
        {detail?.state === "ready" && (
          <pre className="p-6 whitespace-pre-wrap text-sm font-mono leading-6">
            {detail.data.source}
          </pre>
        )}
      </section>
    </div>
  );
}
