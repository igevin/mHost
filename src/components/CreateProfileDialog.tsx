import { useState, useEffect, useRef, useCallback } from "react";
import { createPortal } from "react-dom";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import styles from "./CreateProfileDialog.module.css";

interface CreateProfileDialogProps {
  open: boolean;
  onClose: () => void;
  onCreate: (name: string) => Promise<void>;
  isLoading: boolean;
}

function CreateProfileDialog({ open, onClose, onCreate, isLoading }: CreateProfileDialogProps) {
  const [name, setName] = useState("");
  const [isCreating, setIsCreating] = useState(false);
  const isCreatingRef = useRef(false);
  const { fire, release, onPointerDown } = useWebKitPointerDown();

  useEffect(() => {
    if (open) {
      setName("");
      setIsCreating(false);
      isCreatingRef.current = false;
    }
  }, [open]);

  const handleCreate = useCallback(async () => {
    if (!fire()) return;
    const trimmed = name.trim();
    if (!trimmed || isCreatingRef.current) {
      release();
      return;
    }
    isCreatingRef.current = true;
    setIsCreating(true);
    try {
      await onCreate(trimmed);
    } catch {
      // Error handled by parent
    } finally {
      isCreatingRef.current = false;
      setIsCreating(false);
      release();
    }
  }, [name, onCreate, fire, release]);

  const handleCancel = useCallback(() => {
    if (isCreatingRef.current) return;
    onClose();
  }, [onClose]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        handleCreate();
      }
    },
    [handleCreate],
  );

  if (!open) return null;

  const disabled = !name.trim() || isLoading || isCreating;

  return createPortal(
    <div className={styles.overlay} onClick={onClose}>
      <div className={styles.dialog} onClick={(e) => e.stopPropagation()}>
        <h3 className={styles.title}>Create Profile</h3>
        <div className="form-row">
          <input
            className="input"
            placeholder="Profile name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={handleKeyDown}
          />
          <button
            className="btn btn-primary"
            onClick={handleCreate}
            onPointerDown={onPointerDown(handleCreate)}
            disabled={disabled}
          >
            {isCreating ? "Creating..." : "Create"}
          </button>
          <button
            className="btn btn-ghost"
            onClick={handleCancel}
            onPointerDown={onPointerDown(handleCancel)}
            disabled={isCreating}
          >
            Cancel
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

export default CreateProfileDialog;
