import { useAtomValue } from "jotai";
import {
  enabledProfileAtom,
  isApplyingAtom,
  applyPlanAtom,
} from "../stores/profiles";
import styles from "./Layout.module.css";

interface StatusBarProps {
  onApply?: () => void;
}

function StatusBar({ onApply }: StatusBarProps) {
  const enabledProfile = useAtomValue(enabledProfileAtom);
  const isApplying = useAtomValue(isApplyingAtom);
  const applyPlan = useAtomValue(applyPlanAtom);

  const hasPendingChanges =
    applyPlan !== null &&
    (applyPlan.diff.added.length > 0 || applyPlan.diff.removed.length > 0);

  return (
    <div className={styles.sidebarFooter}>
      <div className={styles.statusCard}>
        <div className={styles.statusRow}>
          <span className={styles.statusLabel}>Active</span>
          <span
            className={`${styles.statusDot} ${enabledProfile ? styles.statusDotOn : styles.statusDotOff}`}
          />
        </div>
        <div className={styles.statusProfile}>
          {enabledProfile ? enabledProfile.name : "None"}
        </div>
        {hasPendingChanges && (
          <div className={styles.statusPending}>Pending Changes</div>
        )}
        {isApplying && (
          <div className={styles.statusApplying}>Applying...</div>
        )}
        {onApply && (
          <button
            className={styles.statusApplyBtn}
            onClick={onApply}
            disabled={isApplying}
            aria-label="Apply changes"
          >
            Apply
          </button>
        )}
      </div>
    </div>
  );
}

export default StatusBar;
