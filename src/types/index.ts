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

export interface DnsStatus {
  running: boolean;
  port: number;
  upstream: string[];
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

export type AppError =
  | { type: "Parse"; message: string }
  | { type: "Apply"; message: string }
  | { type: "Storage"; message: string }
  | { type: "Io"; message: string }
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
