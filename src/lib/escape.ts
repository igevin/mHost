/**
 * Single source of truth for HTML escaping used by components that render
 * untrusted text via `dangerouslySetInnerHTML` (RuleEditor, SystemHosts).
 *
 * Extracted from per-file copies so that any new consumer can reuse the same
 * hardened implementation and so the security-relevant escaping logic only
 * lives in one place.
 *
 * Escapes the five characters that have HTML-special meaning. This is safe
 * for both:
 *   - text content (e.g. `<span>${escapeHtml(value)}</span>`)
 *   - quoted attribute values (e.g. `<a href="${escapeHtml(value)}">…`)
 *
 * NOT safe for: unquoted attribute values, `<script>` blocks, `<style>`
 * blocks, or JavaScript contexts (URLs, event handlers) — use a real
 * sanitizer for those.
 *
 * Order matters: `&` must be replaced first so the ampersands in the
 * entities we emit (`&amp;`, `&lt;`, `&gt;`, `&quot;`, `&#39;`) are not
 * themselves re-escaped into double-encoded output.
 */
export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}