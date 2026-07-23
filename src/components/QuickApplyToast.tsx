import { useEffect } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import {
  quickApplyOutcomeAtom,
  isQuickApplyToastOpenAtom,
  applyPlanAtom,
  applyTargetAtom,
  applyConfirmOpenAtom,
  rollbackHostsActionAtom,
  applyErrorAtom,
} from "../stores/profiles";
import type { ApplyOutcome, Profile } from "../types";
import RollbackButton from "./RollbackButton";
import styles from "./QuickApplyToast.module.css";

const AUTO_DISMISS_MS = 5000;

interface QuickApplyToastProps {
  /** Used to resolve profile IDs to display names. */
  profiles: ReadonlyArray<Pick<Profile, "id" | "name">>;
}

/**
 * Renders a top-right toast after a successful Quick Apply hosts toggle.
 * Shows a structured summary of the change, View Diff button (opens the
 * ApplyConfirmDialog in read-only mode), and Rollback (filesystem backup
 * via `rollbackHostsActionAtom`). Auto-dismisses after 5 seconds.
 *
 * Refs #127.
 */
function QuickApplyToast({ profiles }: QuickApplyToastProps) {
  const outcome = useAtomValue(quickApplyOutcomeAtom);
  const isOpen = useAtomValue(isQuickApplyToastOpenAtom);
  const setIsOpen = useSetAtom(isQuickApplyToastOpenAtom);
  const setApplyPlan = useSetAtom(applyPlanAtom);
  const setApplyTarget = useSetAtom(applyTargetAtom);
  const setApplyConfirmOpen = useSetAtom(applyConfirmOpenAtom);
  const rollbackHostsAction = useSetAtom(rollbackHostsActionAtom);
  const setApplyError = useSetAtom(applyErrorAtom);

  // Reset the dismiss timer on each new outcome (snapshot_id is a stable
  // identity for "this is a new apply"). Fall back to plan reference for
  // cases where snapshot_id is None (DNS-mode apply).
  useEffect(() => {
    if (!isOpen) return;
    const t = setTimeout(() => setIsOpen(false), AUTO_DISMISS_MS);
    return () => clearTimeout(t);
  }, [isOpen, outcome?.snapshot_id, outcome, setIsOpen]);

  if (!isOpen || !outcome) return null;

  const disabledNames = outcome.disabled_profile_ids.map(
    (id) => profiles.find((p) => p.id === id)?.name ?? id,
  );

  const summary = buildSummary(outcome, disabledNames);

  const handleViewDiff = () => {
    // Open the existing ApplyConfirmDialog with the plan. Leave
    // applyTargetAtom = null so executeApplyAtom bails early (existing
    // behavior at actions.ts:163) — the apply already happened, we
    // just want the diff for context.
    setApplyPlan(outcome.plan);
    setApplyTarget(null);
    setApplyConfirmOpen(true);
    setIsOpen(false);
  };

  const handleRollback = async () => {
    try {
      await rollbackHostsAction();
      setIsOpen(false);
    } catch (err) {
      setApplyError("Rollback failed: " + String(err));
    }
  };

  return (
    <div className={styles.toast} role="status" aria-live="polite">
      <div className={styles.header}>
        <strong className={styles.summary}>{summary}</strong>
        <button
          className={styles.closeBtn}
          onClick={() => setIsOpen(false)}
          aria-label="Dismiss"
        >
          ×
        </button>
      </div>
      {outcome.snapshot_id && (
        <div className={styles.meta}>Snapshot saved</div>
      )}
      <div className={styles.actions}>
        <button className="btn btn-ghost" onClick={handleViewDiff}>
          View Diff
        </button>
        <RollbackButton onRollback={handleRollback} />
      </div>
    </div>
  );
}

function buildSummary(outcome: ApplyOutcome, disabledNames: string[]): string {
  const parts: string[] = [];
  if (outcome.added_count > 0) parts.push(`${outcome.added_count} added`);
  if (outcome.removed_count > 0) parts.push(`${outcome.removed_count} removed`);
  if (parts.length === 0) parts.push("no rule changes");
  let msg = `Applied: ${parts.join(", ")}`;
  if (disabledNames.length > 0) {
    msg += `. Disabled: ${disabledNames.join(", ")}`;
  }
  return msg;
}

export default QuickApplyToast;