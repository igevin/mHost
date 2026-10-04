import { describe, it, expect } from "vitest";
import { findMatches } from "../search";

describe("findMatches", () => {
  it("returns an empty array for an empty query", () => {
    expect(findMatches("127.0.0.1 localhost", "")).toEqual([]);
  });

  it("returns an empty array for empty text", () => {
    expect(findMatches("", "localhost")).toEqual([]);
  });

  it("returns an empty array when there is no match", () => {
    expect(findMatches("127.0.0.1 localhost", "nosuchhost")).toEqual([]);
  });

  it("reports exact start/end offsets and zero-based lineIndex", () => {
    const text = "127.0.0.1 localhost";
    const matches = findMatches(text, "localhost");
    expect(matches).toHaveLength(1);
    expect(matches[0].start).toBe(10);
    expect(matches[0].end).toBe(19);
    expect(matches[0].lineIndex).toBe(0);
  });

  it("matches case-insensitively", () => {
    const text = "LOCALHOST localhost LocalHost";
    const matches = findMatches(text, "localhost");
    expect(matches.map((m) => m.start)).toEqual([0, 10, 20]);
  });

  it("treats regex metacharacters as literals", () => {
    const text = "127.0.0.1 example.com\n10.0.0.1 example.com";
    const matches = findMatches(text, "127.0.0.1");
    expect(matches).toHaveLength(1);
    expect(matches[0].start).toBe(0);
    expect(matches[0].lineIndex).toBe(0);
  });

  it("assigns correct lineIndex for matches on separate lines", () => {
    const text = "a.example.com\n127.0.0.1 b.example.com\n#c.example.com";
    const matches = findMatches(text, "example.com");
    expect(matches.map((m) => m.lineIndex)).toEqual([0, 1, 2]);
  });

  it("assigns the same lineIndex to multiple matches on one line", () => {
    const text = "aa.example.com bb.example.com";
    const matches = findMatches(text, "example.com");
    expect(matches.map((m) => m.start)).toEqual([3, 18]);
    expect(matches.every((m) => m.lineIndex === 0)).toBe(true);
  });

  it("counts lineIndex by the line where the match starts", () => {
    // A match cannot span lines with a plain query, but the contract is
    // documented as "line where the match starts" — pin it.
    const text = "line1\nline2\nline3";
    const matches = findMatches(text, "line");
    expect(matches.map((m) => m.lineIndex)).toEqual([0, 1, 2]);
  });

  it("handles a match at the very start of the text", () => {
    const matches = findMatches("abc\nabc", "abc");
    expect(matches.map((m) => m.start)).toEqual([0, 4]);
    expect(matches.map((m) => m.lineIndex)).toEqual([0, 1]);
  });

  it("is consistent with naive line counting on a large mixed text", () => {
    // Guards the O(N+M) line-cursor rewrite: every lineIndex must equal
    // what a per-match `slice(0, index).split("\n")` would produce.
    const lines: string[] = [];
    for (let i = 0; i < 500; i++) {
      lines.push(i % 2 === 0 ? `host${i}.example.com` : `# comment ${i} host.example.com`);
    }
    const text = lines.join("\n");
    const matches = findMatches(text, "example.com");
    expect(matches.length).toBe(500);
    for (const m of matches) {
      const naive = text.slice(0, m.start).split("\n").length - 1;
      expect(m.lineIndex).toBe(naive);
    }
  });

  it("keeps non-overlapping regex semantics for repeated matches", () => {
    // JS `RegExp.exec` with /g never returns overlapping matches — the
    // literal-search contract inherits that behavior.
    const matches = findMatches("aaaa", "aa");
    expect(matches.map((m) => m.start)).toEqual([0, 2]);
  });

  it("handles CRLF line endings like the old split-based logic", () => {
    // Old logic counted "\n" occurrences (split("\n")), so "\r" is part of
    // the line content and lineIndex is unaffected — pin that.
    const text = "a.example.com\r\nb.example.com";
    const matches = findMatches(text, "example.com");
    expect(matches.map((m) => m.lineIndex)).toEqual([0, 1]);
  });
});
