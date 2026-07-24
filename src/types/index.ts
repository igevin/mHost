export type ProfileMode = "hosts" | "dns";

export interface Profile {
  id: string;
  name: string;
  description: string | null;
  enabled: boolean;
  protected: boolean;
  tags: string[];
  rules: HostRule[];
  mode: ProfileMode;
  created_at: string; // ISO 8601
  updated_at: string;
}

/// Enable 时捕获的系统 DNS 快照（语义版本，与 Rust `OriginalDns` 同步）。
/// - `manual`: 用户在 System Settings 里手动配的；disable 时回写 servers
/// - `dhcp_empty`: 用户没手动配；disable 时写 `Empty`（DHCP default），
///   避免跨网络切换时泄漏上次抓到的 DHCP 推的 IP
export type OriginalDns =
  | { kind: "manual"; servers: string[] }
  | { kind: "dhcp_empty" };

export interface DnsStatus {
  running: boolean;
  port: number;
  upstream: string[];
  /// Enable 时捕获的系统 DNS 快照（disable 时按语义还原）。详见 `OriginalDns`。
  original_dns: OriginalDns;
  rule_count: number;
  cache_capacity: number;
}

export interface HostRule {
  id: string;
  ip: string | null;
  domains: string[];
  enabled: boolean;
  comment: string | null;
  source: RuleSource;
}

export type RuleSource =
  | { type: "Manual" }
  | { type: "Remote"; source_id: string; source_name: string }
  | { type: "AdBlock"; source_id: string; source_name: string };

export interface ApplyPlan {
  rules: ResolvedRule[];
  conflicts: RuleConflict[];
  diff: HostsDiff;
  backup_required: boolean;
}

export interface ResolvedRule {
  ip: string;
  domain: string;
  source_profile_id: string;
  source_profile_name: string;
}

export interface RuleConflict {
  domain: string;
  rules: ResolvedRule[];
}

export interface HostsDiff {
  added: string[];
  removed: string[];
  unchanged: string[];
}

/// Strongly typed result of an apply (or previewed apply).
/// Mirrors Rust `mhost_core::ApplyOutcome` — keep these in sync.
export interface ApplyOutcome {
  plan: ApplyPlan;
  added_count: number;
  removed_count: number;
  unchanged_count: number;
  disabled_profile_ids: string[];
  has_conflicts: boolean;
  snapshot_id: string | null;
  backup_path: string | null;
}

/// Mirrors Rust `mhost_core::ApplyMode` (snake_case wire format).
export type ApplyMode = "quick_apply" | "require_preview";

export type AppError =
  | { type: "Parse"; message: string }
  | { type: "Apply"; message: string }
  | { type: "Storage"; message: string }
  | { type: "Io"; message: string }
  | { type: "Network"; message: string }
  | { type: "ExternalApi"; message: string }
  | { type: "InvalidInput"; message: string };

export interface ParseErrorAtLine {
  line_number: number;
  error: string | Record<string, string>;
}

export interface DuplicateRule {
  domain: string;
  lines: number[];
  kind: "same_ip" | "different_ip";
}

export interface ValidateResult {
  rules: HostRule[];
  errors: ParseErrorAtLine[];
  duplicates: DuplicateRule[];
}

export type ExportFormat = "hosts" | "json";

export interface Snapshot {
  id: string;
  name: string;
  description?: string;
  profiles: Profile[];
  created_at: string;
}

export interface SnapshotMeta {
  id: string;
  name: string;
  description?: string;
  profile_count: number;
  created_at: string;
}
