import { useEffect, useState } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import {
  quickApplyOutcomeAtom,
  isQuickApplyToastOpenAtom,
  rollbackHostsActionAtom,
  applyErrorAtom,
} from "../stores/profiles";
import type { ApplyOutcome, Profile } from "../types";
import DiffView from "./DiffView";
import RollbackButton from "./RollbackButton";
import styles from "./QuickApplyToast.module.css";

const AUTO_DISMISS_MS = 5000;

interface QuickApplyToastProps {
  /** Used to resolve profile IDs to display names. */
  profiles: ReadonlyArray<Pick<Profile, "id" | "name">>;
}

/**
 * Renders a top-right toast after a successful Quick Apply hosts toggle.
 * Shows a structured summary of the change, an inline (expandable) diff via
 * the shared `<DiffView>`, and Rollback (filesystem backup via
 * `rollbackHostsActionAtom`). Auto-dismisses after 5 seconds — the timer is
 * paused while the diff is expanded so the user can read it.
 *
 * Refs #127. The diff is rendered read-only inside the toast rather than
 * reopening the ApplyConfirmDialog (the apply already happened, so a
 * confirm dialog with a live "Apply" button would be misleading).
 */
function QuickApplyToast({ profiles }: QuickApplyToastProps) {
  const outcome = useAtomValue(quickApplyOutcomeAtom);
  const isOpen = useAtomValue(isQuickApplyToastOpenAtom);
  const setIsOpen = useSetAtom(isQuickApplyToastOpenAtom);
  const rollbackHostsAction = useSetAtom(rollbackHostsActionAtom);
  const setApplyError = useSetAtom(applyErrorAtom);
  const [showDiff, setShowDiff] = useState(false);

  // Reset the expanded diff whenever a new outcome arrives.
  useEffect(() => {
    setShowDiff(false);
  }, [outcome]);

  // Auto-dismiss after a delay, but not while the diff is expanded (the user
  // is actively reading it).
  useEffect(() => {
    if (!isOpen || showDiff) return;
    const t = setTimeout(() => setIsOpen(false), AUTO_DISMISS_MS);
    return () => clearTimeout(t);
  }, [isOpen, outcome, showDiff, setIsOpen]);

  if (!isOpen || !outcome) return null;

  const disabledNames = outcome.disabled_profile_ids.map(
    (id) => profiles.find((p) => p.id === id)?.name ?? id,
  );

  const summary = buildSummary(outcome, disabledNames);
  const diffEmpty =
    outcome.plan.diff.added.length === 0 &&
    outcome.plan.diff.removed.length === 0;

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
      {outcome.snapshot_id && <div className={styles.meta}>Snapshot saved</div>}
      {showDiff && <DiffView plan={outcome.plan} compact />}
      <div className={styles.actions}>
        {!diffEmpty && (
          <button
            className="btn btn-ghost"
            onClick={() => setShowDiff((v) => !v)}
            aria-expanded={showDiff}
          >
            {showDiff ? "Hide Diff" : "View Diff"}
          </button>
        )}
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
