import { atom } from "jotai";
import type { ApplyOutcome, Profile, DnsStatus } from "../../types";
import { countRealRules } from "../../lib/rules";
import { atomWithLocalStorage } from "../../lib/atomWithLocalStorage";

// ---- Base atoms ----

export const profilesAtom = atom<Profile[]>([]);
export const selectedProfileIdAtom = atom<string | null>(null);
export const isApplyingAtom = atom(false);
export const errorAtom = atom<string | null>(null);
export const isLoadingAtom = atom(false);

// ---- DNS related atoms ----

export const dnsProfilesAtom = atom<Profile[]>([]);
export const dnsEnabledAtom = atom(false);
export const dnsStatusAtom = atom<DnsStatus | null>(null);
export const isDnsLoadingAtom = atom(false);
export const dnsErrorAtom = atom<string | null>(null);

// ---- Derived atoms ----

export const selectedProfileAtom = atom((get) => {
  const profiles = get(profilesAtom);
  const id = get(selectedProfileIdAtom);
  return profiles.find((p) => p.id === id) ?? null;
});

export const enabledProfileAtom = atom((get) => {
  const profiles = get(profilesAtom);
  return profiles.find((p) => p.enabled) ?? null;
});

/** issue #67：DNS 模式允许多 profile 同时启用。
 * 此派生 atom 列出所有启用的 DNS profile，供 StatusBar 摘要与 DnsProfileList 使用。 */
export const enabledDnsProfilesAtom = atom((get) =>
  get(dnsProfilesAtom).filter((p) => p.enabled),
);

/** 启用 DNS profile 的规则总数（跨 profile 求和）。
 * 纯派生：不需要后端调用；`dnsStatusAtom.rule_count` 来自 `RuleEngine` 内存视图，
 * 此 atom 用于 UI 摘要；两侧数据源独立，但启用态下应一致。 */
export const dnsRuleCountAtom = atom((get) =>
  get(enabledDnsProfilesAtom).reduce(
    (sum, p) => sum + countRealRules(p.rules),
    0,
  ),
);

// ---- Apply confirm dialog atoms ----

export const applyConfirmOpenAtom = atom(false);
export const applyPlanAtom = atom<import("../../types").ApplyPlan | null>(null);
export const applyResultAtom = atom<"success" | "error" | null>(null);
export const applyErrorAtom = atom<string | null>(null);
export const applyTargetAtom = atom<{ id: string; enabled: boolean } | null>(null);

// ---- Snapshot atoms ----

export const snapshotsAtom = atom<import("../../types").SnapshotMeta[]>([]);
export const isLoadingSnapshotsAtom = atom(false);
export const snapshotErrorAtom = atom<string | null>(null);

// ---- User preferences (persisted to localStorage) ----
// issue #123: Quick Apply is opt-in. Default OFF preserves the existing
// preview-everywhere behavior so users don't see a behavior change on first
// run after upgrade.
export const quickApplyOnToggleAtom = atomWithLocalStorage<boolean>(
  "mhost.quickApplyOnToggle",
  false,
);

// ---- Quick Apply feedback atoms (Refs #127) ----

/** The most recent `ApplyOutcome` from `enable_and_apply` or
 *  `preview_apply_outcome`. QuickApplyToast reads this to render the
 *  summary + View Diff + Rollback affordances. */
export const quickApplyOutcomeAtom = atom<ApplyOutcome | null>(null);

/** Visibility gate for QuickApplyToast. Mounted in Layout at all times
 *  but renders `null` when closed. */
export const isQuickApplyToastOpenAtom = atom(false);
