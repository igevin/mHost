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

// ---------------------------------------------------------------------------
// AdBlock (issue #130)
//
// Mirrors `mhost_core::AdBlockResponse` / `AdBlockSource` / `AdBlockState`.
// Keep wire format in sync: snake_case from serde rename_all = "snake_case"
// gives us "zero_address" and "nx_domain" on the wire.
// ---------------------------------------------------------------------------

export type AdBlockResponse = "zero_address" | "nx_domain";

export interface AdBlockSource {
  source_id: string;
  name: string;
  url: string;
  enabled: boolean;
  response: AdBlockResponse;
  last_fetched_at: string | null;
  last_error: string | null;
  rule_count: number;
  etag: string | null;
  /**
   * Per-source raise of the global rules cap (issue #207). Serialized
   * unconditionally by the backend (always `null` when unset), so no
   * undefined-vs-null mismatch here (cf. issue #202).
   */
  rules_limit_override: number | null;
  /**
   * Issue #199 sub-task B: wall-clock duration of the last
   * `fetch_and_cache_source` call (success, 304, or failure).
   * Always serialized (always `null` when unset, never `undefined`).
   */
  last_refresh_duration_ms: number | null;
  /**
   * Issue #199 sub-task B: RFC 3339 timestamp of the last *failed*
   * fetch. Distinct from `last_error` (which carries the message
   * of the most recent failure regardless of when). Cleared on
   * the next successful fetch.
   */
  last_refresh_failed_at: string | null;
}

/**
 * Issue #199 sub-task B: cumulative ad-block engine counters
 * returned by `getAdBlockStats()`. Numbers are monotonically
 * increasing since process start; the consumer computes deltas.
 * `enabled` mirrors the current master switch value at the time
   of the call.
 */
export interface AdBlockStats {
  hits_zero_addr: number;
  hits_nxdomain: number;
  hits_whitelist: number;
  misses: number;
  enabled: boolean;
}

export interface AdBlockState {
  enabled: boolean;
  sources: AdBlockSource[];
  whitelist: string[];
  auto_refresh_enabled: boolean;
  refresh_interval_hours: number;
}

/** Issue #211-3: compile-time backend limits delivered over IPC instead of
 * mirrored in the frontend (mirrors drift silently). */
export interface AdBlockLimits {
  rules_per_source_default: number;
  rules_per_source_absolute_max: number;
}

/**
 * Issue #215 §1: cross-source overlap report. Returned by
 * `getAdBlockOverlaps()`. Each entry in `per_source` matches a
 * row in `AdBlockState.sources` (enabled only — disabled sources
 * are never in the report). `details` is keyed by the same
 * SourceId and contains the per-domain breakdown for the drawer.
 */
export interface OverlapSummary {
  source_id: string;
  source_name: string;
  overlapping_domain_count: number;
}

/** One row in the overlap drill-down — a domain that is covered
 * by the owner source AND at least one other source. */
export interface OverlapEntry {
  domain: string;
  covered_by: OverlapSourceRef[];
  /** What `check()` will return for this domain — derived from
   * the priority chain whitelist > nxdomain > zero_addr. One of
   * "Whitelisted" | "NxDomain" | "ZeroAddress". */
  effective: string;
}

export interface OverlapSourceRef {
  source_id: string;
  name: string;
  response: AdBlockResponse;
}

export interface AdBlockOverlapReport {
  per_source: OverlapSummary[];
  details: Record<string, OverlapEntry[]>;
}

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
