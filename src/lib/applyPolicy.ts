import type { ApplyMode, ApplyOutcome } from "../types";

/** Mirrors Rust `DESTRUCTIVE_THRESHOLD` in `commands/apply.rs`.
 *  If you change this, change it there too. */
export const DESTRUCTIVE_THRESHOLD = 100;

/**
 * Pure client-side mirror of Rust `decide_apply_mode`. Keeps an extra
 * IPC round-trip off the Quick Apply path (the preview already returned
 * the full ApplyOutcome; we just classify it here).
 *
 * Rules (first-match wins) MUST match `commands/apply.rs::decide_apply_mode`
 * exactly. The Rust version stays unit-tested as the canonical authority;
 * this mirror is tested with the same table in `applyPolicy.test.ts`.
 */
export function decideApplyMode(outcome: ApplyOutcome): ApplyMode {
  if (outcome.has_conflicts) return "require_preview";
  if (outcome.disabled_profile_ids.length > 0) return "require_preview";
  if (outcome.added_count + outcome.removed_count > DESTRUCTIVE_THRESHOLD)
    return "require_preview";
  return "quick_apply";
}