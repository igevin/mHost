import { useAtomValue } from "jotai";
import {
  isApplyingAtom,
} from "../stores/profiles";
import styles from "./ApplyConfirmDialog.module.css";

interface ApplyConfirmDialogProps {
  open: boolean;
  onClose: () => void;
  applyResult?: "success" | "error" | null;
  applyError?: string | null;
  onRollback?: () => void;
}

function ApplyConfirmDialog({
  open,
  onClose,
  applyResult,
  applyError,
  onRollback,
}: ApplyConfirmDialogProps) {
  const isApplying = useAtomValue(isApplyingAtom);

  if (!open) return null;

  return (
    <div className={styles.overlay} role="dialog" aria-modal="true">
      <div className={styles.dialog}>
        <h2 className={styles.dialogTitle}>Apply Preview</h2>

        {/* Applying progress */}
        {isApplying && (
          <div className={styles.progressSection}>
            <div className={styles.progressText}>Applying changes...</div>
          </div>
        )}

        {/* Success result */}
        {applyResult === "success" && !isApplying && (
          <div className={styles.resultSection}>
            <div className={styles.resultSuccess}>
              Hosts file updated successfully.
            </div>
          </div>
        )}

        {/* Error result */}
        {applyResult === "error" && !isApplying && (
          <div className={styles.resultSection}>
            <div className={styles.resultError}>Apply failed</div>
            {applyError && (
              <div className={styles.resultErrorMessage}>{applyError}</div>
            )}
            {onRollback && (
              <button className="btn btn-danger" onClick={onRollback}>
                Rollback
              </button>
            )}
          </div>
        )}

        {/* Close button for success/error states */}
        {(applyResult === "success" || applyResult === "error") && (
          <div className={styles.dialogActions}>
            <button className="btn btn-ghost" onClick={onClose}>
              Close
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

export default ApplyConfirmDialog;
