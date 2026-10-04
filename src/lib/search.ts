/**
 * Single source of truth for case-insensitive literal search across text
 * views (RuleEditor, SystemHosts). Extracted from RuleEditor so that any
 * read-only or editable text viewer can reuse the same algorithm and
 * stay in sync as the project evolves.
 */

/** A single match of `query` within `text`. */
export interface MatchInfo {
  /** Start offset (inclusive) of the match in `text`. */
  start: number;
  /** End offset (exclusive) of the match in `text`. */
  end: number;
  /** Zero-based line index where the match starts. */
  lineIndex: number;
}

/**
 * Find all case-insensitive literal matches of `query` in `text`.
 *
 * The query is treated as a plain string — regex metacharacters are
 * escaped so "127.0.0.1" matches literally and is not interpreted as a
 * regex pattern.
 *
 * Returns an empty array if either argument is empty.
 *
 * Perf (P-F9, issue #226): the previous implementation derived
 * `lineIndex` per match via `text.slice(0, match.index).split("\n")` —
 * for every match it copied the entire prefix and allocated one string
 * per line, O(M×N) overall (a single-character query hitting every line
 * of a 10k-line profile did ~10⁸ char operations per keystroke). Now a
 * single `indexOf` walk advances a line cursor alongside the regex scan:
 * match offsets are strictly increasing, so the walk is O(N + M) with no
 * per-match allocation.
 */
export function findMatches(text: string, query: string): MatchInfo[] {
  if (!query || !text) return [];
  const matches: MatchInfo[] = [];
  const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const regex = new RegExp(escaped, "gi");
  let line = 0;
  let nextNewline = text.indexOf("\n");
  let match: RegExpExecArray | null;
  while ((match = regex.exec(text)) !== null) {
    const start = match.index;
    while (nextNewline !== -1 && nextNewline < start) {
      line++;
      nextNewline = text.indexOf("\n", nextNewline + 1);
    }
    matches.push({ start, end: start + match[0].length, lineIndex: line });
  }
  return matches;
}
