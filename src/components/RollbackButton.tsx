import { useState } from "react";
import styles from "./RollbackButton.module.css";

interface RollbackButtonProps {
  onRollback: () => Promise<void>;
}

function RollbackButton({ onRollback }: RollbackButtonProps) {
  const [showConfirm, setShowConfirm] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isRollingBack, setIsRollingBack] = useState(false);

  const handleRollbackClick = () => {
    setShowConfirm(true);
    setError(null);
  };

  const handleConfirm = async () => {
    setIsRollingBack(true);
    setError(null);
    try {
      await onRollback();
      setShowConfirm(false);
    } catch (err) {
      setError(
        err instanceof Error ? err.message : String(err),
      );
    } finally {
      setIsRollingBack(false);
    }
  };

  const handleCancel = () => {
    setShowConfirm(false);
    setError(null);
  };

  return (
    <>
      <button
        className="btn btn-danger"
        onClick={handleRollbackClick}
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
                disabled={isRollingBack}
              >
                Cancel
              </button>
              <button
                className="btn btn-danger"
                onClick={handleConfirm}
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
