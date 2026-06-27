import { useState, useEffect } from "react";
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
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    setMounted(true);
  }, []);

  useEffect(() => {
    if (open) {
      setName("");
    }
  }, [open]);

  const handleCreate = async () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    await onCreate(trimmed);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") handleCreate();
  };

  if (!open || !mounted) return null;

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
            autoFocus
          />
          <button
            className="btn btn-primary"
            onClick={handleCreate}
            disabled={!name.trim() || isLoading}
          >
            Create
          </button>
          <button className="btn btn-ghost" onClick={onClose}>
            Cancel
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

export default CreateProfileDialog;
