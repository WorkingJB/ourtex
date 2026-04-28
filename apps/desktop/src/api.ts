import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

export type VaultInfo = {
  workspace_id: string;
  name: string;
  root: string;
  document_count: number;
};

export type WorkspaceInfo = {
  id: string;
  name: string;
  /// `"local"` | `"remote"`.
  kind: string;
  path: string;
  active: boolean;
  /// Remote workspaces only. Frontend uses these to build the rail
  /// and to call server-scoped endpoints (`/v1/orgs/*`).
  server_url?: string;
  tenant_id?: string;
  account_email?: string;
};

export type DocListItem = {
  id: string;
  type: string;
  title: string;
  visibility: string;
  tags: string[];
  updated: string | null;
};

export type DocDetail = {
  id: string;
  type: string;
  visibility: string;
  tags: string[];
  links: string[];
  aliases: string[];
  source: string | null;
  created: string | null;
  updated: string | null;
  body: string;
  version: string;
  /// Team binding for `visibility = 'team'` docs (Phase 3 platform
  /// Slice 2). Always absent for local workspaces.
  team_id?: string | null;
};

export type DocInput = {
  id: string;
  type: string;
  visibility: string;
  tags?: string[];
  links?: string[];
  aliases?: string[];
  source?: string | null;
  body: string;
  team_id?: string | null;
};

export type TokenInfo = {
  id: string;
  label: string;
  scope: string[];
  mode: "read" | "read_propose";
  created_at: string;
  expires_at: string;
  last_used: string | null;
  revoked: boolean;
};

export type IssuedToken = {
  info: TokenInfo;
  secret: string;
};

export type TokenIssueInput = {
  label: string;
  scope: string[];
  mode: "read" | "read_propose";
  ttl_days: number | null;
};

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

export type AuditRow = {
  seq: number;
  ts: string;
  actor: string;
  action: string;
  document_id: string | null;
  scope_used: string[];
  outcome: string;
};

export type AuditPage = {
  entries: AuditRow[];
  total: number;
  chain_valid: boolean;
};

export type VaultChanged = {
  type: string;
  id: string;
  kind: "upsert" | "remove";
};

export type SettingsInfo = {
  has_api_key: boolean;
};

export type ChatMessage = {
  role: "user" | "assistant";
  content: string;
};

export type OnboardingSeedDoc = {
  id: string;
  type: string;
  visibility: string;
  body: string;
};

// ---------- Remote connect ----------

export type ConnectRemoteInput = {
  server_url: string;
  email: string;
  password: string;
  name?: string | null;
  tenant_id?: string | null;
};

/// Mirrors the `ConnectRemoteOutcome` enum on the Rust side. The
/// `kind` discriminator follows serde's `rename_all = "snake_case"`.
export type ConnectRemoteOutcome =
  | { kind: "connected"; workspace: VaultInfo }
  | {
      kind: "pending_approval";
      account_email: string;
      server_url: string;
      pending: PendingSignup[];
    };

// ---------- Auth / accounts ----------

export type AccountInfo = {
  id: string;
  email: string;
  display_name: string;
  created_at: string;
};

export type MeResponse = {
  account: AccountInfo;
  session_id: string;
};

// ---------- Organizations (Phase 3 platform Slice 1) ----------

export type OrgMembership = {
  org_id: string;
  tenant_id: string;
  name: string;
  logo_url: string | null;
  role: "owner" | "admin" | "org_editor" | "member";
  joined_at: string;
};

export type PendingSignup = {
  id: string;
  org_id: string;
  org_name: string;
  requested_role: string;
  status: "pending" | "approved" | "rejected";
  requested_at: string;
};

export type OrgsListResponse = {
  memberships: OrgMembership[];
  pending: PendingSignup[];
};

export type Organization = {
  id: string;
  tenant_id: string;
  name: string;
  logo_url: string | null;
  /// Server returns this as a JSON value; in practice it's an array
  /// of strings, but we keep `unknown` so a future shape change at
  /// the server doesn't break the type.
  allowed_domains: unknown;
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

// ---------- Teams (Phase 3 platform Slice 2) ----------

export type Team = {
  id: string;
  org_id: string;
  name: string;
  slug: string;
  created_at: string;
};

export type TeamSummary = Team & {
  member_count: number;
  viewer_role: "manager" | "member" | null;
};

export type TeamMemberDetail = {
  account_id: string;
  email: string;
  display_name: string;
  role: "manager" | "member";
  joined_at: string;
};

export type LogoData = {
  data_url: string;
  content_type: string;
  etag: string | null;
};

export type LogoUploadResponse = {
  logo_url: string;
  content_type: string;
  sha256: string;
  bytes: number;
};

// ---------- API surface ----------

export const api = {
  workspaceList: () => invoke<WorkspaceInfo[]>("workspace_list"),
  workspaceAdd: (path: string, name?: string | null) =>
    invoke<VaultInfo>("workspace_add", { path, name: name ?? null }),
  workspaceConnectRemote: (input: ConnectRemoteInput) =>
    invoke<ConnectRemoteOutcome>("workspace_connect_remote", { input }),
  workspaceActivate: (id: string) =>
    invoke<VaultInfo>("workspace_activate", { id }),
  workspaceRemove: (id: string) => invoke<void>("workspace_remove", { id }),
  workspaceRename: (id: string, name: string) =>
    invoke<void>("workspace_rename", { id, name }),
  vaultInfo: () => invoke<VaultInfo | null>("vault_info"),
  docList: (opts?: { teamId?: string | null }) =>
    invoke<DocListItem[]>("doc_list", {
      teamId: opts?.teamId ?? null,
    }),
  docRead: (id: string) => invoke<DocDetail>("doc_read", { id }),
  docWrite: (input: DocInput) => invoke<DocDetail>("doc_write", { input }),
  docDelete: (id: string) => invoke<void>("doc_delete", { id }),
  tokenList: () => invoke<TokenInfo[]>("token_list"),
  tokenIssue: (input: TokenIssueInput) =>
    invoke<IssuedToken>("token_issue", { input }),
  tokenRevoke: (id: string) => invoke<void>("token_revoke", { id }),
  auditList: (limit?: number) =>
    invoke<AuditPage>("audit_list", { limit: limit ?? null }),
  proposalList: (status?: "pending" | "approved" | "rejected" | "all") =>
    invoke<Proposal[]>("proposal_list", { status: status ?? null }),
  proposalApprove: (id: string, note?: string) =>
    invoke<Proposal>("proposal_approve", { id, note: note ?? null }),
  proposalReject: (id: string, note?: string) =>
    invoke<Proposal>("proposal_reject", { id, note: note ?? null }),
  settingsStatus: () => invoke<SettingsInfo>("settings_status"),
  settingsSetApiKey: (apiKey: string) =>
    invoke<void>("settings_set_api_key", { apiKey }),
  onboardingChat: (history: ChatMessage[]) =>
    invoke<{ reply: string }>("onboarding_chat", { input: { history } }),
  onboardingFinalize: (history: ChatMessage[]) =>
    invoke<OnboardingSeedDoc[]>("onboarding_finalize", { input: { history } }),
  onboardingSave: (docs: OnboardingSeedDoc[]) =>
    invoke<number>("onboarding_save", { input: { docs } }),
  onVaultChanged: (cb: (evt: VaultChanged) => void): Promise<UnlistenFn> =>
    listen<VaultChanged>("vault://changed", (e) => cb(e.payload)),

  // ---------- Auth (per remote workspace) ----------
  authMe: (workspaceId: string) =>
    invoke<MeResponse>("auth_me", { workspaceId }),
  authLogout: (workspaceId: string) =>
    invoke<void>("auth_logout", { workspaceId }),
  authAccountUpdate: (workspaceId: string, displayName: string) =>
    invoke<AccountInfo>("auth_account_update", {
      workspaceId,
      input: { display_name: displayName },
    }),
  authPasswordChange: (
    workspaceId: string,
    currentPassword: string,
    newPassword: string
  ) =>
    invoke<void>("auth_password_change", {
      workspaceId,
      input: {
        current_password: currentPassword,
        new_password: newPassword,
      },
    }),

  // ---------- Orgs ----------
  orgsList: (workspaceId: string) =>
    invoke<OrgsListResponse>("orgs_list", { workspaceId }),
  orgCreate: (workspaceId: string, name: string) =>
    invoke<Organization>("org_create", {
      workspaceId,
      input: { name },
    }),
  orgGet: (workspaceId: string, orgId: string) =>
    invoke<Organization>("org_get", { workspaceId, orgId }),
  orgUpdate: (workspaceId: string, orgId: string, input: UpdateOrgInput) =>
    invoke<Organization>("org_update", { workspaceId, orgId, input }),
  orgMembers: (workspaceId: string, orgId: string) =>
    invoke<{ members: MemberDetail[] }>("org_members", {
      workspaceId,
      orgId,
    }),
  orgMemberUpdate: (
    workspaceId: string,
    orgId: string,
    accountId: string,
    role: string
  ) =>
    invoke<MemberDetail>("org_member_update", {
      workspaceId,
      orgId,
      accountId,
      input: { role },
    }),
  orgMemberRemove: (
    workspaceId: string,
    orgId: string,
    accountId: string
  ) =>
    invoke<void>("org_member_remove", {
      workspaceId,
      orgId,
      accountId,
    }),
  orgPending: (
    workspaceId: string,
    orgId: string,
    status: "pending" | "approved" | "rejected" | "all" = "pending"
  ) =>
    invoke<{ pending: PendingDetail[] }>("org_pending", {
      workspaceId,
      orgId,
      status,
    }),
  orgPendingApprove: (
    workspaceId: string,
    orgId: string,
    accountId: string,
    role?: string
  ) =>
    invoke<MemberDetail>("org_pending_approve", {
      workspaceId,
      orgId,
      accountId,
      input: role ? { role } : null,
    }),
  orgPendingReject: (
    workspaceId: string,
    orgId: string,
    accountId: string
  ) =>
    invoke<void>("org_pending_reject", {
      workspaceId,
      orgId,
      accountId,
    }),
  orgInvitations: (
    workspaceId: string,
    orgId: string,
    status: "open" | "redeemed" | "all" = "open"
  ) =>
    invoke<{ invitations: Invitation[] }>("org_invitations", {
      workspaceId,
      orgId,
      status,
    }),
  orgInvite: (
    workspaceId: string,
    orgId: string,
    email: string,
    role?: string
  ) =>
    invoke<Invitation>("org_invite", {
      workspaceId,
      orgId,
      input: role ? { email, role } : { email },
    }),
  orgInvitationDelete: (
    workspaceId: string,
    orgId: string,
    invitationId: string
  ) =>
    invoke<void>("org_invitation_delete", {
      workspaceId,
      orgId,
      invitationId,
    }),

  // ---------- Org logo (Slice 2) ----------
  orgLogoGet: (workspaceId: string, orgId: string) =>
    invoke<LogoData | null>("org_logo_get", { workspaceId, orgId }),
  orgLogoUpload: (workspaceId: string, orgId: string, path: string) =>
    invoke<LogoUploadResponse>("org_logo_upload", {
      workspaceId,
      orgId,
      path,
    }),
  orgLogoDelete: (workspaceId: string, orgId: string) =>
    invoke<void>("org_logo_delete", { workspaceId, orgId }),

  // ---------- Teams ----------
  teamsList: (workspaceId: string, orgId: string) =>
    invoke<{ teams: TeamSummary[] }>("teams_list", { workspaceId, orgId }),
  teamCreate: (workspaceId: string, orgId: string, name: string, slug?: string) =>
    invoke<Team>("team_create", {
      workspaceId,
      orgId,
      input: slug ? { name, slug } : { name },
    }),
  teamGet: (workspaceId: string, orgId: string, teamId: string) =>
    invoke<Team>("team_get", { workspaceId, orgId, teamId }),
  teamUpdate: (
    workspaceId: string,
    orgId: string,
    teamId: string,
    input: { name?: string; slug?: string }
  ) =>
    invoke<Team>("team_update", { workspaceId, orgId, teamId, input }),
  teamDelete: (workspaceId: string, orgId: string, teamId: string) =>
    invoke<void>("team_delete", { workspaceId, orgId, teamId }),
  teamMembers: (workspaceId: string, orgId: string, teamId: string) =>
    invoke<{ members: TeamMemberDetail[] }>("team_members", {
      workspaceId,
      orgId,
      teamId,
    }),
  teamMemberAdd: (
    workspaceId: string,
    orgId: string,
    teamId: string,
    accountId: string,
    role?: "manager" | "member"
  ) =>
    invoke<TeamMemberDetail>("team_member_add", {
      workspaceId,
      orgId,
      teamId,
      input: role ? { account_id: accountId, role } : { account_id: accountId },
    }),
  teamMemberUpdate: (
    workspaceId: string,
    orgId: string,
    teamId: string,
    accountId: string,
    role: "manager" | "member"
  ) =>
    invoke<TeamMemberDetail>("team_member_update", {
      workspaceId,
      orgId,
      teamId,
      accountId,
      input: { role },
    }),
  teamMemberRemove: (
    workspaceId: string,
    orgId: string,
    teamId: string,
    accountId: string
  ) =>
    invoke<void>("team_member_remove", {
      workspaceId,
      orgId,
      teamId,
      accountId,
    }),
};

// ---------- Visibility constants (FORMAT v0.2) ----------

export const VISIBILITIES = ["public", "work", "personal", "private"] as const;

/// Visibility values offered when creating a doc in a personal vault.
/// `org` is excluded — there's no org to share with.
export const PERSONAL_VISIBILITIES = ["private", "personal", "work"] as const;

/// Visibility values offered when creating a doc in an org workspace.
/// `personal` and `work` are excluded — both collapse into "My notes
/// for [Org]" via `private` (Phase 3 platform 4-layer model).
/// `team` is appended dynamically by the doc editor when the viewer
/// can write to at least one team.
export const ORG_VISIBILITIES = ["private", "org", "team"] as const;

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
