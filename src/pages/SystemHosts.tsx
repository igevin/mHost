import { useState, useEffect, useRef, useMemo, useCallback } from "react";
import { useLocation } from "react-router-dom";
import { readSystemHosts } from "../lib/tauri";
import { extractErrorMessage } from "../lib/error";
import { findMatches } from "../lib/search";
import type { MatchInfo } from "../lib/search";
import { highlightHostsLine } from "../lib/highlightHosts";
import SearchBar from "../components/SearchBar";
import styles from "./SystemHosts.module.css";

/** 只读 hosts 预览的行高（px）—— 必须与 CSS 的
 *  `--editor-line-height-compact` 一致（定义在 src/styles/global.css，
 *  由 SystemHosts.module.css 的 .editorWrapper 应用给 gutter 与预览）。
 *
 *  issue #248: 不能再用 `12px * 1.6 = 19.2px`。WebKit 会把分数行盒向下
 *  取整成 19px，而 gutter 的行槽位（min-height）不取整 —— 两者逐行错开，
 *  行号会随行数漂移。显式的 19px 让「声明值 == 渲染值」。 */
export const HOSTS_PREVIEW_LINE_HEIGHT_PX = 19;

/**
 * Per-line hosts-syntax highlighting + search-match wrapping (issue #126).
 * Mirrors RuleEditor's `highlightText` exactly so a user inspecting
 * /etc/hosts sees the same IP / domain / comment colors as in the
 * profile editor. The active match gets a different class and a
 * `data-match-index` attribute so callers can scroll it into view.
 */
function highlightHosts(
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
        return highlightHostsLine(line);
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
          const segHtml = highlightHostsLine(seg.text);
          if (seg.isMatch) {
            const isActive = seg.matchIndex === activeMatchIndex;
            const cls = isActive ? styles.searchMatchActive : styles.searchMatch;
            return `<mark class="${cls}" data-match-index="${seg.matchIndex}">${segHtml}</mark>`;
          }
          return segHtml;
        })
        .join("");
    })
    .join("\n");
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
  //
  // issue #124: 命令面板跳转到 system hosts 匹配时带
  // `location.state.focusLine`（原文件物理行号）—— 没有搜索激活时
  // 用它驱动 gutter 高亮。
  // review F6: jumpId nonce —— 同一行的重复跳转也要重滚。
  const location = useLocation();
  const locationState = location.state as { focusLine?: number; jumpId?: number } | null;
  const focusLine = locationState?.focusLine ?? null;
  const jumpId = locationState?.jumpId ?? 0;
  const activeLineNumber =
    searchBarVisible &&
    matches.length > 0 &&
    currentMatchIndex < matches.length
      ? matches[currentMatchIndex].lineIndex + 1
      : focusLine;

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

  // issue #124: 命令面板跳转 —— focusLine 变化且内容加载完成后，把
  // pre 滚到目标行，gutter 同步滚动。jumpId 是重复跳转的 nonce（F6）。
  //
  // issue #248: 行高用真实渲染值（19px），不是 `12px × 1.6 = 19.2px` ——
  // 与 CSS 的 --editor-line-height-compact 保持一致，避免 gutter 与预览逐行漂移。
  useEffect(() => {
    if (focusLine == null || hostsContent === null) return;
    const top = (focusLine - 1) * HOSTS_PREVIEW_LINE_HEIGHT_PX;
    if (previewRef.current) previewRef.current.scrollTop = top;
    if (gutterRef.current) gutterRef.current.scrollTop = top;
  }, [focusLine, jumpId, hostsContent]);

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
    () => (hostsContent ? highlightHosts(hostsContent, matches, currentMatchIndex) : ""),
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
