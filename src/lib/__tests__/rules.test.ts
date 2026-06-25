import { describe, it, expect } from "vitest";
import { countRealRules, isCommentOnly } from "../rules";
import type { HostRule } from "../../types";

function makeRule(overrides: Partial<HostRule> = {}): HostRule {
  return {
    id: "r1",
    ip: "127.0.0.1",
    domains: ["localhost"],
    enabled: true,
    comment: null,
    source: { type: "Manual" },
    ...overrides,
  };
}

describe("countRealRules", () => {
  it("returns 0 for an empty array", () => {
    expect(countRealRules([])).toBe(0);
  });

  it("returns 0 when all rules are comment-only", () => {
    const rules: HostRule[] = [
      makeRule({ ip: null }),
      makeRule({ ip: null }),
    ];
    expect(countRealRules(rules)).toBe(0);
  });

  it("returns correct count for all real rules", () => {
    const rules: HostRule[] = [
      makeRule({ ip: "10.0.0.1", domains: ["a.com"] }),
      makeRule({ ip: "10.0.0.2", domains: ["b.com"] }),
    ];
    expect(countRealRules(rules)).toBe(2);
  });

  it("excludes comment-only rules from mixed list", () => {
    const rules: HostRule[] = [
      makeRule({ ip: null }),
      makeRule({ ip: "10.0.0.1", domains: ["a.com"] }),
      makeRule({ ip: null }),
      makeRule({ ip: "10.0.0.2", domains: ["b.com"] }),
    ];
    expect(countRealRules(rules)).toBe(2);
  });

  it("handles ip undefined gracefully", () => {
    const rules: HostRule[] = [
      makeRule({ ip: undefined as unknown as null }),
      makeRule({ ip: "10.0.0.1", domains: ["a.com"] }),
    ];
    expect(countRealRules(rules)).toBe(1);
  });
});

describe("isCommentOnly", () => {
  it("returns true when ip is null", () => {
    expect(isCommentOnly(makeRule({ ip: null }))).toBe(true);
  });

  it("returns true when ip is undefined", () => {
    expect(isCommentOnly(makeRule({ ip: undefined as unknown as null }))).toBe(true);
  });

  it("returns false when ip has a value", () => {
    expect(isCommentOnly(makeRule({ ip: "10.0.0.1" }))).toBe(false);
  });
});