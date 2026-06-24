import { useState, useEffect, useCallback, useRef } from "react";
import type { Profile, ParseErrorAtLine } from "../types";
import { validateHostsText, importProfile, importProfileFromFile } from "../lib/tauri";
import { extractErrorMessage } from "../lib/error";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import styles from "./ImportDialog.module.css";

type ImportSource = "text" | "file-hosts" | "file-json";

interface ImportDialogProps {
  open: boolean;
  onClose: () => void;
  onImported: (profile: Profile) => void;
}

function ImportDialog({ open, onClose, onImported }: ImportDialogProps) {
  const [name, setName] = useState("");
  const [source, setSource] = useState<ImportSource>("text");
  const [hostsText, setHostsText] = useState("");
  const [filePath, setFilePath] = useState<string | null>(null);
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
      setSource("text");
      setHostsText("");
      setFilePath(null);
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

  const handleSourceChange = useCallback((newSource: ImportSource) => {
    setSource(newSource);
    setFilePath(null);
    setHostsText("");
    setErrors([]);
    setRuleCount(null);
    setImportError(null);
  }, []);

  const handleSelectFile = useCallback(async () => {
    try {
      const filters =
        source === "file-hosts"
          ? [{ name: "Hosts", extensions: ["hosts", "txt"] }]
          : [{ name: "JSON", extensions: ["json"] }];
      const path = await openFileDialog({ multiple: false, filters });
      if (path) {
        setFilePath(path as string);
        setImportError(null);
        // For hosts files, validate the content
        if (source === "file-hosts") {
          // We cannot read file content in frontend; validation will happen on import
          setRuleCount(null);
          setErrors([]);
        } else {
          // JSON files: trust the backend to parse
          setRuleCount(null);
          setErrors([]);
        }
      }
    } catch (err) {
      setImportError(extractErrorMessage(err));
    }
  }, [source]);

  // Cleanup timer on unmount
  useEffect(() => {
    return () => {
      if (validateTimerRef.current) {
        clearTimeout(validateTimerRef.current);
      }
    };
  }, []);

  const canImport =
    name.trim().length > 0 &&
    (source === "text"
      ? errors.length === 0 && ruleCount !== null
      : filePath !== null);

  const handleImport = useCallback(async () => {
    if (!canImport) return;
    setIsImporting(true);
    setImportError(null);
    try {
      let profile: Profile;
      if (source === "text") {
        profile = await importProfile(name.trim(), hostsText);
      } else {
        profile = await importProfileFromFile(name.trim(), filePath!);
      }
      onImported(profile);
      onClose();
    } catch (err) {
      setImportError(extractErrorMessage(err));
    } finally {
      setIsImporting(false);
    }
  }, [canImport, name, hostsText, filePath, source, onImported, onClose]);

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
          <label className="form-label">Import Source</label>
          <div className={styles.sourceSelector}>
            <button
              className={`btn btn-sm ${source === "text" ? "btn-primary" : "btn-ghost"}`}
              onClick={() => handleSourceChange("text")}
            >
              Paste Text
            </button>
            <button
              className={`btn btn-sm ${source === "file-hosts" ? "btn-primary" : "btn-ghost"}`}
              onClick={() => handleSourceChange("file-hosts")}
            >
              Hosts File
            </button>
            <button
              className={`btn btn-sm ${source === "file-json" ? "btn-primary" : "btn-ghost"}`}
              onClick={() => handleSourceChange("file-json")}
            >
              JSON File
            </button>
          </div>
        </div>

        {source === "text" && (
          <div className={styles.formGroup}>
            <label className="form-label" htmlFor="import-text">
              Hosts Text
            </label>
            <textarea
              id="import-text"
              className={`hosts-textarea ${errors.length > 0 ? styles.hasErrors : ""}`}
              placeholder="Paste hosts file content here..."
              value={hostsText}
              onChange={handleTextChange}
              rows={10}
              spellCheck={false}
            />
          </div>
        )}

        {(source === "file-hosts" || source === "file-json") && (
          <div className={styles.formGroup}>
            <label className="form-label">File</label>
            <div className={styles.fileRow}>
              <button
                className="btn btn-ghost btn-sm"
                onClick={handleSelectFile}
                disabled={isImporting}
              >
                Select File...
              </button>
              {filePath && (
                <span className={styles.filePath}>
                  {filePath.split("/").pop() || filePath}
                </span>
              )}
            </div>
          </div>
        )}

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
                Line {err.line_number}: {typeof err.error === "string" ? err.error : JSON.stringify(err.error)}
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
