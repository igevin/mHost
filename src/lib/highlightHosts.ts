import { escapeHtml } from "./escape";

/**
 * Single source of truth for hosts-file syntax highlighting. Used by both
 * RuleEditor (editable) and SystemHosts (read-only) so the two views stay
 * visually consistent — issue #126.
 *
 * Produces an HTML string with `<span class="hosts-token-*">` wrappers.
 * The token colors are defined in `src/styles/hostsHighlight.css` so
 * both call sites share the same look without re-declaring the CSS.
 *
 * Lines are tokenised as:
 *   - empty / whitespace-only  → empty string
 *   - full-line comment        → whole line in `.hosts-token-comment`
 *   - rule with optional inline comment
 *       - leading `# ` (disabled rule prefix) → `.hosts-token-comment`
 *       - leading IPv4 / IPv6                  → `.hosts-token-ip`
 *       - whitespace segments                  → literal whitespace
 *       - domain tokens                        → `.hosts-token-domain`
 *       - inline ` # ...`                      → `.hosts-token-comment`
 */
export const HOSTS_TOKEN_CLASSES = {
  ip: "hosts-token-ip",
  domain: "hosts-token-domain",
  comment: "hosts-token-comment",
} as const;

/**
 * Tokenise a single hosts line into HTML spans. Pure function —
 * no DOM, no React, no module-scoped state — so it is trivially
 * unit-testable and safe to call from any rendering path.
 */
export function highlightHostsLine(line: string): string {
  const trimmed = line.trim();
  if (trimmed === "") return "";

  // Full-line comment (# with no leading text). The leading whitespace
  // (if any) is preserved so column alignment is not lost.
  if (trimmed.startsWith("#")) {
    return `<span class="${HOSTS_TOKEN_CLASSES.comment}">${escapeHtml(line)}</span>`;
  }

 let remaining = line;
  let html = "";

  // Disabled-prefix "# " at the very start of a rule (not a full-line
  // comment because there are characters after the "#").
  if (remaining.startsWith("# ")) {
    html += `<span class="${HOSTS_TOKEN_CLASSES.comment}">${escapeHtml("# ")}</span>`;
    remaining = remaining.slice(2);
  }

  // IP address — match IPv4 first so "::1" doesn't accidentally take
  // the IPv4 branch.
  const ipv4Match = remaining.match(/^(\d+\.\d+\.\d+\.\d+)/);
  if (ipv4Match) {
    html += `<span class="${HOSTS_TOKEN_CLASSES.ip}">${escapeHtml(ipv4Match[1])}</span>`;
    remaining = remaining.slice(ipv4Match[1].length);
  } else {
    const ipv6Match = remaining.match(/^([0-9a-fA-F:]+)/);
    if (ipv6Match && ipv6Match[1].includes(":")) {
      html += `<span class="${HOSTS_TOKEN_CLASSES.ip}">${escapeHtml(ipv6Match[1])}</span>`;
      remaining = remaining.slice(ipv6Match[1].length);
    }
  }

  // Inline comment " # ..." — keep the leading space as part of the rule
  // (rule body), only the " #" onward becomes the comment.
  const commentIdx = remaining.indexOf(" #");
  if (commentIdx >= 0) {
    const beforeComment = remaining.slice(0, commentIdx);
    const comment = remaining.slice(commentIdx);
    html += tokenizeHostsBody(beforeComment);
    html += `<span class="${HOSTS_TOKEN_CLASSES.comment}">${escapeHtml(comment)}</span>`;
  } else {
    html += tokenizeHostsBody(remaining);
  }

  return html;
}

/** Split a hosts-line body (already past IP + disabled prefix) into
 *  whitespace + domain-token spans. */
function tokenizeHostsBody(body: string): string {
  if (body === "") return "";
  const parts = body.split(/(\s+)/);
  let html = "";
  for (const part of parts) {
    if (part === "") continue;
 if (/^\s+$/.test(part)) {
      html += escapeHtml(part);
    } else {
      html += `<span class="${HOSTS_TOKEN_CLASSES.domain}">${escapeHtml(part)}</span>`;
    }
  }
  return html;
}
