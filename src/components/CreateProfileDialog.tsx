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
  // **fix issue #67 bug 1**: 不要在 handler 里再调 fire()/release()。
  // WebKit hook 的 `onPointerDown` wrapper 已经 fire() 了一次；
  // handler 自己的 fire() 会看到锁被消费 → bail → 用户必须点第二次。
  // 改为：依赖 `isCreatingRef.current` 同步守卫做双重提交保护。
  const { onPointerDown } = useWebKitPointerDown();

  useEffect(() => {
    if (open) {
      setName("");
      setIsCreating(false);
      isCreatingRef.current = false;
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
      // Error handled by parent
    } finally {
      isCreatingRef.current = false;
      setIsCreating(false);
    }
  }, [name, onCreate]);

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
