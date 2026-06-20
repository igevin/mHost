import { useState, useEffect, useRef, useCallback } from "react";
import type { HostRule, ParseErrorAtLine } from "../types";
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
      const prefix = rule.enabled ? "" : "# ";
      const line = rule.ip + " " + rule.domains.join(" ");
      if (rule.comment) {
        return prefix + line + " # " + rule.comment;
      }
      return prefix + line;
    })
    .join("\n");
}

/** Parse text into tokens for syntax highlighting */
function tokenize(text: string | undefined): { type: string; text: string }[] {
  const tokens: { type: string; text: string }[] = [];
  if (!text) return tokens;
  const lines = text.split("\n");

  lines.forEach((line, lineIdx) => {
    const trimmed = line.trim();

    if (trimmed === "") {
      tokens.push({ type: "empty", text: lineIdx === lines.length - 1 ? "" : "\n" });
      return;
    }

    // Full line comment
    if (trimmed.startsWith("#")) {
      tokens.push({ type: "comment", text: line + (lineIdx === lines.length - 1 ? "" : "\n") });
      return;
    }

    let remaining = line;

    // Disabled prefix
    if (remaining.startsWith("# ")) {
      tokens.push({ type: "comment", text: "# " });
      remaining = remaining.slice(2);
    }

    // IP address
    const ipMatch = remaining.match(/^(\d+\.\d+\.\d+\.\d+)/);
    if (ipMatch) {
      tokens.push({ type: "ip", text: ipMatch[1] });
      remaining = remaining.slice(ipMatch[1].length);
    }

    // Remaining: spaces, domains, inline comment
    const commentIdx = remaining.indexOf(" #");
    if (commentIdx >= 0) {
      const beforeComment = remaining.slice(0, commentIdx);
      const comment = remaining.slice(commentIdx);

      // Tokenize before comment: spaces and domains
      const parts = beforeComment.split(/(\s+)/);
      for (const part of parts) {
        if (part === "") continue;
        if (/^\s+$/.test(part)) {
          tokens.push({ type: "space", text: part });
        } else {
          tokens.push({ type: "domain", text: part });
        }
      }

      tokens.push({ type: "comment", text: comment });
    } else {
      const parts = remaining.split(/(\s+)/);
      for (const part of parts) {
        if (part === "") continue;
        if (/^\s+$/.test(part)) {
          tokens.push({ type: "space", text: part });
        } else {
          tokens.push({ type: "domain", text: part });
        }
      }
    }

    if (lineIdx < lines.length - 1) {
      tokens.push({ type: "newline", text: "\n" });
    }
  });

  return tokens;
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
  const [errors, setErrors] = useState<ParseErrorAtLine[]>([]);
  const [isValidating, setIsValidating] = useState(false);
  const editorRef = useRef<HTMLDivElement>(null);

  // Sync text when rules prop changes externally
  const prevRulesRef = useRef<HostRule[]>([]);
  useEffect(() => {
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
      setText(rulesToText(rules));
      setErrors([]);
      prevRulesRef.current = rules;
    }
  }, [rules]);

  const handleValidate = useCallback(
    async (value: string) => {
      setIsValidating(true);
      try {
        const result = await validateHostsText(value);
        setErrors(result.errors);
        onErrorChange?.(result.errors.length > 0);
        if (result.errors.length === 0) {
          onChange(result.rules);
        }
      } catch (err) {
        setErrors([{ line_number: 0, error: "Validation failed: " + extractErrorMessage(err) }]);
      } finally {
        setIsValidating(false);
      }
    },
    [onChange, onErrorChange],
  );

  const debouncedValidate = useDebouncedCallback(handleValidate, 300);

  const handleInput = useCallback(
    (e: React.FormEvent<HTMLDivElement>) => {
      const value = e.currentTarget.innerText;
      setText(value);
      debouncedValidate(value);
    },
    [debouncedValidate],
  );

  // Prevent formatting on paste
  const handlePaste = useCallback((e: React.ClipboardEvent) => {
    e.preventDefault();
    const text = e.clipboardData.getData("text/plain");
    document.execCommand("insertText", false, text);
  }, []);

  // Generate highlighted content
  const tokens = tokenize(text);

  return (
    <div className={styles.container}>
      <div
        ref={editorRef}
        role="textbox"
        aria-multiline="true"
        className={`${styles.editor} ${errors.length > 0 ? styles.hasErrors : ""} ${readOnly ? styles.readOnly : ""}`}
        contentEditable={!readOnly}
        onInput={handleInput}
        onPaste={handlePaste}
        suppressContentEditableWarning
        spellCheck={false}
      >
        {tokens.map((token, idx) => (
          <span
            key={idx}
            className={styles[`token${token.type.charAt(0).toUpperCase() + token.type.slice(1)}`]}
          >
            {token.text}
          </span>
        ))}
      </div>

      {isValidating && (
        <div className={styles.validating}>Validating...</div>
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
    </div>
  );
}

export default RuleEditor;
