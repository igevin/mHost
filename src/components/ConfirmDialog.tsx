import { useCallback, useEffect } from "react";
import { createPortal } from "react-dom";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import styles from "./ConfirmDialog.module.css";

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** danger styles the confirm button red — use for destructive actions. */
  danger?: boolean;
  /** While busy, Esc / overlay-click / Cancel are all inert. */
  isLoading?: boolean;
  loadingLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
  /** Optional extra content between the message and the action row
   * (e.g. RollbackButton renders its error alert here). */
  children?: React.ReactNode;
}

/**
 * Generic confirmation modal — the single replacement for the native
 * `@tauri-apps/plugin-dialog` confirm() calls and the one-off inline
 * modals (RollbackButton). Portal-rendered; Esc and overlay-click cancel.
 */
function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  danger = false,
  isLoading = false,
  loadingLabel,
  onConfirm,
  onCancel,
  children,
}: ConfirmDialogProps) {
  // Both buttons bind onPointerDown AND onClick, so each handler owns its
  // own fire()+releaseSoon() guard (the `useWebKitPointerDown` pattern for
  // dual-bound handlers — the onPointerDown *wrapper* would double-consume
  // the guard). Without this, one physical click invokes onConfirm twice.
  const confirmGuard = useWebKitPointerDown();
  const cancelGuard = useWebKitPointerDown();

  useEffect(() => {
    if (!open) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !isLoading) {
        e.preventDefault();
        onCancel();
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [open, isLoading, onCancel]);

  const handleConfirm = useCallback(() => {
    if (!confirmGuard.fire()) return;
    confirmGuard.releaseSoon();
    onConfirm();
  }, [confirmGuard, onConfirm]);

  const handleCancel = useCallback(() => {
    if (isLoading) return;
    if (!cancelGuard.fire()) return;
    cancelGuard.releaseSoon();
    onCancel();
  }, [cancelGuard, isLoading, onCancel]);

  if (!open) return null;

  return createPortal(
    <div
      className={styles.overlay}
      onClick={() => {
        if (!isLoading) onCancel();
      }}
      role="dialog"
      aria-modal="true"
      aria-label={title}
    >
      <div className={styles.dialog} onClick={(e) => e.stopPropagation()}>
        <h3 className={styles.dialogTitle}>{title}</h3>
        <p className={styles.dialogMessage}>{message}</p>
        {children}
        <div className={styles.dialogActions}>
          <button
            className="btn btn-ghost"
            onClick={handleCancel}
            onPointerDown={(e) => {
              if (e.button !== 0) return;
              handleCancel();
            }}
            disabled={isLoading}
          >
            {cancelLabel}
          </button>
          <button
            className={danger ? "btn btn-danger" : "btn btn-primary"}
            onClick={handleConfirm}
            onPointerDown={(e) => {
              if (e.button !== 0) return;
              handleConfirm();
            }}
            disabled={isLoading}
            autoFocus
          >
            {isLoading && loadingLabel ? loadingLabel : confirmLabel}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

export default ConfirmDialog;
