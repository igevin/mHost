import { invoke } from "@tauri-apps/api/core";
import type {
  Profile,
  ApplyPlan,
  ApplyOutcome,
  ValidateResult,
  ExportFormat,
  SnapshotMeta,
  DnsStatus,
  ProfileMode,
  AdBlockState,
  AdBlockLimits,
  AdBlockSource,
  AdBlockResponse,
  BlocklistFormat,
  AdBlockStats,
  AdBlockOverlapReport,
} from "../types";

// ---- Profile commands ----

export async function listProfiles(mode?: ProfileMode): Promise<Profile[]> {
  return invoke("list_profiles", { mode });
}

export async function getProfile(id: string): Promise<Profile> {
  return invoke("get_profile", { id });
}

export async function createProfile(name: string, mode?: ProfileMode): Promise<Profile> {
  return invoke("create_profile", { name, mode });
}

export async function updateProfile(profile: Profile): Promise<Profile> {
  // **fix issue #67 bug 2**: 显式带上 mode。后端 update_profile 默认从 disk
  // 读 mode，如果 disk 上的 mode 是错的（Hypothesis A：create 时
  // Tauri 反序列化 Option<ProfileMode> 漏掉 → 落盘为 Hosts default），
  // 编辑规则后仍然错。显式传 mode 后每次 update 都会强制 reassert。
  return invoke("update_profile", {
    id: profile.id,
    name: profile.name,
    description: profile.description,
    rules: profile.rules,
    mode: profile.mode,
  });
}

export async function deleteProfile(id: string): Promise<void> {
  return invoke("delete_profile", { id });
}

export async function setProfileEnabled(
  id: string,
  enabled: boolean,
): Promise<Profile> {
  return invoke("set_profile_enabled", { id, enabled });
}

// ---- Enable & Apply (single atomic command) ----

export async function enableAndApply(
  id: string,
  enabled: boolean,
  requireSafe = false,
): Promise<ApplyOutcome> {
  return invoke<ApplyOutcome>("enable_and_apply", { id, enabled, requireSafe });
}

/// Read-only IPC: compute what an `enableAndApply(id, enabled)` call would
/// produce, without writing anything. Refs #127.
export async function previewApplyOutcome(
  id: string,
  enabled: boolean,
): Promise<ApplyOutcome> {
  return invoke<ApplyOutcome>("preview_apply_outcome", { id, enabled });
}

// ---- Apply commands ----

export async function generateApplyPlan(): Promise<ApplyPlan> {
  return invoke("generate_apply_plan");
}

export async function applyHosts(): Promise<void> {
  return invoke("apply_hosts");
}

export async function rollbackHosts(): Promise<void> {
  return invoke("rollback_hosts");
}

export async function readSystemHosts(): Promise<string> {
  return invoke("read_system_hosts");
}

// ---- Validate commands ----

export async function validateHostsText(text: string): Promise<ValidateResult> {
  return invoke("validate_hosts_text", { text });
}

// ---- Import / Export / Duplicate commands ----

export async function importProfile(name: string, hostsText: string): Promise<Profile> {
  return invoke("import_profile", { name, hostsText });
}

export async function importProfileFromFile(name: string, path: string): Promise<Profile> {
  return invoke("import_profile_from_file", { name, path });
}

export async function exportProfile(id: string, format: ExportFormat): Promise<string> {
  return invoke("export_profile", { id, format });
}

export async function exportProfileToFile(id: string, format: ExportFormat, path: string): Promise<void> {
  return invoke("export_profile_to_file", { id, format, path });
}

export async function duplicateProfile(id: string, newName: string): Promise<Profile> {
  return invoke("duplicate_profile", { id, newName });
}

// ---- Hosts block commands ----

export async function getManagedBlockContent(): Promise<string | null> {
  return invoke("get_managed_block_content");
}

export async function getLastApplied(): Promise<string | null> {
  return invoke("get_last_applied");
}

export async function generatePreviewPlan(id: string, enabled: boolean): Promise<ApplyPlan> {
  return invoke("generate_preview_plan", { id, enabled });
}

// ---- Snapshot commands ----

export async function saveSnapshot(name: string, description?: string): Promise<SnapshotMeta> {
  return invoke<SnapshotMeta>("save_snapshot", { name, description });
}

export async function listSnapshots(): Promise<SnapshotMeta[]> {
  return invoke<SnapshotMeta[]>("list_snapshots");
}

export async function loadSnapshot(id: string): Promise<void> {
  return invoke<void>("load_snapshot", { id });
}

export async function deleteSnapshot(id: string): Promise<void> {
  return invoke<void>("delete_snapshot", { id });
}

// ---- DNS commands ----

const DNS_MODE_TIMEOUT_MS = 30_000;

/**
 * Wraps a Promise-returning function with a timeout. The returned promise
 * rejects with `message` if the inner function hasn't settled after `ms`.
 *
 * Note: rejecting does NOT cancel the inner operation — the underlying
 * invoke/child-process continues running until it completes or the page
 * is unloaded. Used here to recover the UI from a stuck osascript sudo
 * prompt (the actual cancel-via-kill is a known macOS osascript
 * limitation tracked separately).
 */
async function withTimeout<T>(
  fn: () => Promise<T>,
  ms: number,
  message: string,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      fn(),
      new Promise<T>((_resolve, reject) => {
        timer = setTimeout(() => reject(new Error(message)), ms);
      }),
    ]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

/**
 * Toggle DNS mode on or off.
 *
 * **issue #149 (Settings cancel button)**: accepts an optional
 * `AbortSignal` for symmetry with `fetch`-style APIs, but Tauri 2's
 * `invoke()` does NOT natively propagate the signal to the backend
 * (the in-flight Rust future keeps running after abort). The frontend
 * therefore tracks cancellation via the signal's abort event itself
 * (see `toggleDnsModeAtom`) and additionally fires the separate
 * `cancelDnsMode()` IPC so the Rust `CancellationToken` drives the
 * rollback.
 *
 * **Frontend 30s safety net (retained from Group 3 PR #167)**: the
 * `withTimeout` wrapper still recovers the UI if a stuck osascript
 * sudo prompt never resolves — the Rust-side `tokio::time::timeout`
 * leaks the spawn_blocking thread, so this Promise-level rejection is
 * the only path that actually unblocks the JS side. The user-facing
 * error message guides recovery (run `networksetup -setdnsservers
 * <interface> Empty` to undo a partial enable).
 */
export async function setDnsMode(
  enabled: boolean,
  options?: { signal?: AbortSignal },
): Promise<void> {
  // Tauri 2's `invoke` InvokeOptions doesn't expose `signal`; the
  // `options.signal` is consumed only by the surrounding tracking
  // logic in `toggleDnsModeAtom`. We still accept it here so the
  // call site matches the documented contract and is forward-
  // compatible if Tauri later adds native signal propagation.
  void options?.signal;
  return withTimeout(
    () => invoke<void>("set_dns_mode", { enabled }),
    DNS_MODE_TIMEOUT_MS,
    `DNS mode ${enabled ? "enable" : "disable"} timed out after ${
      DNS_MODE_TIMEOUT_MS / 1000
    }s. ` +
      `The sudo prompt may have been dismissed or never shown. ` +
      `If your system DNS still points at 127.0.0.1, run \`networksetup -setdnsservers <interface> Empty\` manually.`,
  );
}

/**
 * Fire the backend `CancellationToken` for the in-flight `set_dns_mode`
 * call, causing it to roll back any committed side effects (proxy
 * startup, system DNS rewrite, manifest persist) and return
 * `MhostError::Cancelled`.
 *
 * **issue #149**: a no-op when no DNS operation is in flight. Safe to
 * call from the signal-abort handler even if the operation finished
 * milliseconds earlier.
 */
export async function cancelDnsMode(): Promise<void> {
  return invoke("cancel_dns_mode");
}

export async function getDnsMode(): Promise<boolean> {
  return invoke("get_dns_mode");
}

export async function reloadDnsRules(): Promise<void> {
  return invoke("reload_dns_rules");
}

export async function getDnsStatus(): Promise<DnsStatus> {
  return invoke("get_dns_status");
}

export async function listDnsProfiles(): Promise<Profile[]> {
  return invoke("list_dns_profiles");
}

// ---- AdBlock commands (issue #130) ----

export async function getAdBlockState(): Promise<AdBlockState> {
  return invoke<AdBlockState>("get_ad_block_state");
}

/** Issue #211-3: the backend's compile-time ad-block limits (single source
 * of truth for the override-entry gating in the UI). */
export async function getAdBlockLimits(): Promise<AdBlockLimits> {
  return invoke<AdBlockLimits>("get_ad_block_limits");
}

/** Issue #199 sub-task B: cumulative ad-block engine hit / miss
 * counters. Returns zeros if DNS mode is off. */
export async function getAdBlockStats(): Promise<AdBlockStats> {
  return invoke<AdBlockStats>("get_ad_block_stats");
}

export async function setAdBlockEnabled(enabled: boolean): Promise<void> {
  return invoke<void>("set_ad_block_enabled", { enabled });
}

export async function setAdBlockRefreshInterval(hours: number): Promise<void> {
  return invoke<void>("set_ad_block_refresh_interval", { hours });
}

export async function setAdBlockAutoRefreshEnabled(enabled: boolean): Promise<void> {
  return invoke<void>("set_ad_block_auto_refresh_enabled", { enabled });
}

export async function listAdBlockSources(): Promise<AdBlockSource[]> {
  return invoke<AdBlockSource[]>("list_ad_block_sources");
}

/**
 * Issue #215 §1: cross-source overlap report. Computed
 * lazily on the server (only when this IPC is called) —
 * not part of `getAdBlockState()` so the page-load IPC
 * stays cheap. The UI calls this when the overlap chip
 * is clicked (drawer open) and on first mount with a
 * small refresh strategy if needed.
 */
export async function getAdBlockOverlaps(): Promise<AdBlockOverlapReport> {
  return invoke<AdBlockOverlapReport>("get_ad_block_overlaps");
}

export async function addAdBlockSource(
  name: string,
  url: string,
  response: AdBlockResponse,
  format: BlocklistFormat,
): Promise<AdBlockSource> {
  return invoke<AdBlockSource>("add_ad_block_source", { name, url, response, format });
}

export async function removeAdBlockSource(sourceId: string): Promise<void> {
  return invoke<void>("remove_ad_block_source", { sourceId });
}

export async function setAdBlockSourceEnabled(
  sourceId: string,
  enabled: boolean,
): Promise<AdBlockSource> {
  return invoke<AdBlockSource>("set_ad_block_source_enabled", {
    sourceId,
    enabled,
  });
}

export async function setAdBlockSourceResponse(
  sourceId: string,
  response: AdBlockResponse,
): Promise<AdBlockSource> {
  return invoke<AdBlockSource>("set_ad_block_source_response", {
    sourceId,
    response,
  });
}

/** Issue #207: raise (`limit`) or revoke (`null`) a source's rules-cap
 * override. The retry is a separate call to `refreshAdBlockSource`. */
export async function setAdBlockSourceRulesLimitOverride(
  sourceId: string,
  limit: number | null,
): Promise<AdBlockSource> {
  return invoke<AdBlockSource>("set_ad_block_source_rules_limit_override", {
    sourceId,
    limit,
  });
}

export async function refreshAdBlockSource(
  sourceId: string,
): Promise<AdBlockSource> {
  return invoke<AdBlockSource>("refresh_ad_block_source", { sourceId });
}

/**
 * Issue #215: move a source up or down in the display order.
 * Boundary moves (first → up, last → down) are no-ops on the
 * server, so the caller does NOT need to disable the buttons
 * first — but the UI does so the user gets immediate feedback
 * (a disabled button is less surprising than an apparently
 * un-actionable click).
 *
 * Returns the full (re-ordered) source list so the caller can
 * patch its local state without an extra `getAdBlockState`
 * round trip.
 */
export async function reorderAdBlockSources(
  sourceId: string,
  direction: "up" | "down",
): Promise<AdBlockSource[]> {
  return invoke<AdBlockSource[]>("reorder_ad_block_sources", {
    sourceId,
    direction,
  });
}

export async function refreshAllAdBlockSources(): Promise<AdBlockSource[]> {
  return invoke<AdBlockSource[]>("refresh_all_ad_block_sources");
}

export async function listAdBlockWhitelist(): Promise<string[]> {
  return invoke<string[]>("list_ad_block_whitelist");
}

export async function addAdBlockWhitelist(domain: string): Promise<string[]> {
  return invoke<string[]>("add_ad_block_whitelist", { domain });
}

export async function removeAdBlockWhitelist(domain: string): Promise<string[]> {
  return invoke<string[]>("remove_ad_block_whitelist", { domain });
}

/** Issue #196: per-entry failure from a bulk whitelist paste. The frontend
 * toasts `reason` (a human-readable validator message) so the user can
 * spot which lines were rejected without losing the rest of the batch. */
export interface WhitelistInputError {
  input: string;
  reason: string;
}

/** Issue #196: result of a bulk whitelist add. `whitelist` is the full
 * current state (canonical, post-normalization); `rejected` lists the
 * entries the validator refused. Duplicates are silently deduplicated
 * — they don't appear in `rejected` because the user pasting 200 lines
 * into a list that already contains 50 of them shouldn't spam the toast. */
export interface AddWhitelistManyResult {
  whitelist: string[];
  rejected: WhitelistInputError[];
}

/** Bulk add — triggers exactly one persist + DNS-reload cycle
 * regardless of how many entries are pasted. Single-entry
 * `addAdBlockWhitelist` is a thin wrapper over the same backend path. */
export async function addAdBlockWhitelistMany(
  domains: string[],
): Promise<AddWhitelistManyResult> {
  return invoke<AddWhitelistManyResult>("add_ad_block_whitelist_many", {
    domains,
  });
}

/** Bulk remove — same trim+lowercase normalization as the single-entry
 * variant, missing entries are silently ignored (no rejected list). */
export async function removeAdBlockWhitelistMany(
  domains: string[],
): Promise<string[]> {
  return invoke<string[]>("remove_ad_block_whitelist_many", { domains });
}

// ---- Update commands ----

export interface LatestRelease {
  tag: string;
  url: string;
  title: string | null;
  body: string | null;
}

export async function checkUpdate(currentVersion: string): Promise<LatestRelease | null> {
  return invoke<LatestRelease | null>("check_update", { currentVersion });
}
