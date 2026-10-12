/**
 * fzf-like subsequence fuzzy matcher (issue #124) — hand-rolled, no library.
 *
 * `fuzzyMatch("apiloc", "api.local")` matches: every query char appears in
 * order in the text. Score rewards contiguous runs and segment-boundary
 * hits (after `.` / `-` / `_` or at text start) and penalizes a late first
 * match. The Rust side (`commands/query.rs::fuzzy_score`) mirrors this for
 * domain matching; keep the scoring philosophy aligned.
 */
export interface FuzzyMatch {
  score: number;
  /** Matched character indices in the text (for <mark> highlighting). */
  positions: number[];
}

export function fuzzyMatch(query: string, text: string): FuzzyMatch | null {
  if (!query) return null;
  const q = query.toLowerCase();
  const t = text.toLowerCase();

  const positions: number[] = [];
  let cursor = 0;
  for (const qc of q) {
    const hit = t.indexOf(qc, cursor);
    if (hit === -1) return null;
    positions.push(hit);
    cursor = hit + 1;
  }

  let score = 0;
  for (let i = 0; i < positions.length; i++) {
    if (i > 0 && positions[i] === positions[i - 1] + 1) score += 10;
    const pos = positions[i];
    if (pos === 0 || ".-_".includes(t[pos - 1])) score += 5;
  }
  score -= positions[0];
  return { score, positions };
}

/**
 * Filter + rank `items` by fuzzy score against `getText(item)`.
 * Best score first; ties keep the input order (stable sort).
 */
export function fuzzyRank<T>(
  query: string,
  items: T[],
  getText: (item: T) => string,
): { item: T; match: FuzzyMatch }[] {
  if (!query.trim()) {
    return [];
  }
  return items
    .map((item, index) => ({ item, index, match: fuzzyMatch(query, getText(item)) }))
    .filter((e): e is { item: T; index: number; match: FuzzyMatch } => e.match !== null)
    .sort((a, b) => b.match.score - a.match.score || a.index - b.index)
    .map(({ item, match }) => ({ item, match }));
}
