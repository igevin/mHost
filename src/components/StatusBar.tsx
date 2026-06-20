import { useAtomValue } from "jotai";
import { enabledProfileAtom, isApplyingAtom } from "../stores/profiles";
import styles from "./Layout.module.css";

function StatusBar() {
  const enabledProfile = useAtomValue(enabledProfileAtom);
  const isApplying = useAtomValue(isApplyingAtom);

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
        {isApplying && (
          <div className={styles.statusApplying}>Applying...</div>
        )}
      </div>
    </div>
  );
}

export default StatusBar;
