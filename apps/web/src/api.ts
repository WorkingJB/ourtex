// Thin HTTP client against ourtex-server. Mirrors the desktop `api`
// object's surface where practical so shared views stay portable.
//
// Dev server proxies `/v1/*` → the ourtex-server (see vite.config.ts);
// production builds will need either same-origin hosting or an explicit
// OURTEX_API_BASE build-time constant.
import { loadSession } from "./session";

export type ApiError = {
  tag: string;
  message: string;
  status: number;
};

export class ApiFailure extends Error {
  tag: string;
  status: number;
  constructor(err: ApiError) {
    super(err.message);
    this.tag = err.tag;
    this.status = err.status;
  }
}

async function request<T>(
  method: string,
  path: string,
  body?: unknown
): Promise<T> {
  const headers: Record<string, string> = {};
  const session = loadSession();
  if (session) headers["Authorization"] = `Bearer ${session.token}`;
  if (body !== undefined) headers["Content-Type"] = "application/json";

  const res = await fetch(path, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });

  if (res.status === 204) return undefined as T;
  const text = await res.text();
  const parsed = text ? JSON.parse(text) : null;
  if (!res.ok) {
    const err = parsed?.error ?? { tag: "server_error", message: res.statusText };
    throw new ApiFailure({ ...err, status: res.status });
  }
  return parsed as T;
}

// ---------- Auth ----------

export type AccountDto = {
  id: string;
  email: string;
  display_name: string;
  created_at: string;
};

export type SessionIssued = {
  id: string;
  secret: string;
  expires_at: string;
};

export type LoginResponse = { account: AccountDto; session: SessionIssued };
export type SignupResponse = { account: AccountDto; session: SessionIssued };

// ---------- Tenants ----------

export type Membership = {
  tenant_id: string;
  name: string;
  kind: string;
  role: string;
  created_at: string;
};

// ---------- Documents ----------

export type ListEntry = {
  doc_id: string;
  type_: string;
  visibility: string;
  title: string;
  updated: string | null;
  tags: string[];
};

export type DocResponse = {
  doc_id: string;
  type_: string;
  visibility: string;
  version: string;
  updated_at: string;
  source: string;
};

export type WriteResponse = {
  doc_id: string;
  type_: string;
  visibility: string;
  version: string;
  updated_at: string;
};

export const VISIBILITIES = ["public", "work", "personal", "private"] as const;

export const SEED_TYPES = [
  "identity",
  "roles",
  "goals",
  "relationships",
  "memories",
  "tools",
  "preferences",
  "domains",
  "decisions",
] as const;

// ---------- Tokens ----------

export type PublicToken = {
  id: string;
  label: string;
  scope: string[];
  mode: string; // "read" | "read_propose"
  max_docs: number;
  max_bytes: number;
  created_at: string;
  expires_at: string;
  last_used_at: string | null;
  revoked_at: string | null;
};

export type IssueTokenRequest = {
  label: string;
  scope: string[];
  mode?: "read" | "read_propose";
  ttl_days?: number | null;
};

export type IssueTokenResponse = {
  secret: string;
  token: PublicToken;
};

// ---------- Audit ----------

export type AuditRow = {
  seq: number;
  ts: string;
  actor: string;
  action: string;
  document_id: string | null;
  scope_used: string[];
  outcome: string;
  prev_hash: string;
  hash: string;
};

export type AuditResponse = {
  entries: AuditRow[];
  head_hash: string | null;
};

// ---------- Crypto state ----------

export type CryptoState = {
  seeded: boolean;
  kdf_salt: string | null;
  wrapped_content_key: string | null;
  key_version: number | null;
  unlocked: boolean;
};

export const api = {
  login: (email: string, password: string, label?: string) =>
    request<LoginResponse>("POST", "/v1/auth/login", {
      email,
      password,
      label,
    }),
  signup: (email: string, password: string, display_name?: string) =>
    request<SignupResponse>("POST", "/v1/auth/signup", {
      email,
      password,
      display_name,
    }),
  logout: () => request<void>("DELETE", "/v1/auth/logout"),
  me: () =>
    request<{ account: AccountDto; session_id: string }>("GET", "/v1/auth/me"),

  tenants: () =>
    request<{ memberships: Membership[] }>("GET", "/v1/tenants"),

  docList: (tenantId: string) =>
    request<{ entries: ListEntry[] }>(
      "GET",
      `/v1/t/${tenantId}/vault/docs`
    ),
  docRead: (tenantId: string, docId: string) =>
    request<DocResponse>(
      "GET",
      `/v1/t/${tenantId}/vault/docs/${encodeURIComponent(docId)}`
    ),
  docWrite: (
    tenantId: string,
    docId: string,
    source: string,
    baseVersion: string | null
  ) =>
    request<WriteResponse>(
      "PUT",
      `/v1/t/${tenantId}/vault/docs/${encodeURIComponent(docId)}`,
      baseVersion === null ? { source } : { source, base_version: baseVersion }
    ),
  docDelete: (tenantId: string, docId: string, baseVersion: string | null) => {
    const q = baseVersion
      ? `?base_version=${encodeURIComponent(baseVersion)}`
      : "";
    return request<void>(
      "DELETE",
      `/v1/t/${tenantId}/vault/docs/${encodeURIComponent(docId)}${q}`
    );
  },

  tokenList: (tenantId: string) =>
    request<{ tokens: PublicToken[] }>("GET", `/v1/t/${tenantId}/tokens`),
  tokenIssue: (tenantId: string, input: IssueTokenRequest) =>
    request<IssueTokenResponse>("POST", `/v1/t/${tenantId}/tokens`, input),
  tokenRevoke: (tenantId: string, tokenId: string) =>
    request<void>(
      "DELETE",
      `/v1/t/${tenantId}/tokens/${encodeURIComponent(tokenId)}`
    ),

  auditList: (tenantId: string, limit = 500, after?: number) => {
    const params = new URLSearchParams();
    params.set("limit", String(limit));
    if (after !== undefined) params.set("after", String(after));
    return request<AuditResponse>(
      "GET",
      `/v1/t/${tenantId}/audit?${params.toString()}`
    );
  },

  cryptoState: (tenantId: string) =>
    request<CryptoState>("GET", `/v1/t/${tenantId}/vault/crypto`),
  initCrypto: (
    tenantId: string,
    saltWire: string,
    wrappedContentKey: string
  ) =>
    request<{ key_version: number }>(
      "POST",
      `/v1/t/${tenantId}/vault/init-crypto`,
      { kdf_salt: saltWire, wrapped_content_key: wrappedContentKey }
    ),
  publishSessionKey: (tenantId: string, contentKeyWire: string) =>
    request<{ expires_at: string; ttl_seconds: number }>(
      "POST",
      `/v1/t/${tenantId}/session-key`,
      { key: contentKeyWire }
    ),
  revokeSessionKey: (tenantId: string) =>
    request<void>("DELETE", `/v1/t/${tenantId}/session-key`),
};
