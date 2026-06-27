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
  const inputRef = useRef<HTMLInputElement>(null);

  // Reset name and focus input when dialog opens
  useEffect(() => {
    if (open) {
      setName("");
      setIsCreating(false);
      isCreatingRef.current = false;
      // Focus after a tick to ensure the input is mounted
      const timer = setTimeout(() => {
        inputRef.current?.focus();
      }, 0);
      return () => clearTimeout(timer);
    }
  }, [open]);

  const handleCreate = useCallback(async () => {
    const trimmed = name.trim();
    if (!trimmed || isCreatingRef.current) return;
    isCreatingRef.current = true;
    setIsCreating(true);
    try {
      await onCreate(trimmed);
    } catch {
      // Error is handled by parent via setError
    } finally {
      isCreatingRef.current = false;
      setIsCreating(false);
    }
  }, [name, onCreate]);

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
            ref={inputRef}
            className="input"
            placeholder="Profile name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={handleKeyDown}
          />
          <button
            className="btn btn-primary"
            onClick={handleCreate}
            disabled={disabled}
          >
            {isCreating ? "Creating..." : "Create"}
          </button>
          <button className="btn btn-ghost" onClick={onClose} disabled={isCreating}>
            Cancel
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

export default CreateProfileDialog;
