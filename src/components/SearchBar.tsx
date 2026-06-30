import { useState, useEffect, useRef, useCallback } from "react";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import styles from "./SearchBar.module.css";

export interface SearchBarProps {
  visible: boolean;
  onClose: () => void;
  query: string;
  onQueryChange: (q: string) => void;
  replaceText: string;
  onReplaceTextChange: (t: string) => void;
  matchCount: number;
  currentMatchIndex: number;
  onPrev: () => void;
  onNext: () => void;
  onReplace: () => void;
  onReplaceAll: () => void;
  readOnly?: boolean;
}

function SearchBar({
  visible,
  onClose,
  query,
  onQueryChange,
  replaceText,
  onReplaceTextChange,
  matchCount,
  currentMatchIndex,
  onPrev,
  onNext,
  onReplace,
  onReplaceAll,
  readOnly = false,
}: SearchBarProps) {
  const [replaceExpanded, setReplaceExpanded] = useState(false);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const { onPointerDown } = useWebKitPointerDown();
  // Separate guard for toggle to prevent double-fire from onClick + onPointerDown
  const toggleGuard = useWebKitPointerDown();

  useEffect(() => {
    if (visible) {
      searchInputRef.current?.focus();
      searchInputRef.current?.select();
    }
  }, [visible]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        onNext();
      } else if (e.key === "Enter" && e.shiftKey) {
        e.preventDefault();
        onPrev();
      } else if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    },
    [onNext, onPrev, onClose],
  );

  const handleToggleReplace = useCallback(() => {
    if (!toggleGuard.fire()) return;
    setReplaceExpanded((prev) => !prev);
    setTimeout(toggleGuard.release, 50);
  }, [toggleGuard]);

  const matchDisplay = matchCount > 0 ? `${currentMatchIndex + 1}/${matchCount}` : "0/0";

  if (!visible) return null;

  return (
    <div className={styles.searchBar} onKeyDown={handleKeyDown} data-search-bar="true">
      <div className={styles.searchRow}>
        <input
          ref={searchInputRef}
          type="text"
          className={styles.searchInput}
          placeholder="Find"
          value={query}
          onChange={(e) => onQueryChange(e.target.value)}
          data-testid="search-input"
        />
        <span className={styles.matchCount}>{matchDisplay}</span>
        <button
          className={styles.navButton}
          onClick={onPrev}
          onPointerDown={onPointerDown(onPrev)}
          title="Previous match (Shift+Enter)"
          data-testid="prev-button"
        >
          ↑
        </button>
        <button
          className={styles.navButton}
          onClick={onNext}
          onPointerDown={onPointerDown(onNext)}
          title="Next match (Enter)"
          data-testid="next-button"
        >
          ↓
        </button>
        {!readOnly && (
          <button
            className={`${styles.actionButton} ${replaceExpanded ? styles.actionButtonActive : ""}`}
            onClick={handleToggleReplace}
            onPointerDown={onPointerDown(handleToggleReplace)}
            data-testid="toggle-replace"
          >
            Replace
          </button>
        )}
        <button
          className={styles.closeButton}
          onClick={onClose}
          onPointerDown={onPointerDown(onClose)}
          title="Close (Esc)"
          data-testid="close-button"
        >
          ×
        </button>
      </div>
      {replaceExpanded && !readOnly && (
        <div className={styles.replaceRow}>
          <input
            type="text"
            className={styles.replaceInput}
            placeholder="Replace with"
            value={replaceText}
            onChange={(e) => onReplaceTextChange(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                onReplace();
              }
            }}
            data-testid="replace-input"
          />
          <button
            className={styles.actionButton}
            onClick={onReplace}
            onPointerDown={onPointerDown(onReplace)}
            disabled={matchCount === 0}
            data-testid="replace-button"
          >
            Replace
          </button>
          <button
            className={styles.actionButton}
            onClick={onReplaceAll}
            onPointerDown={onPointerDown(onReplaceAll)}
            disabled={matchCount === 0}
            data-testid="replace-all-button"
          >
            Replace All
          </button>
        </div>
      )}
    </div>
  );
}

export default SearchBar;
