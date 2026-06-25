import type { HostRule } from "../types";

/**
 * Count only real hosts rules, excluding comment-only entries.
 */
export function countRealRules(rules: HostRule[]): number {
  return rules.filter((r) => r.ip !== null).length;
}

/**
 * Check if a rule is a comment-only entry (no IP).
 */
export function isCommentOnly(rule: HostRule): boolean {
  return rule.ip === null || rule.ip === undefined;
}