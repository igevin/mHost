import { useState, useCallback, useRef, useEffect } from "react";
import { createPortal } from "react-dom";
import type { ApplyPlan } from "../types";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import styles from "./ApplyConfirmDialog.module.css";

interface ApplyConfirmDialogProps {
  open: boolean;
  plan: ApplyPlan | null;
  onConfirm: () => void;
  onCancel: () => void;
  isApplying: boolean;
  applyResult?: "success" | "error" | null;
  applyError?: string | null;
  onRollback?: () => void;
}

function ApplyConfirmDialog({
  open,
  plan,
  onConfirm,
  onCancel,
  isApplying,
  applyResult,
  applyError,
  onRollback,
}: ApplyConfirmDialogProps) {
  const { onPointerDown } = useWebKitPointerDown();
  const [showUnchanged, setShowUnchanged] = useState(false);
  const confirmedRef = useRef(false);
  const cancelledRef = useRef(false);
  const rolledBackRef = useRef(false);

  useEffect(() => {
    if (open) {
      confirmedRef.current = false;
      cancelledRef.current = false;
      rolledBackRef.current = false;
    }
  }, [open, plan, isApplying, applyResult]);

  const handleConfirm = useCallback(() => {
    if (confirmedRef.current) return;
    confirmedRef.current = true;
    onConfirm();
  }, [onConfirm]);

  const handleCancel = useCallback(() => {
    if (cancelledRef.current) return;
    cancelledRef.current = true;
    if (isApplying) return;
    onCancel();
  }, [isApplying, onCancel]);

  const handleRollback = useCallback(() => {
    if (rolledBackRef.current) return;
    rolledBackRef.current = true;
    onRollback?.();
  }, [onRollback]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Escape" && !isApplying) {
        onCancel();
      }
    },
    [isApplying, onCancel],
  );

  if (!open) return null;

  const isPreview = !isApplying && !applyResult && !!plan;
  const hasConflicts = plan && plan.conflicts.length > 0;
  const diffEmpty = plan && plan.diff.added.length === 0 && plan.diff.removed.length === 0;

  return createPortal(
    <div
      className={styles.overlay}
      onClick={handleCancel}
      onKeyDown={handleKeyDown}
      role="dialog"
      aria-modal="true"
    >
      <div className={styles.dialog} onClick={(e) => e.stopPropagation()}>
        <h2 className={styles.dialogTitle}>
          {isPreview ? "Confirm Changes" : "Apply Preview"}
        </h2>

        {/* Preview state */}
        {isPreview && (
          <>
            {/* Diff preview */}
            <div className={styles.diffSection}>
              {diffEmpty ? (
                <div className={styles.diffEmpty}>No changes detected</div>
              ) : (
                <div className={styles.diffPreview}>
                  {plan.diff.added.map((line, i) => (
                    <div key={`+${i}`} className={`${styles.diffLine} ${styles.diffAdded}`}>
                      + {line}
                    </div>
                  ))}
                  {plan.diff.removed.map((line, i) => (
                    <div key={`-${i}`} className={`${styles.diffLine} ${styles.diffRemoved}`}>
                      - {line}
                    </div>
                  ))}
                  {plan.diff.unchanged.length > 0 && (
                    <>
                      {!showUnchanged ? (
                        <button
                          className={styles.diffUnchangedCollapsed}
                          onClick={() => setShowUnchanged(true)}
                        >
                          ...{plan.diff.unchanged.length} unchanged lines...
                        </button>
                      ) : (
                        <>
                          {plan.diff.unchanged.map((line, i) => (
                            <div
                              key={`u${i}`}
                              className={`${styles.diffLine} ${styles.diffUnchanged}`}
                            >
                              {`  ${line}`}
                            </div>
                          ))}
                          <button
                            className={styles.diffUnchangedCollapsed}
                            onClick={() => setShowUnchanged(false)}
                          >
                            Collapse unchanged lines
                          </button>
                        </>
                      )}
                    </>
                  )}
                </div>
              )}
            </div>

            {/* Conflicts */}
            {hasConflicts && (
              <div className={styles.conflictSection}>
                <div className={styles.conflictWarning}>
                  Warning: {plan.conflicts.length} conflict(s) detected
                </div>
                <div className={styles.conflictList}>
                  {plan.conflicts.map((conflict, i) => (
                    <div key={i} className={styles.conflictItem}>
                      <span className={styles.conflictDomain}>{conflict.domain}</span>
                      <span>
                        — {conflict.rules.map((r) => r.source_profile_name).join(", ")}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Backup notice */}
            {plan.backup_required && (
              <div className={styles.backupNotice}>
                A backup will be created before applying.
              </div>
            )}

            {/* Actions */}
            <div className={styles.dialogActions}>
              <button
                className="btn btn-primary"
                onClick={handleConfirm}
                onPointerDown={onPointerDown(handleConfirm)}
                disabled={isApplying || !!hasConflicts}
              >
                {isApplying ? "Applying..." : "Confirm Apply"}
              </button>
              <button
                className="btn btn-ghost"
                onClick={handleCancel}
                onPointerDown={onPointerDown(handleCancel)}
                disabled={isApplying}
              >
                Cancel
              </button>
            </div>
          </>
        )}

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
            <div className={styles.dialogActions}>
              <button
                className="btn btn-ghost"
                onClick={handleCancel}
                onPointerDown={onPointerDown(handleCancel)}
              >
                Close
              </button>
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
            <div className={styles.dialogActions}>
              {onRollback && (
                <button
                  className="btn btn-danger"
                  onClick={handleRollback}
                  onPointerDown={onPointerDown(handleRollback)}
                >
                  Rollback
                </button>
              )}
              <button
                className="btn btn-ghost"
                onClick={handleCancel}
                onPointerDown={onPointerDown(handleCancel)}
              >
                Close
              </button>
            </div>
          </div>
        )}
      </div>
    </div>,
    document.body,
  );
}

export default ApplyConfirmDialog;
