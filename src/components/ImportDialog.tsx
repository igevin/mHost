import { useState, useEffect, useCallback, useRef } from "react";
import { createPortal } from "react-dom";
import type { HostRule, Profile, ParseErrorAtLine, DuplicateRule } from "../types";
import { countRealRules } from "../lib/rules";
import { validateHostsText, importProfile, importProfileFromFile } from "../lib/tauri";
import { extractErrorMessage } from "../lib/error";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import styles from "./ImportDialog.module.css";

type ImportSource = "text" | "file-hosts" | "file-json";

interface ImportDialogProps {
  open: boolean;
  onClose: () => void;
  mode?: "create" | "replace";
  onImported?: (profile: Profile) => void;
  onRulesParsed?: (rules: HostRule[], tempProfileId?: string) => void;
}

function ImportDialog({ open, onClose, mode = "create", onImported, onRulesParsed }: ImportDialogProps) {
  const isReplace = mode === "replace";
  const [name, setName] = useState("");
  const [source, setSource] = useState<ImportSource>("text");
  const [hostsText, setHostsText] = useState("");
  const [filePath, setFilePath] = useState<string | null>(null);
  const [errors, setErrors] = useState<ParseErrorAtLine[]>([]);
  const [duplicates, setDuplicates] = useState<DuplicateRule[]>([]);
  const [ruleCount, setRuleCount] = useState<number | null>(null);
  const [isValidating, setIsValidating] = useState(false);
  const [isImporting, setIsImporting] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);
  const validateTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const { fire, release, onPointerDown } = useWebKitPointerDown();

  // Reset state when dialog opens/closes
  useEffect(() => {
    if (open) {
      setName("");
      setSource("text");
      setHostsText("");
      setFilePath(null);
      setErrors([]);
      setDuplicates([]);
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
      setDuplicates(result.duplicates);
      const hasBlocking = result.errors.length > 0 || result.duplicates.some((d) => d.kind === "different_ip");
      setRuleCount(!hasBlocking ? countRealRules(result.rules) : null);
    } catch (_err) {
      setErrors([{ line_number: 0, error: "Validation failed" }]);
      setDuplicates([]);
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
    setDuplicates([]);
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
      if (typeof path === "string") {
        setFilePath(path);
        setImportError(null);
        // Validation will happen on import; clear any stale state
        setRuleCount(null);
        setErrors([]);
        setDuplicates([]);
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

  const hasTextBlocking = errors.length > 0 || duplicates.some((d) => d.kind === "different_ip");
  const canImport = isReplace
    ? source === "text"
      ? !hasTextBlocking && ruleCount !== null
      : filePath !== null
    : name.trim().length > 0 &&
      (source === "text"
        ? !hasTextBlocking && ruleCount !== null
        : filePath !== null);

  const handleImport = useCallback(async () => {
    if (!fire()) return;
    if (!canImport) {
      release();
      return;
    }
    setIsImporting(true);
    setImportError(null);
    try {
      if (isReplace && onRulesParsed) {
        // Replace mode: parse rules without creating a visible new profile
        if (source === "text") {
          const result = await validateHostsText(hostsText);
          if (result.errors.length > 0 || result.duplicates.some((d) => d.kind === "different_ip")) {
            setErrors(result.errors);
            setDuplicates(result.duplicates);
            setIsImporting(false);
            release();
            return;
          }
          onRulesParsed(result.rules);
        } else {
          if (!filePath) return;
          // Create a temporary profile to parse the file, then extract rules
          const tempName = `__import_temp_${Date.now()}`;
          const tempProfile = await importProfileFromFile(tempName, filePath);
          onRulesParsed(tempProfile.rules, tempProfile.id);
        }
        onClose();
      } else if (onImported) {
        // Create mode: create a new profile
        let profile: Profile;
        if (source === "text") {
          profile = await importProfile(name.trim(), hostsText);
        } else {
          if (!filePath) return;
          profile = await importProfileFromFile(name.trim(), filePath);
        }
        onImported(profile);
        onClose();
      }
    } catch (err) {
      setImportError(extractErrorMessage(err));
    } finally {
      setIsImporting(false);
      release();
    }
  }, [canImport, isReplace, source, hostsText, filePath, name, onRulesParsed, onImported, onClose, fire, release]);

  const handleCancel = useCallback(() => {
    if (isImporting) return;
    onClose();
  }, [isImporting, onClose]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      }
    },
    [onClose],
  );

  if (!open) return null;

  return createPortal(
    <div className={styles.overlay} onClick={onClose} onKeyDown={handleKeyDown}>
      <div className={styles.dialog} onClick={(e) => e.stopPropagation()}>
        <h3 className={styles.title}>
          {isReplace ? "Import Rules" : "Import Profile"}
        </h3>

        {!isReplace && (
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
        )}

        <div className={styles.formGroup}>
          <label className="form-label">Import Source</label>
          <div className={styles.sourceSelector}>
            <button
              className={`btn btn-sm ${source === "text" ? "btn-primary" : "btn-ghost"}`}
              onClick={() => handleSourceChange("text")}
              onPointerDown={(e) => { if (e.button === 0) handleSourceChange("text"); }}
            >
              Paste Text
            </button>
            <button
              className={`btn btn-sm ${source === "file-hosts" ? "btn-primary" : "btn-ghost"}`}
              onClick={() => handleSourceChange("file-hosts")}
              onPointerDown={(e) => { if (e.button === 0) handleSourceChange("file-hosts"); }}
            >
              Hosts File
            </button>
            <button
              className={`btn btn-sm ${source === "file-json" ? "btn-primary" : "btn-ghost"}`}
              onClick={() => handleSourceChange("file-json")}
              onPointerDown={(e) => { if (e.button === 0) handleSourceChange("file-json"); }}
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
              className={`hosts-textarea ${hasTextBlocking ? styles.hasErrors : ""}`}
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
                onPointerDown={onPointerDown(handleSelectFile)}
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

        {duplicates.length > 0 && (
          <div className={styles.errorList}>
            {duplicates.map((dup) => (
              <div
                key={`dup-${dup.domain}`}
                className={dup.kind === "different_ip" ? styles.errorItem : styles.warningItem}
              >
                {dup.kind === "different_ip"
                  ? `冲突: 域名 "${dup.domain}" 映射到不同 IP (行 ${dup.lines.join(", ")})`
                  : `冗余: 域名 "${dup.domain}" 重复出现 (行 ${dup.lines.join(", ")})`}
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
            onPointerDown={onPointerDown(handleImport)}
            disabled={!canImport || isImporting}
          >
            {isImporting ? "Importing..." : isReplace ? "Import & Replace" : "Import"}
          </button>
          <button
            className="btn btn-ghost"
            onClick={handleCancel}
            onPointerDown={onPointerDown(handleCancel)}
            disabled={isImporting}
          >
            Cancel
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

export default ImportDialog;
