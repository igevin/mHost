import { describe, it, expect } from "vitest";
import { fuzzyMatch, fuzzyRank } from "../fuzzy";

describe("fuzzyMatch (issue #124)", () => {
  it("matches a plain substring", () => {
    const m = fuzzyMatch("api", "api.local");
    expect(m).not.toBeNull();
    expect(m!.positions).toEqual([0, 1, 2]);
  });

  it("matches a subsequence (fzf-style)", () => {
    const m = fuzzyMatch("apiloc", "api.local");
    expect(m).not.toBeNull();
    expect(m!.positions).toEqual([0, 1, 2, 4, 5, 6]);
  });

  it("is case-insensitive", () => {
    expect(fuzzyMatch("API", "api.local")).not.toBeNull();
    expect(fuzzyMatch("api", "API.LOCAL")).not.toBeNull();
  });

  it("returns null when the query is not a subsequence", () => {
    expect(fuzzyMatch("zzz", "api.local")).toBeNull();
    expect(fuzzyMatch("localapi", "api.local")).toBeNull();
  });

  it("returns null for an empty query", () => {
    expect(fuzzyMatch("", "api.local")).toBeNull();
  });

  it("scores contiguous matches above sparse ones", () => {
    const contiguous = fuzzyMatch("api", "api.local")!;
    const sparse = fuzzyMatch("ail", "alpha.internal.local")!;
    expect(contiguous.score).toBeGreaterThan(sparse.score);
  });

  it("rewards matches at segment boundaries", () => {
    // `loc` 在 "api.local" 中命中段首（. 后），在 "api.xloc" 中是段中。
    const boundary = fuzzyMatch("loc", "api.local")!;
    const midSegment = fuzzyMatch("loc", "api.xloc")!;
    expect(boundary.score).toBeGreaterThan(midSegment.score);
  });

  it("penalizes a late first match", () => {
    const early = fuzzyMatch("api", "api.local")!;
    const late = fuzzyMatch("api", "xxxxapi.local")!;
    expect(early.score).toBeGreaterThan(late.score);
  });
});

describe("fuzzyRank", () => {
  it("filters non-matches and sorts best-first", () => {
    const items = ["Open settings", "Create new profile", "Switch to DNS mode"];
    const ranked = fuzzyRank("set", items, (s) => s);
    expect(ranked.map((r) => r.item)).toEqual(["Open settings"]);
  });

  it("ranks contiguous hits above sparse hits", () => {
    const ranked = fuzzyRank("api", ["x.p.i sparse", "api local"], (s) => s);
    expect(ranked[0].item).toBe("api local");
  });

  it("keeps input order on score ties (stable)", () => {
    const ranked = fuzzyRank("ab", ["ab-1", "ab-2"], (s) => s);
    expect(ranked.map((r) => r.item)).toEqual(["ab-1", "ab-2"]);
  });

  it("returns an empty list for a blank query", () => {
    expect(fuzzyRank("  ", ["a", "b"], (s) => s)).toEqual([]);
  });
});
