import { useState } from "react";
import { extractErrorMessage } from "../lib/error";
import ConfirmDialog from "./ConfirmDialog";

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
      setError(extractErrorMessage(err));
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

      <ConfirmDialog
        open={showConfirm}
        title="Confirm Rollback"
        message="Are you sure you want to rollback the hosts file to the previous backup? This will replace the current hosts file."
        confirmLabel="Confirm"
        loadingLabel="Rolling back..."
        danger
        isLoading={isRollingBack}
        onConfirm={handleConfirm}
        onCancel={handleCancel}
      >
        {error && <div className="alert alert-error">{error}</div>}
      </ConfirmDialog>
    </>
  );
}

export default RollbackButton;
