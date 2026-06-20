import { useAtomValue, useSetAtom } from "jotai";
import {
  applyPlanAtom,
  isApplyingAtom,
  applyHostsActionAtom,
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
  const applyPlan = useAtomValue(applyPlanAtom);
  const isApplying = useAtomValue(isApplyingAtom);
  const applyHosts = useSetAtom(applyHostsActionAtom);

  if (!open) return null;

  const hasConflicts = applyPlan !== null && applyPlan.conflicts.length > 0;

  const handleConfirm = async () => {
    try {
      await applyHosts();
    } catch {
      // Error is handled by the store
    }
  };

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

        {/* Diff preview (show when not applying and no result yet) */}
        {!isApplying && !applyResult && applyPlan && (
          <>
            {/* Conflicts */}
            {hasConflicts && (
              <div className={styles.conflictSection}>
                {applyPlan.conflicts.map((conflict) => (
                  <div key={conflict.domain} className={styles.conflictItem}>
                    Conflict on{" "}
                    <span className={styles.conflictDomain}>
                      {conflict.domain}
                    </span>
                    : {conflict.rules.length} profiles claim this domain
                  </div>
                ))}
              </div>
            )}

            {/* Diff */}
            <div className={styles.diffPreview}>
              {applyPlan.diff.added.length === 0 &&
              applyPlan.diff.removed.length === 0 &&
              applyPlan.diff.unchanged.length === 0 ? (
                <div className={styles.diffEmpty}>No changes to preview.</div>
              ) : (
                <>
                  {applyPlan.diff.removed.map((line, i) => (
                    <div
                      key={`removed-${i}`}
                      className={`${styles.diffLine} ${styles.diffRemoved}`}
                    >
                      {line}
                    </div>
                  ))}
                  {applyPlan.diff.unchanged.map((line, i) => (
                    <div
                      key={`unchanged-${i}`}
                      className={`${styles.diffLine} ${styles.diffUnchanged}`}
                    >
                      {line}
                    </div>
                  ))}
                  {applyPlan.diff.added.map((line, i) => (
                    <div
                      key={`added-${i}`}
                      className={`${styles.diffLine} ${styles.diffAdded}`}
                    >
                      {line}
                    </div>
                  ))}
                </>
              )}
            </div>

            {/* Actions */}
            <div className={styles.dialogActions}>
              <button className="btn btn-ghost" onClick={onClose}>
                Cancel
              </button>
              <button
                className="btn btn-primary"
                onClick={handleConfirm}
                disabled={hasConflicts}
              >
                Confirm
              </button>
            </div>
          </>
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
