import { useCallback, useRef, useEffect } from "react";
import { createPortal } from "react-dom";
import type { ApplyPlan } from "../types";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import DiffView from "./DiffView";
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
            {/* Diff preview (shared component with QuickApplyToast) */}
            <DiffView plan={plan} />

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
