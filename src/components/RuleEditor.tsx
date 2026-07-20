import { useState, useEffect, useRef, useCallback, useMemo, useDeferredValue } from "react";
import type { HostRule, ValidateResult } from "../types";
import { validateHostsText } from "../lib/tauri";
import { extractErrorMessage } from "../lib/error";
import { findMatches } from "../lib/search";
import type { MatchInfo } from "../lib/search";
import { escapeHtml } from "../lib/escape";
import SearchBar from "./SearchBar";
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

/** Single-line syntax highlighting (no search marks) */
function highlightLine(line: string): string {
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
}

/** Parse text into HTML with syntax highlighting and search marks */
function highlightText(
  text: string,
  matches: MatchInfo[],
  activeMatchIndex: number,
): string {
  if (!text) return "";
  const lines = text.split("\n");
  const lineStarts: number[] = [];
  let offset = 0;
  for (let i = 0; i < lines.length; i++) {
    lineStarts.push(offset);
    offset += lines[i].length + 1;
  }

  return lines
    .map((line, lineIdx) => {
      const lineStart = lineStarts[lineIdx];
      const lineEnd = lineStart + line.length;
      const lineMatches = matches
        .map((m, idx) => ({ ...m, matchIndex: idx }))
        .filter((m) => m.start < lineEnd && m.end > lineStart);

      if (lineMatches.length === 0) {
        return highlightLine(line);
      }

      const segments: { text: string; isMatch: boolean; matchIndex: number }[] = [];
      let currentPos = 0;

      for (const match of lineMatches) {
        const matchStartInLine = Math.max(0, match.start - lineStart);
        const matchEndInLine = Math.min(line.length, match.end - lineStart);

        if (matchStartInLine > currentPos) {
          segments.push({
            text: line.slice(currentPos, matchStartInLine),
            isMatch: false,
            matchIndex: -1,
          });
        }

        if (matchEndInLine > matchStartInLine) {
          segments.push({
            text: line.slice(matchStartInLine, matchEndInLine),
            isMatch: true,
            matchIndex: match.matchIndex,
          });
        }
        currentPos = Math.max(currentPos, matchEndInLine);
      }

      if (currentPos < line.length) {
        segments.push({
          text: line.slice(currentPos),
          isMatch: false,
          matchIndex: -1,
        });
      }

      return segments
        .map((seg) => {
          const segHtml = highlightLine(seg.text);
          if (seg.isMatch) {
            const isActive = seg.matchIndex === activeMatchIndex;
            const markClass = isActive ? styles.searchMatchActive : styles.searchMatch;
            return `<mark class="${markClass}">${segHtml}</mark>`;
          }
          return segHtml;
        })
        .join("");
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
  const lineNumbersRef = useRef<HTMLDivElement>(null);

  // Track whether the user is actively editing — prevents external rule sync
  // from overwriting the textarea while the user is typing.
  const isEditingRef = useRef(false);

  // Search state
  const [searchQuery, setSearchQuery] = useState("");
  const [replaceText, setReplaceText] = useState("");
  const [searchBarVisible, setSearchBarVisible] = useState(false);
  const [currentMatchIndex, setCurrentMatchIndex] = useState(0);

  const matches = useMemo(() => findMatches(text, searchQuery), [text, searchQuery]);

  // Clamp currentMatchIndex when matches change
  useEffect(() => {
    setCurrentMatchIndex((prev) => {
      if (matches.length === 0) return 0;
      if (prev >= matches.length) return matches.length - 1;
      return prev;
    });
  }, [matches]);

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
  }, [rules, text]);

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

  // Sync scroll between textarea, highlight layer, and line numbers
  const handleScroll = useCallback(() => {
    if (textareaRef.current && highlightRef.current && lineNumbersRef.current) {
      highlightRef.current.scrollTop = textareaRef.current.scrollTop;
      highlightRef.current.scrollLeft = textareaRef.current.scrollLeft;
      lineNumbersRef.current.scrollTop = textareaRef.current.scrollTop;
    }
  }, []);

  // Track the line the caret is on — used to highlight the active row in the gutter.
  // Derives from textarea.selectionStart so it works for both keyboard nav and clicks.
  //
  // Perf (issue #125 review): the previous implementation did
  //   value.slice(0, selectionStart).split("\n").length - 1
  // on every keystroke, re-scanning the prefix from char 0 every time. For a
  // 5k-line file with the caret at line 3000, that's 3000 char-codes per
  // keystroke. We now keep a (lastSel, lastLine) cache and walk only the
  // *delta* between selections — O(diff), typically 1 char. The fast path
  // (sel unchanged) returns immediately, and the equality guard on the
  // setState prevents re-renders when the line index didn't change.
  const [activeLineNumber, setActiveLineNumber] = useState(0);
  const lastSelRef = useRef(0);
  const lastLineRef = useRef(0);

  const handleActiveLineUpdate = useCallback(() => {
    const ta = textareaRef.current;
    if (!ta) return;
    const sel = ta.selectionStart;
    if (sel === lastSelRef.current) return; // fast path

    const lastSel = lastSelRef.current;
    const lastLine = lastLineRef.current;
    const lo = Math.min(sel, lastSel);
    const hi = Math.max(sel, lastSel);
    let delta = 0;
    // \n = charCode 10. Walking the delta (not the full prefix) makes this
    // O(diff): 1 for a keystroke, N for a paste of N chars.
    for (let i = lo; i < hi; i++) {
      if (ta.value.charCodeAt(i) === 10) delta++;
    }
    const lineIdx = sel > lastSel ? lastLine + delta : lastLine - delta;

    lastSelRef.current = sel;
    lastLineRef.current = lineIdx;
    setActiveLineNumber((prev) => (prev === lineIdx ? prev : lineIdx));
  }, []);

  // Scroll to a specific line in the textarea
  const scrollToMatch = useCallback((lineIndex: number) => {
    if (textareaRef.current) {
      const lineHeight = 20.8; // 13px * 1.6
      textareaRef.current.scrollTop = lineIndex * lineHeight;
    }
  }, []);

  // Navigation handlers
  const handlePrev = useCallback(() => {
    if (matches.length === 0) return;
    const newIndex = currentMatchIndex <= 0 ? matches.length - 1 : currentMatchIndex - 1;
    setCurrentMatchIndex(newIndex);
    scrollToMatch(matches[newIndex].lineIndex);
  }, [matches, currentMatchIndex, scrollToMatch]);

  const handleNext = useCallback(() => {
    if (matches.length === 0) return;
    const newIndex = currentMatchIndex >= matches.length - 1 ? 0 : currentMatchIndex + 1;
    setCurrentMatchIndex(newIndex);
    scrollToMatch(matches[newIndex].lineIndex);
  }, [matches, currentMatchIndex, scrollToMatch]);

  // Replace handlers
  const handleReplace = useCallback(() => {
    if (matches.length === 0 || currentMatchIndex < 0 || currentMatchIndex >= matches.length) return;
    const match = matches[currentMatchIndex];
    const newText = text.slice(0, match.start) + replaceText + text.slice(match.end);
    isEditingRef.current = true;
    setText(newText);
    debouncedValidate(newText);
  }, [text, matches, currentMatchIndex, replaceText, debouncedValidate]);

  const handleReplaceAll = useCallback(() => {
    if (matches.length === 0) return;
    let newText = text;
    for (let i = matches.length - 1; i >= 0; i--) {
      const match = matches[i];
      newText = newText.slice(0, match.start) + replaceText + newText.slice(match.end);
    }
    isEditingRef.current = true;
    setText(newText);
    debouncedValidate(newText);
    setCurrentMatchIndex(0);
  }, [text, matches, replaceText, debouncedValidate]);

  // Keyboard shortcuts for search
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "f") {
        e.preventDefault();
        setSearchBarVisible(true);
        return;
      }

      if (e.key === "Escape" && searchBarVisible) {
        const target = e.target as HTMLElement;
        const isInSearchBar =
          typeof target.closest === "function" &&
          target.closest('[data-search-bar="true"]') !== null;
        if (!isInSearchBar) {
          e.preventDefault();
          setSearchBarVisible(false);
          setSearchQuery("");
          setReplaceText("");
        }
      }
    };

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [searchBarVisible]);

  // Generate highlighted content — Perf fix (#30): use deferred value to avoid blocking on every keystroke
  const deferredText = useDeferredValue(text);
  const highlightedHtml = useMemo(
    () => highlightText(deferredText, matches, currentMatchIndex),
    [deferredText, matches, currentMatchIndex],
  );
  const editorHasBlockingIssues = errors.length > 0 || (validateResult?.duplicates?.some((d) => d.kind === "different_ip") ?? false);

  // **fix (P-F8, issue #90)**: line numbers depended on `[text]` — every
  // keystroke (even within the same line) rebuilt the array. Split the
  // dependency: a cheap `lineCount` memo on `[text]` (recomputed only when
  // text changes — full split+count) and the `lineNumbers` memo depends
  // only on `[lineCount]`. For typing within a line, `lineCount` reference
  // is stable → `lineNumbers` returns the same array (React sees identity
  // equality). For typing across a newline, `lineCount` changes by ±1
  // → array is rebuilt, but only by the delta size.
  const lineCount = useMemo(() => text.split("\n").length, [text]);
  const lineNumbers = useMemo(
    () => Array.from({ length: lineCount }, (_, i) => i + 1),
    [lineCount],
  );

  return (
    <div className={styles.container}>
      <SearchBar
        visible={searchBarVisible}
        onClose={() => {
          setSearchBarVisible(false);
          setSearchQuery("");
          setReplaceText("");
        }}
        query={searchQuery}
        onQueryChange={setSearchQuery}
        replaceText={replaceText}
        onReplaceTextChange={setReplaceText}
        matchCount={matches.length}
        currentMatchIndex={currentMatchIndex}
        onPrev={handlePrev}
        onNext={handleNext}
        onReplace={handleReplace}
        onReplaceAll={handleReplaceAll}
        readOnly={readOnly}
      />
      <div className={`${styles.editorWrapper} ${editorHasBlockingIssues ? styles.editorWrapperHasErrors : ""}`}>
        {/* Line Numbers */}
        <div ref={lineNumbersRef} className={styles.lineNumbers}>
          {lineNumbers.map((num) => {
            const lineIdx = num - 1;
            const isActive = !readOnly && lineIdx === activeLineNumber;
            return (
              <div
                key={num}
                className={`${styles.lineNumber} ${isActive ? styles.lineNumberActive : ""}`}
              >
                {num}
              </div>
            );
          })}
        </div>

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
          // onSelect alone covers clicks, arrow keys, Home/End, and Shift+arrow
          // extends (the browser fires `select` whenever the selection changes).
          // Adding onClick/onKeyUp on top only causes redundant re-renders.
          onSelect={handleActiveLineUpdate}
          readOnly={readOnly}
          spellCheck={false}
          placeholder="Enter hosts rules, one per line:&#10;127.0.0.1 localhost # local dev&#10;192.168.1.100 api.dev.local # API server"
        />
      </div>

      {isValidating && <div className={styles.validating}>Validating...</div>}
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
              className={dup.kind === "different_ip" ? styles.errorItem : styles.warningItem}
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
