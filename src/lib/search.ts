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
 */
export function findMatches(text: string, query: string): MatchInfo[] {
  if (!query || !text) return [];
  const matches: MatchInfo[] = [];
  const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const regex = new RegExp(escaped, "gi");
  let match: RegExpExecArray | null;
  while ((match = regex.exec(text)) !== null) {
    const lineIndex = text.slice(0, match.index).split("\n").length - 1;
    matches.push({ start: match.index, end: match.index + match[0].length, lineIndex });
  }
  return matches;
}