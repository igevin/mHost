import { useEffect, useState, useCallback, useRef } from "react";
import { createPortal } from "react-dom";
import { useAtomValue, useSetAtom } from "jotai";
import {
  snapshotsAtom,
  isLoadingSnapshotsAtom,
  snapshotErrorAtom,
  fetchSnapshotsAtom,
  loadSnapshotAtom,
  deleteSnapshotAtom,
} from "../stores/profiles";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import ConfirmDialog from "./ConfirmDialog";
import type { SnapshotMeta } from "../types";
import styles from "./SnapshotPanel.module.css";

interface SnapshotPanelProps {
  showCreateDialog: boolean;
  onCloseCreateDialog: () => void;
  onCreateSnapshot: (name: string, description?: string) => Promise<void>;
}

function SnapshotPanel({
  showCreateDialog,
  onCloseCreateDialog,
  onCreateSnapshot,
}: SnapshotPanelProps) {
  const snapshots = useAtomValue(snapshotsAtom);
  const isLoading = useAtomValue(isLoadingSnapshotsAtom);
  const error = useAtomValue(snapshotErrorAtom);
  const fetchSnapshots = useSetAtom(fetchSnapshotsAtom);
  const loadSnapshot = useSetAtom(loadSnapshotAtom);
  const deleteSnapshot = useSetAtom(deleteSnapshotAtom);

  const [isSaving, setIsSaving] = useState(false);
  const [isRollingBackId, setIsRollingBackId] = useState<string | null>(null);
  const [isDeletingId, setIsDeletingId] = useState<string | null>(null);

  // Pending destructive confirmation (replaces the native plugin-dialog
  // confirm() calls). The matching guard stays latched from the click until
  // the dialog resolves.
  const [pendingConfirm, setPendingConfirm] = useState<{
    kind: "rollback" | "delete";
    snapshot: SnapshotMeta;
  } | null>(null);

  const [createName, setCreateName] = useState("");
  const [createDescription, setCreateDescription] = useState("");

  const { onPointerDown } = useWebKitPointerDown();
  const saveGuard = useWebKitPointerDown();
  const rollbackGuard = useWebKitPointerDown();
  const deleteGuard = useWebKitPointerDown();

  const isSavingRef = useRef(false);

  useEffect(() => {
    fetchSnapshots();
  }, [fetchSnapshots]);

  useEffect(() => {
    if (showCreateDialog) {
      setCreateName("");
      setCreateDescription("");
      isSavingRef.current = false;
    }
  }, [showCreateDialog]);

  const handleSave = useCallback(async () => {
    if (!saveGuard.fire()) return;
    const trimmed = createName.trim();
    if (!trimmed || isSavingRef.current) {
      saveGuard.release();
      return;
    }
    isSavingRef.current = true;
    setIsSaving(true);
    try {
      await onCreateSnapshot(trimmed, createDescription.trim() || undefined);
      onCloseCreateDialog();
    } catch {
      // Error handled by store
    } finally {
      isSavingRef.current = false;
      setIsSaving(false);
      saveGuard.release();
    }
  }, [createName, createDescription, onCreateSnapshot, onCloseCreateDialog, saveGuard]);

  const handleRollback = useCallback(
    (snapshot: SnapshotMeta) => {
      if (!rollbackGuard.fire()) return;
      setPendingConfirm({ kind: "rollback", snapshot });
    },
    [rollbackGuard],
  );

  const handleDelete = useCallback(
    (snapshot: SnapshotMeta) => {
      if (!deleteGuard.fire()) return;
      setPendingConfirm({ kind: "delete", snapshot });
    },
    [deleteGuard],
  );

  const handleConfirmCancel = useCallback(() => {
    setPendingConfirm(null);
    // Whichever guard latched on the opening click is released here;
    // release() is an idempotent reset, so clearing both is safe.
    rollbackGuard.release();
    deleteGuard.release();
  }, [rollbackGuard, deleteGuard]);

  const handleConfirmAction = useCallback(async () => {
    if (!pendingConfirm) return;
    const { kind, snapshot } = pendingConfirm;
    setPendingConfirm(null);
    if (kind === "rollback") {
      setIsRollingBackId(snapshot.id);
      try {
        await loadSnapshot(snapshot.id);
      } catch {
        // Error handled by store
      } finally {
        setIsRollingBackId(null);
        rollbackGuard.release();
      }
    } else {
      setIsDeletingId(snapshot.id);
      try {
        await deleteSnapshot(snapshot.id);
      } catch {
        // Error handled by store
      } finally {
        setIsDeletingId(null);
        deleteGuard.release();
      }
    }
  }, [pendingConfirm, loadSnapshot, deleteSnapshot, rollbackGuard, deleteGuard]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        handleSave();
      } else if (e.key === "Escape") {
        e.preventDefault();
        if (!isSaving) {
          onCloseCreateDialog();
        }
      }
    },
    [handleSave, isSaving, onCloseCreateDialog],
  );

  return (
    <div className={styles.panel}>
      {error && <div className="alert alert-error">{error}</div>}

      {isLoading && snapshots.length === 0 && (
        <div className={styles.emptyState}>Loading snapshots...</div>
      )}

      {!isLoading && snapshots.length === 0 && (
        <div className={styles.emptyState}>
          <p>No snapshots yet.</p>
          <p className={styles.emptyHint}>
            Create a snapshot to save your current configuration.
          </p>
        </div>
      )}

      {snapshots.length > 0 && (
        <div className={styles.snapshotList}>
          {snapshots.map((snapshot) => (
            <div key={snapshot.id} className={styles.snapshotCard}>
              <div className={styles.snapshotCardHeader}>
                <h3 className={styles.snapshotName}>{snapshot.name}</h3>
                <span className={styles.snapshotBadge}>
                  {snapshot.profile_count} profile
                  {snapshot.profile_count !== 1 ? "s" : ""}
                </span>
              </div>
              {snapshot.description && (
                <p className={styles.snapshotDesc}>{snapshot.description}</p>
              )}
              <div className={styles.snapshotMeta}>
                <span>{formatDate(snapshot.created_at)}</span>
              </div>
              <div className={styles.snapshotActions}>
                {/* These handlers own their guards (saveGuard / rollbackGuard /
                 * deleteGuard), so per the useWebKitPointerDown contract they
                 * are NOT wrapped in `onPointerDown(...)` — both events feed
                 * the handler directly with an inline `button !== 0` filter. */}
                <button
                  className="btn btn-primary btn-sm"
                  onClick={() => handleRollback(snapshot)}
                  onPointerDown={(e) => {
                    if (e.button !== 0) return;
                    handleRollback(snapshot);
                  }}
                  disabled={isRollingBackId === snapshot.id}
                >
                  {isRollingBackId === snapshot.id
                    ? "Rolling back..."
                    : "Rollback"}
                </button>
                <button
                  className="btn btn-danger btn-sm"
                  onClick={() => handleDelete(snapshot)}
                  onPointerDown={(e) => {
                    if (e.button !== 0) return;
                    handleDelete(snapshot);
                  }}
                  disabled={isDeletingId === snapshot.id}
                >
                  {isDeletingId === snapshot.id ? "Deleting..." : "Delete"}
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      {showCreateDialog &&
        createPortal(
          <div
            className={styles.dialogOverlay}
            onClick={isSaving ? undefined : onCloseCreateDialog}
          >
            <div
              className={styles.dialog}
              onClick={(e) => e.stopPropagation()}
            >
              <h3 className={styles.dialogTitle}>Create Snapshot</h3>
              <div className={styles.dialogBody}>
                <div className="form-group">
                  <label className="form-label">Name</label>
                  <input
                    className="input"
                    placeholder="Snapshot name"
                    value={createName}
                    onChange={(e) => setCreateName(e.target.value)}
                    onKeyDown={handleKeyDown}
                    autoFocus
                  />
                </div>
                <div className="form-group">
                  <label className="form-label">Description (optional)</label>
                  <textarea
                    className="input"
                    placeholder="Optional description"
                    value={createDescription}
                    onChange={(e) => setCreateDescription(e.target.value)}
                    rows={3}
                  />
                </div>
              </div>
              <div className={styles.dialogActions}>
                <button
                  className="btn btn-primary"
                  onClick={handleSave}
                  onPointerDown={(e) => {
                    if (e.button !== 0) return;
                    handleSave();
                  }}
                  disabled={!createName.trim() || isSaving}
                >
                  {isSaving ? "Saving..." : "Save"}
                </button>
                <button
                  className="btn btn-ghost"
                  onClick={onCloseCreateDialog}
                  onPointerDown={onPointerDown(onCloseCreateDialog)}
                  disabled={isSaving}
                >
                  Cancel
                </button>
              </div>
            </div>
          </div>,
          document.body,
        )}

      <ConfirmDialog
        open={pendingConfirm !== null}
        title={
          pendingConfirm?.kind === "rollback"
            ? "Rollback to Snapshot"
            : "Delete Snapshot"
        }
        message={
          pendingConfirm?.kind === "rollback"
            ? `Rollback to snapshot "${pendingConfirm.snapshot.name}"? Current configuration will be overwritten.`
            : `Delete snapshot "${pendingConfirm?.snapshot.name}"?`
        }
        confirmLabel={
          pendingConfirm?.kind === "rollback" ? "Rollback" : "Delete"
        }
        danger
        onConfirm={handleConfirmAction}
        onCancel={handleConfirmCancel}
      />
    </div>
  );
}

/* ---- Helpers ---- */

function formatDate(isoDate: string): string {
  try {
    const date = new Date(isoDate);
    if (isNaN(date.getTime())) return isoDate;
    return date.toLocaleString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return isoDate;
  }
}

export default SnapshotPanel;
