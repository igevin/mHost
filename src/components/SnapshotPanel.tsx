import { useEffect, useState, useCallback, useRef } from "react";
import { createPortal } from "react-dom";
import { useAtomValue, useSetAtom } from "jotai";
import { confirm } from "@tauri-apps/plugin-dialog";
import {
  snapshotsAtom,
  isLoadingSnapshotsAtom,
  snapshotErrorAtom,
  fetchSnapshotsAtom,
  saveSnapshotAtom,
  loadSnapshotAtom,
  deleteSnapshotAtom,
} from "../stores/profiles";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import { extractErrorMessage } from "../lib/error";
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
    async (snapshot: SnapshotMeta) => {
      if (!rollbackGuard.fire()) return;
      const confirmed = await confirm(
        `Rollback to snapshot "${snapshot.name}"? Current configuration will be overwritten.`,
      );
      if (!confirmed) {
        rollbackGuard.release();
        return;
      }
      setIsRollingBackId(snapshot.id);
      try {
        await loadSnapshot(snapshot.id);
      } catch {
        // Error handled by store
      } finally {
        setIsRollingBackId(null);
        rollbackGuard.release();
      }
    },
    [loadSnapshot, rollbackGuard],
  );

  const handleDelete = useCallback(
    async (snapshot: SnapshotMeta) => {
      if (!deleteGuard.fire()) return;
      const confirmed = await confirm(`Delete snapshot "${snapshot.name}"?`);
      if (!confirmed) {
        deleteGuard.release();
        return;
      }
      setIsDeletingId(snapshot.id);
      try {
        await deleteSnapshot(snapshot.id);
      } catch {
        // Error handled by store
      } finally {
        setIsDeletingId(null);
        deleteGuard.release();
      }
    },
    [deleteSnapshot, deleteGuard],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        handleSave();
      }
    },
    [handleSave],
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
                <button
                  className="btn btn-primary btn-sm"
                  onClick={() => handleRollback(snapshot)}
                  onPointerDown={onPointerDown(() =>
                    handleRollback(snapshot),
                  )}
                  disabled={isRollingBackId === snapshot.id}
                >
                  {isRollingBackId === snapshot.id
                    ? "Rolling back..."
                    : "Rollback"}
                </button>
                <button
                  className="btn btn-danger btn-sm"
                  onClick={() => handleDelete(snapshot)}
                  onPointerDown={onPointerDown(() =>
                    handleDelete(snapshot),
                  )}
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
            onClick={onCloseCreateDialog}
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
                  onPointerDown={onPointerDown(handleSave)}
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
