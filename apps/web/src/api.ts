// Thin HTTP client against orchext-server. Mirrors the desktop `api`
// object's surface where practical so shared views stay portable.
//
// Dev server proxies `/v1/*` → the orchext-server (see vite.config.ts);
// production builds will need either same-origin hosting or an explicit
// ORCHEXT_API_BASE build-time constant.
//
// Auth: cookie-based. The browser sends `orchext_session` (HttpOnly)
// automatically on every same-origin request; we attach
// `X-Orchext-CSRF` to the readable `orchext_csrf` cookie value on
// state-changing requests (double-submit pattern). All fetches use
// `credentials: 'include'` so the cookies are actually sent.
import { getCsrfToken } from "./session";

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

const MUTATING = new Set(["POST", "PUT", "PATCH", "DELETE"]);

async function request<T>(
  method: string,
  path: string,
  body?: unknown
): Promise<T> {
  const headers: Record<string, string> = {};
  if (body !== undefined) headers["Content-Type"] = "application/json";
  if (MUTATING.has(method)) {
    const csrf = getCsrfToken();
    if (csrf) headers["X-Orchext-CSRF"] = csrf;
  }

  const res = await fetch(path, {
    method,
    headers,
    credentials: "include",
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

// The browser auth endpoints intentionally do NOT return the bearer
// secret. The session reaches the browser only through the HttpOnly
// `orchext_session` cookie set in the same response — JS, the network
// tab, and any XSS therefore can't read a transferable token.
export type BrowserSession = {
  id: string;
  expires_at: string;
};

export type LoginResponse = { account: AccountDto; session: BrowserSession };
export type SignupResponse = { account: AccountDto; session: BrowserSession };

// ---------- Tenants ----------

export type Membership = {
  tenant_id: string;
  name: string;
  kind: string;
  role: string;
  created_at: string;
};

// ---------- Organizations (Phase 3 platform Slice 1) ----------

/// Returned by `GET /v1/orgs` — enriched view of org-tenant
/// memberships joined with the `organizations` row.
export type OrgMembership = {
  org_id: string;
  tenant_id: string;
  name: string;
  logo_url: string | null;
  role: "owner" | "admin" | "org_editor" | "member";
  joined_at: string;
};

/// Pending signup row visible to the requesting account on `GET /v1/orgs`.
export type PendingSignup = {
  id: string;
  org_id: string;
  org_name: string;
  requested_role: string;
  status: "pending" | "approved" | "rejected";
  requested_at: string;
};

export type OrgsResponse = {
  memberships: OrgMembership[];
  pending: PendingSignup[];
};

export type Organization = {
  id: string;
  tenant_id: string;
  name: string;
  logo_url: string | null;
  allowed_domains: string[];
  settings: Record<string, unknown>;
  created_at: string;
};

export type MemberDetail = {
  account_id: string;
  email: string;
  display_name: string;
  role: "owner" | "admin" | "org_editor" | "member";
  joined_at: string;
};

/// Admin pending-queue row (richer than `PendingSignup` — includes
/// the requesting account's email + display_name).
export type PendingDetail = {
  id: string;
  account_id: string;
  email: string;
  display_name: string;
  requested_role: string;
  status: "pending" | "approved" | "rejected";
  note: string | null;
  requested_at: string;
};

export type UpdateOrgInput = {
  name?: string;
  logo_url?: string | null;
  allowed_domains?: string[];
  settings?: Record<string, unknown>;
};

export type Invitation = {
  id: string;
  org_id: string;
  email: string;
  role: "owner" | "admin" | "org_editor" | "member";
  invited_by: string;
  invited_at: string;
  redeemed_at: string | null;
  redeemed_by: string | null;
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

/// Visibility values offered when creating a doc in a personal vault.
/// `org` is excluded — there's no org to share with.
export const PERSONAL_VISIBILITIES = ["private", "personal", "work"] as const;

/// Visibility values offered when creating a doc in an org workspace.
/// `personal` and `work` are excluded — both collapse into "My notes
/// for [Org]" via `private` (Phase 3 platform 4-layer model).
export const ORG_VISIBILITIES = ["private", "org"] as const;

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
  // `org` is the seed type for org-shared business context (brand,
  // mission, top-level goals). Visible to all members; writes gated
  // by `org_editor`-or-higher (D17g).
  "org",
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

// ---------- OAuth ----------

export type OAuthAuthorizeRequest = {
  tenant_id: string;
  client_label: string;
  redirect_uri: string;
  scope: string[];
  mode: "read" | "read_propose";
  code_challenge: string;
  code_challenge_method: string;
  ttl_days: number | null;
  max_docs: number | null;
  max_bytes: number | null;
};

export type OAuthAuthorizeResponse = {
  code: string;
  redirect_uri: string;
  expires_in: number;
};

// ---------- Proposals ----------

// `frontmatter` is whatever JSON the agent sent — we don't constrain
// it here so the diff view can render unknown keys verbatim. The same
// is true of `body_replace` / `body_append`: if both arrived (which
// the server would have rejected, but defensively), the UI renders
// `body_replace` and ignores `body_append`.
export type ProposalPatch = {
  frontmatter?: Record<string, unknown> | null;
  body_replace?: string | null;
  body_append?: string | null;
};

export type Proposal = {
  id: string;
  doc_id: string;
  base_version: string;
  patch: ProposalPatch;
  reason: string | null;
  status: "pending" | "approved" | "rejected";
  actor_token_id: string | null;
  actor_token_label: string;
  actor_account_id: string | null;
  decided_by: string | null;
  decided_at: string | null;
  decision_note: string | null;
  applied_version: string | null;
  created_at: string;
};

export type ApproveResponse = {
  proposal: Proposal;
  applied_version: string;
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

  // ---------- Orgs ----------
  orgsList: () => request<OrgsResponse>("GET", "/v1/orgs"),
  orgGet: (orgId: string) =>
    request<Organization>("GET", `/v1/orgs/${encodeURIComponent(orgId)}`),
  orgCreate: (name: string) =>
    request<Organization>("POST", "/v1/orgs", { name }),
  orgUpdate: (orgId: string, input: UpdateOrgInput) =>
    request<Organization>(
      "PATCH",
      `/v1/orgs/${encodeURIComponent(orgId)}`,
      input
    ),
  orgMembers: (orgId: string) =>
    request<{ members: MemberDetail[] }>(
      "GET",
      `/v1/orgs/${encodeURIComponent(orgId)}/members`
    ),
  orgMemberUpdate: (orgId: string, accountId: string, role: string) =>
    request<MemberDetail>(
      "PATCH",
      `/v1/orgs/${encodeURIComponent(orgId)}/members/${encodeURIComponent(accountId)}`,
      { role }
    ),
  orgMemberRemove: (orgId: string, accountId: string) =>
    request<void>(
      "DELETE",
      `/v1/orgs/${encodeURIComponent(orgId)}/members/${encodeURIComponent(accountId)}`
    ),
  orgPending: (
    orgId: string,
    status: "pending" | "approved" | "rejected" | "all" = "pending"
  ) =>
    request<{ pending: PendingDetail[] }>(
      "GET",
      `/v1/orgs/${encodeURIComponent(orgId)}/pending?status=${status}`
    ),
  orgPendingApprove: (orgId: string, accountId: string, role?: string) =>
    request<MemberDetail>(
      "POST",
      `/v1/orgs/${encodeURIComponent(orgId)}/pending/${encodeURIComponent(accountId)}/approve`,
      role ? { role } : {}
    ),
  orgPendingReject: (orgId: string, accountId: string) =>
    request<void>(
      "POST",
      `/v1/orgs/${encodeURIComponent(orgId)}/pending/${encodeURIComponent(accountId)}/reject`,
      {}
    ),
  orgInvitations: (
    orgId: string,
    status: "open" | "redeemed" | "all" = "open"
  ) =>
    request<{ invitations: Invitation[] }>(
      "GET",
      `/v1/orgs/${encodeURIComponent(orgId)}/invitations?status=${status}`
    ),
  orgInvite: (orgId: string, email: string, role?: string) =>
    request<Invitation>(
      "POST",
      `/v1/orgs/${encodeURIComponent(orgId)}/invitations`,
      role ? { email, role } : { email }
    ),
  orgInvitationDelete: (orgId: string, invitationId: string) =>
    request<void>(
      "DELETE",
      `/v1/orgs/${encodeURIComponent(orgId)}/invitations/${encodeURIComponent(invitationId)}`
    ),

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

  oauthAuthorize: (input: OAuthAuthorizeRequest) =>
    request<OAuthAuthorizeResponse>("POST", "/v1/oauth/authorize", input),

  tokenList: (tenantId: string) =>
    request<{ tokens: PublicToken[] }>("GET", `/v1/t/${tenantId}/tokens`),
  tokenIssue: (tenantId: string, input: IssueTokenRequest) =>
    request<IssueTokenResponse>("POST", `/v1/t/${tenantId}/tokens`, input),
  tokenRevoke: (tenantId: string, tokenId: string) =>
    request<void>(
      "DELETE",
      `/v1/t/${tenantId}/tokens/${encodeURIComponent(tokenId)}`
    ),

  proposalsList: (
    tenantId: string,
    status: "pending" | "approved" | "rejected" | "all" = "pending"
  ) =>
    request<{ proposals: Proposal[] }>(
      "GET",
      `/v1/t/${tenantId}/proposals?status=${status}`
    ),
  proposalApprove: (tenantId: string, proposalId: string, note?: string) =>
    request<ApproveResponse>(
      "POST",
      `/v1/t/${tenantId}/proposals/${encodeURIComponent(proposalId)}/approve`,
      { note: note ?? null }
    ),
  proposalReject: (tenantId: string, proposalId: string, note?: string) =>
    request<Proposal>(
      "POST",
      `/v1/t/${tenantId}/proposals/${encodeURIComponent(proposalId)}/reject`,
      { note: note ?? null }
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
    wrappedContentKey: string,
    keyCheck: string
  ) =>
    request<{ key_version: number }>(
      "POST",
      `/v1/t/${tenantId}/vault/init-crypto`,
      {
        kdf_salt: saltWire,
        wrapped_content_key: wrappedContentKey,
        key_check: keyCheck,
      }
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
