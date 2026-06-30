import { useState, useEffect, useRef, useCallback, useMemo, useDeferredValue } from "react";
import type { HostRule, ValidateResult } from "../types";
import { validateHostsText } from "../lib/tauri";
import { extractErrorMessage } from "../lib/error";
import styles from "./RuleEditor.module.css";

interface RuleEditorProps {
  rules: HostRule[];
  onChange: (rules: HostRule[]) => void;
  onErrorChange?: (hasErrors: boolean) => void;
  readOnly?: boolean;
}

/** Convert HostRule[] to hosts file text format */
function rulesToText(rules: HostRule[]): string {
  return rules
    .map((rule) => {
      // Comment-only rule: output the comment as-is
      if (rule.ip === null || rule.ip === undefined) {
        return rule.comment || "";
      }
      const prefix = rule.enabled ? "" : "# ";
      const line = rule.ip + " " + rule.domains.join(" ");
      if (rule.comment) {
        return prefix + line + " # " + rule.comment;
      }
      return prefix + line;
    })
    .join("\n");
}

/** Escape HTML special chars for safe rendering in highlight layer */
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

/** Parse text into HTML with syntax highlighting */
function highlightText(text: string): string {
  if (!text) return "";
  const lines = text.split("\n");

  return lines
    .map((line) => {
      const trimmed = line.trim();

      if (trimmed === "") {
        return "";
      }

      // Full line comment
      if (trimmed.startsWith("#")) {
        return `<span class="${styles.tokenComment}">${escapeHtml(line)}</span>`;
      }

      let remaining = line;
      let html = "";

      // Disabled prefix
      if (remaining.startsWith("# ")) {
        html += `<span class="${styles.tokenComment}">${escapeHtml("# ")}</span>`;
        remaining = remaining.slice(2);
      }

      // IP address (IPv4 or IPv6)
      const ipv4Match = remaining.match(/^(\d+\.\d+\.\d+\.\d+)/);
      const ipv6Match = remaining.match(/^([0-9a-fA-F:]+)/);
      if (ipv4Match) {
        html += `<span class="${styles.tokenIp}">${escapeHtml(ipv4Match[1])}</span>`;
        remaining = remaining.slice(ipv4Match[1].length);
      } else if (ipv6Match && ipv6Match[1].includes(":")) {
        html += `<span class="${styles.tokenIp}">${escapeHtml(ipv6Match[1])}</span>`;
        remaining = remaining.slice(ipv6Match[1].length);
      }

      // Remaining: spaces, domains, inline comment
      const commentIdx = remaining.indexOf(" #");
      if (commentIdx >= 0) {
        const beforeComment = remaining.slice(0, commentIdx);
        const comment = remaining.slice(commentIdx);

        // Tokenize before comment
        const parts = beforeComment.split(/(\s+)/);
        for (const part of parts) {
          if (part === "") continue;
          if (/^\s+$/.test(part)) {
            html += escapeHtml(part); // spaces as-is
          } else {
            html += `<span class="${styles.tokenDomain}">${escapeHtml(part)}</span>`;
          }
        }

        html += `<span class="${styles.tokenComment}">${escapeHtml(comment)}</span>`;
      } else {
        const parts = remaining.split(/(\s+)/);
        for (const part of parts) {
          if (part === "") continue;
          if (/^\s+$/.test(part)) {
            html += escapeHtml(part);
          } else {
            html += `<span class="${styles.tokenDomain}">${escapeHtml(part)}</span>`;
          }
        }
      }

      return html;
    })
    .join("\n");
}

/** Simple debounce hook */
function useDebouncedCallback<T extends (...args: Parameters<T>) => void>(
  callback: T,
  delay: number,
): T {
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const callbackRef = useRef(callback);
  callbackRef.current = callback;

  const debounced = useCallback(
    (...args: Parameters<T>) => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
      }
      timerRef.current = setTimeout(() => {
        callbackRef.current(...args);
      }, delay);
    },
    [delay],
  ) as T;

  useEffect(() => {
    return () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
      }
    };
  }, []);

  return debounced;
}

function RuleEditor({ rules, onChange, onErrorChange, readOnly = false }: RuleEditorProps) {
  const [text, setText] = useState(() => rulesToText(rules));
  const [validateResult, setValidateResult] = useState<ValidateResult | null>(null);
  const errors = validateResult?.errors ?? [];
  const [isValidating, setIsValidating] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const highlightRef = useRef<HTMLDivElement>(null);

  // Track whether the user is actively editing — prevents external rule sync
  // from overwriting the textarea while the user is typing.
  const isEditingRef = useRef(false);

  // Sync text when rules prop changes externally (not from our own onChange)
  const prevRulesRef = useRef<HostRule[]>([]);
  useEffect(() => {
    const newText = rulesToText(rules);

    // If the generated text matches current text, just update the ref
    // and skip overwriting (preserves cursor position).
    // Also reset editing flag — user has converged on the validated state.
    if (newText === text) {
      isEditingRef.current = false;
      prevRulesRef.current = rules;
      return;
    }

    // Skip text overwrite if the change originated from our own editing.
    if (isEditingRef.current) {
      isEditingRef.current = false;
      prevRulesRef.current = rules;
      return;
    }

    const prevRules = prevRulesRef.current;
    const rulesChanged =
      rules.length !== prevRules.length ||
      rules.some(
        (r, i) =>
          r.ip !== prevRules[i]?.ip ||
          r.domains.join(",") !== prevRules[i]?.domains.join(",") ||
          r.enabled !== prevRules[i]?.enabled ||
          r.comment !== prevRules[i]?.comment,
      );
    if (rulesChanged) {
      setText(newText);
      setValidateResult(null);
      prevRulesRef.current = rules;
    }
  }, [rules]);

  const handleValidate = useCallback(
    async (value: string) => {
      setIsValidating(true);
      try {
        const result = await validateHostsText(value);
        setValidateResult(result);
        const hasBlockingIssues = result.errors.length > 0 || result.duplicates.some((d) => d.kind === "different_ip");
        onErrorChange?.(hasBlockingIssues);
        if (!hasBlockingIssues) {
          onChange(result.rules);
        }
      } catch (err) {
        setValidateResult({
          rules: [],
          errors: [{ line_number: 0, error: "Validation failed: " + extractErrorMessage(err) }],
          duplicates: [],
        });
      } finally {
        setIsValidating(false);
      }
    },
    [onChange, onErrorChange],
  );

  const debouncedValidate = useDebouncedCallback(handleValidate, 300);

  const handleChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement>) => {
      const value = e.target.value;
      isEditingRef.current = true; // mark as editing to prevent external sync
      setText(value);
      debouncedValidate(value);
    },
    [debouncedValidate],
  );

  // Sync scroll between textarea and highlight layer
  const handleScroll = useCallback(() => {
    if (textareaRef.current && highlightRef.current) {
      highlightRef.current.scrollTop = textareaRef.current.scrollTop;
      highlightRef.current.scrollLeft = textareaRef.current.scrollLeft;
    }
  }, []);

  // Generate highlighted content — Perf fix (#30): use deferred value to avoid blocking on every keystroke
  const deferredText = useDeferredValue(text);
  const highlightedHtml = useMemo(() => highlightText(deferredText), [deferredText]);
  const editorHasBlockingIssues = errors.length > 0 || (validateResult?.duplicates?.some((d) => d.kind === "different_ip") ?? false);

  return (
    <div className={styles.container}>
      <div className={`${styles.editorWrapper} ${editorHasBlockingIssues ? styles.editorWrapperHasErrors : ""}`}>
        {/* Highlight Layer */}
        <div
          ref={highlightRef}
          className={styles.highlightLayer}
          aria-hidden="true"
          dangerouslySetInnerHTML={{ __html: highlightedHtml }}
        />

        {/* Textarea */}
        <textarea
          ref={textareaRef}
          className={styles.textarea}
          value={text}
          onChange={handleChange}
          onScroll={handleScroll}
          readOnly={readOnly}
          spellCheck={false}
          placeholder="Enter hosts rules, one per line:&#10;127.0.0.1 localhost # local dev&#10;192.168.1.100 api.dev.local # API server"
        />
      </div>

      {isValidating && (
        <div className={styles.validating}>Validating...</div>
      )}
      {(errors.length > 0 || (validateResult?.duplicates?.length ?? 0) > 0) && (
        <div className={styles.errorList}>
          {errors.map((err) => (
            <div key={err.line_number} className={styles.errorItem}>
              Line {err.line_number}: {typeof err.error === "string" ? err.error : JSON.stringify(err.error)}
            </div>
          ))}
          {validateResult?.duplicates?.map((dup) => (
            <div
              key={`dup-${dup.domain}`}
              className={
                dup.kind === "different_ip"
                  ? styles.errorItem
                  : styles.warningItem
              }
            >
              {dup.kind === "different_ip"
                ? `冲突: 域名 "${dup.domain}" 映射到不同 IP (行 ${dup.lines.join(", ")})`
                : `冗余: 域名 "${dup.domain}" 重复出现 (行 ${dup.lines.join(", ")})`}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export default RuleEditor;
