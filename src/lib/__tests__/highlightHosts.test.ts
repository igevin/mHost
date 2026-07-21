import { describe, it, expect } from "vitest";
import { highlightHostsLine, HOSTS_TOKEN_CLASSES } from "../highlightHosts";

/** Shared class-name constants live in `HOSTS_TOKEN_CLASSES` so a UI
 *  consumer can compare against the literal expected values. */
const IP = HOSTS_TOKEN_CLASSES.ip;
const DOMAIN = HOSTS_TOKEN_CLASSES.domain;
const COMMENT = HOSTS_TOKEN_CLASSES.comment;

describe("highlightHostsLine (issue #126)", () => {
  it("returns empty string for an empty line", () => {
    expect(highlightHostsLine("")).toBe("");
  });

  it("returns empty string for whitespace-only line", () => {
    expect(highlightHostsLine("   ")).toBe("");
  });

  it("wraps a full-line comment in the comment class", () => {
    expect(highlightHostsLine("# header")).toBe(
      `<span class="${COMMENT}"># header</span>`,
    );
  });

  it("handles a full-line comment with leading whitespace", () => {
    expect(highlightHostsLine("   # indented comment")).toBe(
      `<span class="${COMMENT}">   # indented comment</span>`,
    );
  });

  it("tokenises an IPv4 + domain rule into ip + domain spans", () => {
    expect(highlightHostsLine("10.0.0.1 api.local")).toBe(
      `<span class="${IP}">10.0.0.1</span> <span class="${DOMAIN}">api.local</span>`,
    );
  });

  it("tokenises multiple domains separated by whitespace", () => {
    const out = highlightHostsLine("192.168.1.1 example.com example.org");
    expect(out).toContain(`<span class="${IP}">192.168.1.1</span>`);
    expect(out).toContain(`<span class="${DOMAIN}">example.com</span>`);
    expect(out).toContain(`<span class="${DOMAIN}">example.org</span>`);
  });

  it("tokenises an IPv6 address as the ip span", () => {
    const out = highlightHostsLine("::1 ip6-localhost");
    expect(out).toContain(`<span class="${IP}">::1</span>`);
    expect(out).toContain(`<span class="${DOMAIN}">ip6-localhost</span>`);
  });

  it("splits an inline \" #\" comment into a trailing comment span", () => {
    const out = highlightHostsLine("127.0.0.1 localhost # local");
    expect(out).toContain(`<span class="${IP}">127.0.0.1</span>`);
    expect(out).toContain(`<span class="${DOMAIN}">localhost</span>`);
    expect(out).toContain(`<span class="${COMMENT}"> # local</span>`);
  });

  it("HTML-escapes the original text content of every token", () => {
    // Important: `<` must not become an actual tag. The IP/domain/
    // comment classes must remain literal class names so they match
    // what `hostsHighlight.css` declares.
    const out = highlightHostsLine("127.0.0.1 <weird>.dev # <script>");
    expect(out).toContain("&lt;weird&gt;.dev");
    expect(out).toContain("&lt;script&gt;");
    expect(out).not.toMatch(/<weird>/);
  });

  it("emits only the class names declared in HOSTS_TOKEN_CLASSES", () => {
    // Sanity check: every class attribute the helper produces is one
    // of the contract class names that SystemHosts / RuleEditor rely
    // on for styling. Catches accidental drift if the helper ever
    // gains a new token category.
    const out = highlightHostsLine("127.0.0.1 localhost # local");
    const matched = out.match(/class="([^"]+)"/g) ?? [];
    const names = new Set(matched.map((m) => m.replace(/class="|"/g, "")));
    expect(names.has(HOSTS_TOKEN_CLASSES.ip)).toBe(true);
    expect(names.has(HOSTS_TOKEN_CLASSES.domain)).toBe(true);
    expect(names.has(HOSTS_TOKEN_CLASSES.comment)).toBe(true);
    // No unexpected class names.
    for (const n of names) {
      expect(Object.values(HOSTS_TOKEN_CLASSES)).toContain(n);
    }
  });
});
