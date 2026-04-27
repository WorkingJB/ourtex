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
  kind: string;
  path: string;
  active: boolean;
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

export const api = {
  workspaceList: () => invoke<WorkspaceInfo[]>("workspace_list"),
  workspaceAdd: (path: string, name?: string | null) =>
    invoke<VaultInfo>("workspace_add", { path, name: name ?? null }),
  workspaceActivate: (id: string) =>
    invoke<VaultInfo>("workspace_activate", { id }),
  workspaceRemove: (id: string) => invoke<void>("workspace_remove", { id }),
  workspaceRename: (id: string, name: string) =>
    invoke<void>("workspace_rename", { id, name }),
  vaultInfo: () => invoke<VaultInfo | null>("vault_info"),
  docList: () => invoke<DocListItem[]>("doc_list"),
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
