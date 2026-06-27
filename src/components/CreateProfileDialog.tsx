import { useState, useEffect, useRef, useCallback } from "react";
import { createPortal } from "react-dom";
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
  // Track whether the current pointer interaction has already fired,
  // so we don't double-fire if click also comes through.
  const firedRef = useRef(false);

  useEffect(() => {
    if (open) {
      setName("");
      setIsCreating(false);
      isCreatingRef.current = false;
      firedRef.current = false;
    }
  }, [open]);

  const handleCreate = useCallback(async () => {
    if (firedRef.current) return;
    const trimmed = name.trim();
    if (!trimmed || isCreatingRef.current) return;
    firedRef.current = true;
    isCreatingRef.current = true;
    setIsCreating(true);
    try {
      await onCreate(trimmed);
    } catch {
      // Error handled by parent
    } finally {
      isCreatingRef.current = false;
      setIsCreating(false);
      // Reset firedRef after a tick so the button is usable again
      setTimeout(() => {
        firedRef.current = false;
      }, 50);
    }
  }, [name, onCreate]);

  const handleCancel = useCallback(() => {
    if (firedRef.current || isCreatingRef.current) return;
    firedRef.current = true;
    onClose();
    setTimeout(() => {
      firedRef.current = false;
    }, 50);
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

  // WebKit workaround: use pointerdown instead of click, because WebKit
  // swallows the first click after typing when focus moves from input to button.
  const handlePointerDown = useCallback(
    (handler: () => void) => (e: React.PointerEvent) => {
      // Only handle primary button (left mouse button / single touch)
      if (e.button !== 0) return;
      handler();
    },
    [],
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
            onPointerDown={handlePointerDown(handleCreate)}
            disabled={disabled}
          >
            {isCreating ? "Creating..." : "Create"}
          </button>
          <button
            className="btn btn-ghost"
            onClick={handleCancel}
            onPointerDown={handlePointerDown(handleCancel)}
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
