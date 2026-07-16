import { useState, useEffect, useRef, useMemo, useCallback } from "react";
import { readSystemHosts } from "../lib/tauri";
import { extractErrorMessage } from "../lib/error";
import { findMatches } from "../lib/search";
import type { MatchInfo } from "../lib/search";
import { escapeHtml } from "../lib/escape";
import SearchBar from "../components/SearchBar";
import styles from "./SystemHosts.module.css";

/**
 * Wrap case-insensitive literal matches in `<mark>` tags. No syntax
 * highlighting — SystemHosts is plain text, so each segment is just
 * escaped HTML with optional mark wrapping. The active match gets a
 * different class and a `data-match-index` attribute so callers can
 * scroll it into view.
 */
function highlightMatches(
  text: string,
  matches: MatchInfo[],
  activeMatchIndex: number,
): string {
  if (!text) return "";
  if (matches.length === 0) return escapeHtml(text);

  const segments: { text: string; isMatch: boolean; matchIndex: number }[] = [];
  let currentPos = 0;
  for (let i = 0; i < matches.length; i++) {
    const m = matches[i];
    if (m.start > currentPos) {
      segments.push({ text: text.slice(currentPos, m.start), isMatch: false, matchIndex: -1 });
    }
    if (m.end > m.start) {
      segments.push({ text: text.slice(m.start, m.end), isMatch: true, matchIndex: i });
    }
    currentPos = Math.max(currentPos, m.end);
  }
  if (currentPos < text.length) {
    segments.push({ text: text.slice(currentPos), isMatch: false, matchIndex: -1 });
  }

  return segments
    .map((seg) => {
      const html = escapeHtml(seg.text);
      if (seg.isMatch) {
        const isActive = seg.matchIndex === activeMatchIndex;
        const cls = isActive ? styles.searchMatchActive : styles.searchMatch;
        return `<mark class="${cls}" data-match-index="${seg.matchIndex}">${html}</mark>`;
      }
      return html;
    })
    .join("");
}

function SystemHosts() {
  const [hostsContent, setHostsContent] = useState<string | null>(null);
  const [hostsError, setHostsError] = useState<string | null>(null);

  // Search state — mirrors RuleEditor's pattern.
  const [searchQuery, setSearchQuery] = useState("");
  const [searchBarVisible, setSearchBarVisible] = useState(false);
  const [currentMatchIndex, setCurrentMatchIndex] = useState(0);
  const previewRef = useRef<HTMLPreElement>(null);
  const gutterRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let mounted = true;
    readSystemHosts()
      .then((content) => {
        if (mounted) setHostsContent(content);
      })
      .catch((err) => {
        if (mounted) setHostsError(extractErrorMessage(err));
      });
    return () => {
      mounted = false;
    };
  }, []);

  const matches = useMemo(
    () => (hostsContent ? findMatches(hostsContent, searchQuery) : []),
    [hostsContent, searchQuery],
  );

  // Total line count of the hosts file. Memoized because re-splitting on
  // every keystroke is wasteful. Used for the line-number gutter (issue
  // #111): `Array.from({ length: totalLines })` produces the gutter rows.
  const totalLines = useMemo(
    () => (hostsContent ? hostsContent.split("\n").length : 0),
    [hostsContent],
  );

  // 1-based numbers for every line in the file. The gutter renders one
  // <div class="lineNumber"> per entry. Memoized on `[totalLines]` so
  // typing a search query (which only changes `matches`, not `totalLines`)
  // does not rebuild this array — perf fix from issue #111 (mirrors the
  // line-numbers memo pattern in RuleEditor.tsx).
  const lineNumbers = useMemo(
    () => Array.from({ length: totalLines }, (_, i) => i + 1),
    [totalLines],
  );

  // 1-based line number of the currently active match, or null when no
  // match is active. Drives the gutter's highlighted line (no separate
  // badge anymore — #111 made the gutter the primary affordance).
  const activeLineNumber =
    searchBarVisible &&
    matches.length > 0 &&
    currentMatchIndex < matches.length
      ? matches[currentMatchIndex].lineIndex + 1
      : null;

  // Clamp currentMatchIndex when the match set shrinks (e.g. user shortens
  // the query). Mirrors RuleEditor.tsx behavior.
  useEffect(() => {
    setCurrentMatchIndex((prev) => {
      if (matches.length === 0) return 0;
      if (prev >= matches.length) return matches.length - 1;
      return prev;
    });
  }, [matches]);

  // Scroll the active match into view on navigation. scrollIntoView handles
  // wrapped lines + variable line heights without us hardcoding lineHeight.
  // Guard for jsdom / non-browser environments where scrollIntoView is absent.
  //
  // `matches` is in the deps because the effect reads `matches.length`, but it
  // gets a new array reference on every keystroke (it's a useMemo over the
  // query). We gate on the *index* actually changing via prevIndexRef so that
  // typing a query does not trigger a smooth-scroll — only ↑/↓/Enter navigation
  // does (issue #109). Mirrors the RuleEditor navigation pattern.
  const prevIndexRef = useRef(currentMatchIndex);
  useEffect(() => {
    if (prevIndexRef.current === currentMatchIndex) return;
    prevIndexRef.current = currentMatchIndex;
    if (!previewRef.current || matches.length === 0) return;
    const active = previewRef.current.querySelector<HTMLElement>(
      `[data-match-index="${currentMatchIndex}"]`,
    );
    if (active && typeof active.scrollIntoView === "function") {
      active.scrollIntoView({ block: "center", behavior: "smooth" });
    }
  }, [currentMatchIndex, matches]);

  // Sync gutter scroll position with the preview (issue #111). The gutter
  // is `overflow: hidden`, so its scrollbars never show — but setting
  // scrollTop still moves the inner content, which is how we make the
  // gutter track the visible range. Mirrors RuleEditor.tsx's
  // textarea→lineNumbers sync on scroll.
  const handlePreviewScroll = useCallback(() => {
    if (previewRef.current && gutterRef.current) {
      gutterRef.current.scrollTop = previewRef.current.scrollTop;
    }
  }, []);

  // Keyboard shortcuts: Cmd+F / Ctrl+F opens search bar; Esc outside the
  // search input closes it and clears the query. Mirrors RuleEditor.tsx:375-399.
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
        }
      }
    };

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [searchBarVisible]);

  const highlightedHtml = useMemo(
    () => (hostsContent ? highlightMatches(hostsContent, matches, currentMatchIndex) : ""),
    [hostsContent, matches, currentMatchIndex],
  );

  const handleClose = () => {
    setSearchBarVisible(false);
    setSearchQuery("");
  };

  const handlePrev = () => {
    if (matches.length === 0) return;
    setCurrentMatchIndex((i) => (i <= 0 ? matches.length - 1 : i - 1));
  };

  const handleNext = () => {
    if (matches.length === 0) return;
    setCurrentMatchIndex((i) => (i >= matches.length - 1 ? 0 : i + 1));
  };

  return (
    <div className="mhost-page">
      <header className="mhost-page-header">
        <h1 className="mhost-page-title">System Hosts</h1>
      </header>

      <div className={`card ${styles.hostsCard}`}>
        <h3 className="card-title">System Hosts Preview</h3>
        <div className={styles.searchRow}>
          <SearchBar
            visible={searchBarVisible}
            onClose={handleClose}
            query={searchQuery}
            onQueryChange={setSearchQuery}
            replaceText=""
            onReplaceTextChange={() => {}}
            matchCount={matches.length}
            currentMatchIndex={currentMatchIndex}
            onPrev={handlePrev}
            onNext={handleNext}
            onReplace={() => {}}
            onReplaceAll={() => {}}
            readOnly
          />
        </div>
        {hostsError ? (
          <div className="alert alert-error">{hostsError}</div>
        ) : hostsContent === null ? (
          <div className="loading">Loading...</div>
        ) : (
          <div
            className={styles.editorWrapper}
            data-testid="system-hosts-editor-wrapper"
          >
            {/* Line-number gutter (issue #111). Always rendered once the
                hosts content has loaded; the active match's line is
                visually emphasised via `lineNumberActive`. */}
            <div
              ref={gutterRef}
              className={styles.lineNumbers}
              data-testid="line-numbers"
              aria-hidden="true"
            >
              {lineNumbers.map((n) => (
                <div
                  key={n}
                  className={
                    n === activeLineNumber
                      ? `${styles.lineNumber} ${styles.lineNumberActive}`
                      : styles.lineNumber
                  }
                  data-testid="line-number"
                  data-line-number={n}
                >
                  {n}
                </div>
              ))}
            </div>
            <pre
              ref={previewRef}
              className={styles.hostsPreview}
              onScroll={handlePreviewScroll}
              dangerouslySetInnerHTML={{ __html: highlightedHtml }}
            />
          </div>
        )}
      </div>
    </div>
  );
}

export default SystemHosts;
