import { useEffect, useState, useCallback } from "react";
import { listBackups, rollbackToBackup } from "../lib/tauri";
import { extractErrorMessage } from "../lib/error";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import type { BackupInfo } from "../types";
import styles from "./BackupPanel.module.css";

interface BackupPanelProps {
  onRollback?: () => void;
}

function formatSize(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

function BackupPanel({ onRollback }: BackupPanelProps) {
  const [backups, setBackups] = useState<BackupInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [confirmBackup, setConfirmBackup] = useState<BackupInfo | null>(null);
  const [isRollingBack, setIsRollingBack] = useState(false);
  const { onPointerDown } = useWebKitPointerDown();
  const dialogGuard = useWebKitPointerDown();

  const fetchBackups = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await listBackups();
      setBackups(data);
    } catch (err) {
      setError(extractErrorMessage(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchBackups();
  }, [fetchBackups]);

  const handleRollbackClick = (backup: BackupInfo) => {
    if (!dialogGuard.fire()) return;
    setConfirmBackup(backup);
    setError(null);
    setTimeout(dialogGuard.release, 50);
  };

  const handleConfirm = async () => {
    if (!confirmBackup) return;
    setIsRollingBack(true);
    setError(null);
    try {
      await rollbackToBackup(confirmBackup.id);
      setConfirmBackup(null);
      onRollback?.();
      await fetchBackups();
    } catch (err) {
      setError(extractErrorMessage(err));
    } finally {
      setIsRollingBack(false);
    }
  };

  const handleCancel = () => {
    if (!dialogGuard.fire()) return;
    setConfirmBackup(null);
    setError(null);
    setTimeout(dialogGuard.release, 50);
  };

  if (loading) {
    return <div className={styles.loading}>Loading backups...</div>;
  }

  if (backups.length === 0) {
    return <div className={styles.emptyState}>No backups yet</div>;
  }

  return (
    <div className={styles.backupPanel}>
      {error && !confirmBackup && <div className="alert alert-error">{error}</div>}

      <table className={styles.backupTable}>
        <thead>
          <tr>
            <th>Timestamp</th>
            <th>Filename</th>
            <th>Size</th>
            <th className={styles.actionsCol}>Actions</th>
          </tr>
        </thead>
        <tbody>
          {backups.map((backup) => (
            <tr key={backup.id}>
              <td>{new Date(backup.timestamp).toLocaleString()}</td>
              <td className={styles.filenameCell}>{backup.filename}</td>
              <td>{formatSize(backup.size)}</td>
              <td className={styles.actionsCol}>
                <button
                  className="btn btn-sm btn-danger"
                  onClick={() => handleRollbackClick(backup)}
                  onPointerDown={onPointerDown(() => handleRollbackClick(backup))}
                  disabled={isRollingBack}
                >
                  Rollback
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      {confirmBackup && (
        <div className={styles.overlay} role="dialog" aria-modal="true">
          <div className={styles.dialog}>
            <h3 className={styles.dialogTitle}>Confirm Rollback</h3>
            <p className={styles.dialogMessage}>
              Rollback to backup from{" "}
              {new Date(confirmBackup.timestamp).toLocaleString()}? This will
              replace your current hosts file.
            </p>

            {error && <div className="alert alert-error">{error}</div>}

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
    </div>
  );
}

export default BackupPanel;
