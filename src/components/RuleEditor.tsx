import { useState, useEffect, useRef, useCallback } from "react";
import type { HostRule, ParseErrorAtLine } from "../types";
import { validateHostsText } from "../lib/tauri";
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
      const line = rule.ip + " " + rule.domains.join(" ");
      if (rule.comment) {
        return line + " # " + rule.comment;
      }
      return line;
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

  // Cleanup on unmount
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

  // Sync text when rules prop changes externally (only on substantive changes)
  const prevRulesRef = useRef<HostRule[]>([]);
  useEffect(() => {
    const prevRules = prevRulesRef.current;
    const rulesChanged =
      rules.length !== prevRules.length ||
      rules.some(
        (r, i) =>
          r.ip !== prevRules[i]?.ip ||
          r.domains.join(",") !== prevRules[i]?.domains.join(","),
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
        setErrors([{ line_number: 0, error: "Validation failed: " + String(err) }]);
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
      setText(value);
      debouncedValidate(value);
    },
    [debouncedValidate],
  );

  return (
    <div className={styles.container}>
      <textarea
        className={`hosts-textarea ${errors.length > 0 ? styles.hasErrors : ""}`}
        value={text}
        onChange={handleChange}
        readOnly={readOnly}
        rows={Math.max(8, text.split("\n").length + 2)}
        spellCheck={false}
        placeholder="Enter hosts rules, one per line: IP domain1 domain2 # comment"
      />
      {isValidating && (
        <div className={styles.validating}>Validating...</div>
      )}
      {errors.length > 0 && (
        <div className={styles.errorList}>
          {errors.map((err) => (
            <div key={err.line_number} className={styles.errorItem}>
              Line {err.line_number}: {typeof err.error === 'string' ? err.error : JSON.stringify(err.error)}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export default RuleEditor;
