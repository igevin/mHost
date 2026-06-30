import { useState } from "react";
import { extractErrorMessage } from "../lib/error";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import styles from "./RollbackButton.module.css";

interface RollbackButtonProps {
  onRollback: () => Promise<void>;
  size?: "default" | "small";
}

function RollbackButton({ onRollback, size = "default" }: RollbackButtonProps) {
  const [showConfirm, setShowConfirm] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isRollingBack, setIsRollingBack] = useState(false);
  const { onPointerDown } = useWebKitPointerDown();
  const dialogGuard = useWebKitPointerDown();

  const handleRollbackClick = () => {
    if (!dialogGuard.fire()) return;
    setShowConfirm(true);
    setError(null);
    setTimeout(dialogGuard.release, 50);
  };

  const handleConfirm = async () => {
    setIsRollingBack(true);
    setError(null);
    try {
      await onRollback();
      setShowConfirm(false);
    } catch (err) {
      setError(extractErrorMessage(err));
    } finally {
      setIsRollingBack(false);
    }
  };

  const handleCancel = () => {
    if (!dialogGuard.fire()) return;
    setShowConfirm(false);
    setError(null);
    setTimeout(dialogGuard.release, 50);
  };

  const btnClass = size === "small" ? "btn btn-sm btn-danger" : "btn btn-danger";

  return (
    <>
      <button
        className={btnClass}
        onClick={handleRollbackClick}
        onPointerDown={onPointerDown(handleRollbackClick)}
        disabled={isRollingBack}
      >
        Rollback
      </button>

      {showConfirm && (
        <div className={styles.overlay} role="dialog" aria-modal="true">
          <div className={styles.dialog}>
            <h3 className={styles.dialogTitle}>Confirm Rollback</h3>
            <p className={styles.dialogMessage}>
              Are you sure you want to rollback the hosts file to the previous
              backup? This will replace the current hosts file.
            </p>

            {error && (
              <div className="alert alert-error">{error}</div>
            )}

            <div className={styles.dialogActions}>
              <button
                className="btn btn-ghost"
                onClick={handleCancel}
                onPointerDown={onPointerDown(handleCancel)}
                disabled={isRollingBack}
              >
                Cancel
              </button>
              <button
                className="btn btn-danger"
                onClick={handleConfirm}
                onPointerDown={onPointerDown(handleConfirm)}
                disabled={isRollingBack}
              >
                {isRollingBack ? "Rolling back..." : "Confirm"}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}

export default RollbackButton;
