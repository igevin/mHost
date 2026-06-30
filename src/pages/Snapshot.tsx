import { useState, useCallback } from "react";
import { useSetAtom } from "jotai";
import { saveSnapshotAtom } from "../stores/profiles";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import SnapshotPanel from "../components/SnapshotPanel";

function SnapshotPage() {
  const [showCreateDialog, setShowCreateDialog] = useState(false);
  const saveSnapshot = useSetAtom(saveSnapshotAtom);
  const { onPointerDown } = useWebKitPointerDown();

  const handleCreateSnapshot = useCallback(
    async (name: string, description?: string) => {
      await saveSnapshot({ name, description });
    },
    [saveSnapshot],
  );

  return (
    <div className="mhost-page">
      <header className="mhost-page-header">
        <h1 className="mhost-page-title">Snapshots</h1>
        <div className="mhost-page-actions">
          <button
            className="btn btn-primary"
            onClick={() => setShowCreateDialog(true)}
            onPointerDown={onPointerDown(() => setShowCreateDialog(true))}
          >
            Create Snapshot
          </button>
        </div>
      </header>

      <SnapshotPanel
        showCreateDialog={showCreateDialog}
        onCloseCreateDialog={() => setShowCreateDialog(false)}
        onCreateSnapshot={handleCreateSnapshot}
      />
    </div>
  );
}

export default SnapshotPage;
