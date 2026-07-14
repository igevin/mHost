import { useState, useEffect, useRef, useMemo } from "react";
import { readSystemHosts } from "../lib/tauri";
import { extractErrorMessage } from "../lib/error";
import { findMatches } from "../lib/search";
import SearchBar from "../components/SearchBar";
import styles from "./SystemHosts.module.css";

/** Escape HTML special chars for safe rendering via dangerouslySetInnerHTML. */
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

/**
 * Wrap case-insensitive literal matches in `<mark>` tags. No syntax
 * highlighting — SystemHosts is plain text, so each segment is just
 * escaped HTML with optional mark wrapping. The active match gets a
 * different class and a `data-match-index` attribute so callers can
 * scroll it into view.
 */
function highlightMatches(
  text: string,
  matches: { start: number; end: number }[],
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
  useEffect(() => {
    if (!previewRef.current || matches.length === 0) return;
    const active = previewRef.current.querySelector<HTMLElement>(
      `[data-match-index="${currentMatchIndex}"]`,
    );
    if (active && typeof active.scrollIntoView === "function") {
      active.scrollIntoView({ block: "center", behavior: "smooth" });
    }
  }, [currentMatchIndex, matches]);

  // Keyboard shortcuts: Cmd+F / Ctrl+F opens search bar; Esc outside the
  // search input closes it and clears the query. Mirrors RuleEditor.tsx:393-418.
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
        {hostsError ? (
          <div className="alert alert-error">{hostsError}</div>
        ) : hostsContent === null ? (
          <div className="loading">Loading...</div>
        ) : (
          <pre
            ref={previewRef}
            className={styles.hostsPreview}
            dangerouslySetInnerHTML={{ __html: highlightedHtml }}
          />
        )}
      </div>
    </div>
  );
}

export default SystemHosts;