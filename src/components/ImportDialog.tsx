import { useState, useEffect, useCallback, useRef } from "react";
import type { Profile, ParseErrorAtLine } from "../types";
import { validateHostsText, importProfile } from "../lib/tauri";
import styles from "./ImportDialog.module.css";

interface ImportDialogProps {
  open: boolean;
  onClose: () => void;
  onImported: (profile: Profile) => void;
}

function ImportDialog({ open, onClose, onImported }: ImportDialogProps) {
  const [name, setName] = useState("");
  const [hostsText, setHostsText] = useState("");
  const [errors, setErrors] = useState<ParseErrorAtLine[]>([]);
  const [ruleCount, setRuleCount] = useState<number | null>(null);
  const [isValidating, setIsValidating] = useState(false);
  const [isImporting, setIsImporting] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);
  const validateTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Reset state when dialog opens/closes
  useEffect(() => {
    if (open) {
      setName("");
      setHostsText("");
      setErrors([]);
      setRuleCount(null);
      setIsValidating(false);
      setIsImporting(false);
      setImportError(null);
    }
  }, [open]);

  const validateText = useCallback(async (text: string) => {
    if (!text.trim()) {
      setErrors([]);
      setRuleCount(null);
      return;
    }
    setIsValidating(true);
    try {
      const result = await validateHostsText(text);
      setErrors(result.errors);
      setRuleCount(result.errors.length === 0 ? result.rules.length : null);
    } catch (_err) {
      setErrors([{ line_number: 0, error: "Validation failed" }]);
      setRuleCount(null);
    } finally {
      setIsValidating(false);
    }
  }, []);

  const handleTextChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement>) => {
      const value = e.target.value;
      setHostsText(value);
      setImportError(null);

      // Debounce validation (300ms)
      if (validateTimerRef.current) {
        clearTimeout(validateTimerRef.current);
      }
      validateTimerRef.current = setTimeout(() => {
        validateText(value);
      }, 300);
    },
    [validateText],
  );

  // Cleanup timer on unmount
  useEffect(() => {
    return () => {
      if (validateTimerRef.current) {
        clearTimeout(validateTimerRef.current);
      }
    };
  }, []);

  const canImport = name.trim().length > 0 && errors.length === 0 && ruleCount !== null && ruleCount > 0;

  const handleImport = useCallback(async () => {
    if (!canImport) return;
    setIsImporting(true);
    setImportError(null);
    try {
      const profile = await importProfile(name.trim(), hostsText);
      onImported(profile);
      onClose();
    } catch (err) {
      setImportError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsImporting(false);
    }
  }, [canImport, name, hostsText, onImported, onClose]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      }
    },
    [onClose],
  );

  if (!open) return null;

  return (
    <div className={styles.overlay} onClick={onClose} onKeyDown={handleKeyDown}>
      <div className={styles.dialog} onClick={(e) => e.stopPropagation()}>
        <h3 className={styles.title}>Import Profile</h3>

        <div className={styles.formGroup}>
          <label className="form-label" htmlFor="import-name">
            Profile Name
          </label>
          <input
            id="import-name"
            className="input"
            placeholder="Enter profile name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoFocus
          />
        </div>

        <div className={styles.formGroup}>
          <label className="form-label" htmlFor="import-text">
            Hosts Text
          </label>
          <textarea
            id="import-text"
            className={`${styles.textarea} ${errors.length > 0 ? styles.hasErrors : ""}`}
            placeholder="Paste hosts file content here..."
            value={hostsText}
            onChange={handleTextChange}
            rows={10}
            spellCheck={false}
          />
        </div>

        {isValidating && (
          <div className={styles.status}>Validating...</div>
        )}

        {ruleCount !== null && ruleCount > 0 && (
          <div className={styles.preview}>
            Preview: {ruleCount} rule{ruleCount !== 1 ? "s" : ""} parsed successfully
          </div>
        )}

        {errors.length > 0 && (
          <div className={styles.errorList}>
            {errors.map((err) => (
              <div key={err.line_number} className={styles.errorItem}>
                Line {err.line_number}: {err.error}
              </div>
            ))}
          </div>
        )}

        {importError && (
          <div className="alert alert-error">{importError}</div>
        )}

        <div className={styles.actions}>
          <button
            className="btn btn-primary"
            onClick={handleImport}
            disabled={!canImport || isImporting}
          >
            {isImporting ? "Importing..." : "Import"}
          </button>
          <button className="btn btn-ghost" onClick={onClose}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}

export default ImportDialog;
