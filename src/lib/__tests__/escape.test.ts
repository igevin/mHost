import { describe, it, expect } from "vitest";
import { escapeHtml } from "../escape";

describe("escapeHtml", () => {
  it("escapes & as &amp;", () => {
    expect(escapeHtml("a & b")).toBe("a &amp; b");
  });

  it("escapes < as &lt;", () => {
    expect(escapeHtml("<script>")).toBe("&lt;script&gt;");
  });

  it("escapes > as &gt;", () => {
    expect(escapeHtml("a > b")).toBe("a &gt; b");
  });

  it("escapes \" as &quot;", () => {
    // Required for safety when a value lands inside a quoted attribute
    // (e.g. href="${url}"). Without this, an attacker can break out of
    // the attribute and inject a new one (stored XSS).
    expect(escapeHtml('say "hi"')).toBe("say &quot;hi&quot;");
  });

  it("escapes ' as &#39;", () => {
    // Same rationale as above for single-quoted attributes. Numeric
    // entity is intentional — avoids the &apos; ambiguity in older HTML.
    expect(escapeHtml("it's")).toBe("it&#39;s");
  });

  it("escapes all five special characters together", () => {
    // Cover every replaced character in one input so the regex set
    // is exercised as a whole. Also exercises the order: `&` first so
    // the ampersands in the entities we emit are not double-encoded.
    expect(escapeHtml(`<a href="x" title='y'>&`)).toBe(
      "&lt;a href=&quot;x&quot; title=&#39;y&#39;&gt;&amp;",
    );
  });

  it("does not double-escape entities produced by itself", () => {
    // `&` must run first so the `&` in `&amp;`, `&lt;`, etc. is not
    // re-escaped. If order regressed, `&amp;` would become `&amp;amp;`.
    expect(escapeHtml("&amp;")).toBe("&amp;amp;");
    expect(escapeHtml("&lt;")).toBe("&amp;lt;");
  });

  it("returns empty string for empty input", () => {
    expect(escapeHtml("")).toBe("");
  });

  it("leaves plain text unchanged", () => {
    expect(escapeHtml("hello world")).toBe("hello world");
  });

  it("handles strings that contain no special characters", () => {
    const safe = "127.0.0.1 localhost.localdomain # comment";
    expect(escapeHtml(safe)).toBe(safe);
  });

  it("neutralises a script-tag injection attempt", () => {
    // The real-world shape that motivates this hardening: a user pastes
    // a hostile hosts entry (e.g. from an imported profile) and we
    // render it via dangerouslySetInnerHTML. The browser must see
    // inert text, not a live <script> element.
    const hostile = "<script>alert('xss')</script>";
    const escaped = escapeHtml(hostile);
    expect(escaped).not.toContain("<script>");
    expect(escaped).not.toContain("</script>");
    expect(escaped).toBe(
      "&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;",
    );
  });

  it("neutralises an attribute-break-out attempt", () => {
    // The exact failure mode called out in issue #110: a value
    // containing `"` would close the surrounding attribute and let the
    // attacker inject `onerror=` or similar. Verifying `"` is escaped
    // locks that fix in.
    const hostile = '" onerror="alert(1)';
    const escaped = escapeHtml(hostile);
    expect(escaped).not.toContain('"');
    expect(escaped).toBe("&quot; onerror=&quot;alert(1)");
  });
});