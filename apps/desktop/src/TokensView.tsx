import { useEffect, useMemo, useState } from "react";
import {
  api,
  IssuedToken,
  ORG_VISIBILITIES,
  PERSONAL_VISIBILITIES,
  TokenInfo,
} from "./api";
import { Context } from "./OrgRail";

export function TokensView({
  ctx,
  onMutated,
}: {
  /// Active context. The IssueForm offers a context-aware visibility
  /// scope set so org workspaces don't surface `work`/`personal` (no
  /// such docs there) and personal/local vaults don't surface `org`
  /// (no org membership). Phase 3 platform 4-layer model.
  ctx?: Context;
  onMutated?: () => void | Promise<void>;
}) {
  const [tokens, setTokens] = useState<TokenInfo[]>([]);
  const [issuing, setIssuing] = useState(false);
  const [justIssued, setJustIssued] = useState<IssuedToken | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function refresh() {
    try {
      setTokens(await api.tokenList());
      await onMutated?.();
    } catch (e) {
      setError(String(e));
    }
  }
  useEffect(() => {
    void refresh();
  }, []);

  return (
    <div className="p-6 max-w-4xl mx-auto">
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-lg font-semibold">Tokens</h2>
        <button
          onClick={() => setIssuing(true)}
          className="px-3 py-1.5 text-sm bg-brand-600 text-white rounded hover:bg-brand-700"
        >
          + Issue token
        </button>
      </div>

      {issuing && (
        <IssueForm
          ctxKind={ctx?.kind ?? "local"}
          onDone={async (t) => {
            setIssuing(false);
            setJustIssued(t);
            await refresh();
          }}
          onCancel={() => setIssuing(false)}
        />
      )}

      {justIssued && (
        <OneTimeSecret
          token={justIssued}
          onDismiss={() => setJustIssued(null)}
        />
      )}

      {error && (
        <div className="mb-4 p-3 bg-red-50 text-red-700 text-sm rounded-lg border border-red-200">
          {error}
        </div>
      )}

      <div className="bg-white border border-neutral-200 rounded-lg overflow-hidden">
        <table className="w-full text-sm">
          <thead className="bg-neutral-50 text-neutral-600 text-left text-xs uppercase tracking-wider">
            <tr>
              <th className="px-3 py-2">Label</th>
              <th className="px-3 py-2">Scope</th>
              <th className="px-3 py-2">Mode</th>
              <th className="px-3 py-2">Expires</th>
              <th className="px-3 py-2">Last used</th>
              <th className="px-3 py-2">Status</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {tokens.length === 0 && (
              <tr>
                <td colSpan={7} className="px-3 py-6 text-center text-neutral-500">
                  No tokens yet. Issue one to connect an MCP client.
                </td>
              </tr>
            )}
            {tokens.map((t) => (
              <tr key={t.id} className="border-t border-neutral-100">
                <td className="px-3 py-2">
                  <div className="font-medium">{t.label}</div>
                  <div className="text-xs text-neutral-500 font-mono">{t.id}</div>
                </td>
                <td className="px-3 py-2">
                  <div className="flex flex-wrap gap-1">
                    {t.scope.map((s) => (
                      <span
                        key={s}
                        className={
                          "inline-block px-1.5 py-0.5 rounded text-[10px] " +
                          (s === "private"
                            ? "bg-red-100 text-red-700"
                            : "bg-neutral-100 text-neutral-700")
                        }
                      >
                        {s}
                      </span>
                    ))}
                  </div>
                </td>
                <td className="px-3 py-2 text-neutral-600">{t.mode}</td>
                <td className="px-3 py-2 text-neutral-600">
                  {fmtDate(t.expires_at)}
                </td>
                <td className="px-3 py-2 text-neutral-600">
                  {t.last_used ? fmtDate(t.last_used) : "—"}
                </td>
                <td className="px-3 py-2">
                  {t.revoked ? (
                    <span className="text-red-600 text-xs">revoked</span>
                  ) : (
                    <span className="text-green-700 text-xs">active</span>
                  )}
                </td>
                <td className="px-3 py-2 text-right">
                  {!t.revoked && (
                    <button
                      onClick={async () => {
                        if (!confirm(`Revoke "${t.label}"?`)) return;
                        await api.tokenRevoke(t.id);
                        await refresh();
                      }}
                      className="text-xs text-red-600 hover:underline"
                    >
                      Revoke
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function fmtDate(iso: string): string {
  return new Date(iso).toLocaleDateString();
}

function IssueForm({
  ctxKind,
  onDone,
  onCancel,
}: {
  ctxKind: Context["kind"];
  onDone: (t: IssuedToken) => void | Promise<void>;
  onCancel: () => void;
}) {
  const [label, setLabel] = useState("Claude Desktop");
  const isOrg = ctxKind === "org";
  const visibilityChoices = useMemo<readonly string[]>(
    () =>
      isOrg
        ? ["public", ...ORG_VISIBILITIES]
        : ["public", ...PERSONAL_VISIBILITIES],
    [isOrg]
  );
  const [scope, setScope] = useState<Record<string, boolean>>(() => {
    const init: Record<string, boolean> = {};
    for (const v of visibilityChoices) {
      init[v] = isOrg
        ? v === "org" || v === "public"
        : v === "work" || v === "public";
    }
    return init;
  });
  const [ttlDays, setTtlDays] = useState<string>("90");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const selected = Object.entries(scope)
    .filter(([, on]) => on)
    .map(([k]) => k);

  async function submit() {
    setErr(null);
    setBusy(true);
    try {
      const ttl = ttlDays.trim() ? parseInt(ttlDays, 10) : null;
      const out = await api.tokenIssue({
        label: label.trim(),
        scope: selected,
        mode: "read",
        ttl_days: Number.isFinite(ttl as number) ? (ttl as number) : null,
      });
      await onDone(out);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mb-4 p-4 bg-white border border-neutral-200 rounded-lg">
      <div className="flex items-center justify-between mb-3">
        <h3 className="font-medium">Issue a new token</h3>
        <button
          onClick={onCancel}
          className="text-xs text-neutral-500 hover:text-neutral-900"
        >
          Cancel
        </button>
      </div>

      <div className="grid grid-cols-2 gap-3 mb-3">
        <label>
          <div className="text-xs text-neutral-600 mb-1">Label</div>
          <input
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            className="w-full px-3 py-1.5 border border-neutral-300 rounded text-sm"
          />
        </label>
        <label>
          <div className="text-xs text-neutral-600 mb-1">TTL (days)</div>
          <input
            value={ttlDays}
            onChange={(e) => setTtlDays(e.target.value)}
            type="number"
            min="1"
            max="365"
            className="w-full px-3 py-1.5 border border-neutral-300 rounded text-sm"
          />
        </label>
      </div>

      <div className="mb-3">
        <div className="text-xs text-neutral-600 mb-1">Scope</div>
        <div className="flex flex-wrap gap-3">
          {visibilityChoices.map((v) => (
            <label key={v} className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={scope[v] ?? false}
                onChange={(e) =>
                  setScope((prev) => ({ ...prev, [v]: e.target.checked }))
                }
              />
              {v}
            </label>
          ))}
        </div>
        {scope.private && (
          <div className="mt-2 text-xs text-red-700 bg-red-50 border border-red-200 p-2 rounded">
            This token will be able to read documents marked{" "}
            <span className="font-mono">private</span>. Only grant this to agents you trust explicitly.
          </div>
        )}
      </div>

      {err && (
        <div className="mb-3 p-2 bg-red-50 text-red-700 text-sm rounded border border-red-200">
          {err}
        </div>
      )}

      <button
        onClick={submit}
        disabled={busy || !label.trim() || selected.length === 0}
        className="px-3 py-1.5 text-sm bg-brand-600 text-white rounded hover:bg-brand-700 disabled:opacity-50"
      >
        {busy ? "Issuing…" : "Issue"}
      </button>
    </div>
  );
}

function OneTimeSecret({
  token,
  onDismiss,
}: {
  token: IssuedToken;
  onDismiss: () => void;
}) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="mb-4 p-4 bg-amber-50 border border-amber-200 rounded-lg">
      <div className="flex items-center justify-between mb-2">
        <h3 className="font-medium text-amber-900">
          Token issued — copy the secret now
        </h3>
        <button
          onClick={onDismiss}
          className="text-xs text-amber-700 hover:text-amber-900"
        >
          Dismiss
        </button>
      </div>
      <p className="text-sm text-amber-800 mb-3">
        This is the only time the full secret will be shown. After you dismiss
        this panel, only the token ID remains visible.
      </p>
      <div className="flex gap-2">
        <code className="flex-1 bg-white border border-amber-200 px-3 py-2 rounded text-sm font-mono break-all">
          {token.secret}
        </code>
        <button
          onClick={() => {
            navigator.clipboard.writeText(token.secret);
            setCopied(true);
            setTimeout(() => setCopied(false), 1500);
          }}
          className="px-3 py-2 text-sm bg-amber-600 text-white rounded hover:bg-amber-700"
        >
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
    </div>
  );
}
