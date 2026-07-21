import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { BrowserRouter } from "react-router-dom";

const mockReadSystemHosts = vi.fn();

vi.mock("../../lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/tauri")>();
  return {
    ...actual,
    readSystemHosts: (..._args: unknown[]) => mockReadSystemHosts(),
  };
});

import SystemHosts from "../SystemHosts";

const SAMPLE_HOSTS = [
  "127.0.0.1 localhost",
  "192.168.1.1 example.com example.org",
  "# A comment line",
  "10.0.0.1 api.local",
].join("\n");

/** Returns the rendered <pre> innerHTML. After issue #126 each hosts line
 *  is split into token spans, so we assert against the HTML directly
 *  instead of looking for a single text node. */
function preHtml(): string {
  return (document.querySelector("pre") as HTMLPreElement | null)?.innerHTML ?? "";
}

function renderSystemHosts() {
  return render(
    <BrowserRouter>
      <SystemHosts />
    </BrowserRouter>,
  );
}

async function waitForContent() {
  // Loading state shows "Loading..." until readSystemHosts resolves.
  // Post-#126 we wait for any hosts-token span to appear, so this
  // helper works for both the default fixture and tests that mock
  // a different hosts file.
  await waitFor(() => {
    const html = preHtml();
    expect(html).toMatch(/hosts-token-/);
  });
}

describe("SystemHosts", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockReadSystemHosts.mockResolvedValue(SAMPLE_HOSTS);
  });

  it("renders hosts content after async load", async () => {
    renderSystemHosts();
    await waitForContent();
    // issue #126: all four fixture lines should produce the expected
    // token spans — IPs / domains / comments get dedicated classes.
    const html = preHtml();
    expect(html).toMatch(/hosts-token-ip[^>]*>192\.168\.1\.1</);
    expect(html).toMatch(/hosts-token-domain[^>]*>example\.com</);
    expect(html).toMatch(/hosts-token-domain[^>]*>example\.org</);
    expect(html).toMatch(/hosts-token-comment[^>]*># A comment line</);
    expect(html).toMatch(/hosts-token-ip[^>]*>10\.0\.0\.1</);
    expect(html).toMatch(/hosts-token-domain[^>]*>api\.local</);
  });

  // ---- issue #126: token colors in SystemHosts ----

  it("tokenises an IPv4 rule into ip + domain spans", async () => {
    mockReadSystemHosts.mockResolvedValue("10.0.0.1 api.local\n");
    renderSystemHosts();
    await waitForContent();
    const html = preHtml();
    expect(html).toContain('<span class="hosts-token-ip">10.0.0.1</span>');
    expect(html).toContain('<span class="hosts-token-domain">api.local</span>');
    // Single space between tokens is preserved as escaped text.
    expect(html).toContain(" ");
  });

  it("tokenises an IPv6 rule into an ip span + domain spans", async () => {
    // Whole-line `#`-prefix lines are treated as comments per the hosts
    // convention (and that takes precedence over the disabled-prefix
    // span path) — so we exercise the ip-only branch with a plain IPv6
    // rule instead of a "disabled" one.
    mockReadSystemHosts.mockResolvedValue("::1 ip6-localhost\n");
    renderSystemHosts();
    await waitForContent();
    const html = preHtml();
    expect(html).toContain('<span class="hosts-token-ip">::1</span>');
    expect(html).toContain('<span class="hosts-token-domain">ip6-localhost</span>');
  });

  it("tokenises an inline comment as a trailing comment span", async () => {
    mockReadSystemHosts.mockResolvedValue("127.0.0.1 localhost # local\n");
    renderSystemHosts();
    await waitForContent();
    const html = preHtml();
    expect(html).toContain('<span class="hosts-token-ip">127.0.0.1</span>');
    expect(html).toContain('<span class="hosts-token-domain">localhost</span>');
    expect(html).toContain('<span class="hosts-token-comment"> # local</span>');
  });

  it("tokenises a full-line comment as a single comment span", async () => {
    mockReadSystemHosts.mockResolvedValue("# header line\n");
    renderSystemHosts();
    await waitFor(() => {
      expect(preHtml()).toContain('<span class="hosts-token-comment"># header line</span>');
    });
  });

  it("escapes special characters in hosts content", async () => {
    mockReadSystemHosts.mockResolvedValue("127.0.0.1 <weird>.dev\n");
    renderSystemHosts();
    await waitFor(() => {
      const html = preHtml();
      expect(html).toContain("&lt;weird&gt;.dev");
      expect(html).not.toMatch(/<weird>/);
    });
  });

  it("renders error state when readSystemHosts rejects", async () => {
    mockReadSystemHosts.mockRejectedValue(new Error("boom"));
    renderSystemHosts();
    expect(await screen.findByText("boom")).toBeInTheDocument();
  });

  it("opens search bar on Cmd+F", async () => {
    renderSystemHosts();
    await waitForContent();
    expect(screen.queryByTestId("search-input")).not.toBeInTheDocument();
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    expect(screen.getByTestId("search-input")).toBeInTheDocument();
  });

  it("opens search bar on Ctrl+F (non-Mac)", async () => {
    renderSystemHosts();
    await waitForContent();
    fireEvent.keyDown(document, { key: "f", ctrlKey: true });
    expect(screen.getByTestId("search-input")).toBeInTheDocument();
  });

  it("updates match count as the query changes", async () => {
    renderSystemHosts();
    await waitForContent();
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "example" } });
    // SAMPLE_HOSTS has two matches: "example.com" and "example.org" → 1/2.
    expect(screen.getByText("1/2")).toBeInTheDocument();
  });

  it("renders one row per line in the gutter (issue #111)", async () => {
    renderSystemHosts();
    await waitForContent();
    // SAMPLE_HOSTS has 4 lines → 4 numbered rows.
    const rows = screen.getAllByTestId("line-number");
    expect(rows).toHaveLength(4);
    expect(rows.map((r) => r.dataset.lineNumber)).toEqual(["1", "2", "3", "4"]);
    // Row 1 should contain the literal "1", row 4 should contain "4".
    expect(rows[0]).toHaveTextContent("1");
    expect(rows[3]).toHaveTextContent("4");
  });

  it("highlights the active match's line in the gutter", async () => {
    renderSystemHosts();
    await waitForContent();
    // SAMPLE_HOSTS has 4 lines; "example" first appears on line 2.
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "example" } });

    const rows = screen.getAllByTestId("line-number");
    const line2 = rows.find((r) => r.dataset.lineNumber === "2");
    expect(line2).toBeDefined();
    expect(line2!.className).toMatch(/lineNumberActive/);

    // Other rows are NOT highlighted.
    const other = rows.filter((r) => r.dataset.lineNumber !== "2");
    for (const r of other) {
      expect(r.className).not.toMatch(/lineNumberActive/);
    }
  });

  it("removes the active-line gutter highlight when the search bar is closed", async () => {
    renderSystemHosts();
    await waitForContent();
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "example" } });
    const rows = screen.getAllByTestId("line-number");
    expect(rows.some((r) => r.className.includes("lineNumberActive"))).toBe(true);

    fireEvent.keyDown(document, { key: "Escape" });
    const afterClose = screen.getAllByTestId("line-number");
    expect(afterClose.some((r) => r.className.includes("lineNumberActive"))).toBe(false);
  });

  it("removes the active-line gutter highlight when the query has no matches", async () => {
    renderSystemHosts();
    await waitForContent();
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "nonexistent" } });
    const rows = screen.getAllByTestId("line-number");
    expect(rows.some((r) => r.className.includes("lineNumberActive"))).toBe(false);
  });

  it("updates the active-line gutter highlight as the user navigates between matches", async () => {
    renderSystemHosts();
    await waitForContent();
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    // "api.local" lives on line 4 — unique match → highlight is on row 4.
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "api.local" } });
    let rows = screen.getAllByTestId("line-number");
    const activeRowNumbers = () =>
      rows
        .filter((r) => r.className.includes("lineNumberActive"))
        .map((r) => r.dataset.lineNumber);
    expect(activeRowNumbers()).toEqual(["4"]);

    // Narrow the query to "1" — first match is on line 1.
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "1" } });
    rows = screen.getAllByTestId("line-number");
    expect(activeRowNumbers()).toEqual(["1"]);

    // Navigate to the next match (line 1 → could stay on 1 if the next "1"
    // is in the same line, but the *index* advanced — navigation itself is
    // the contract). The active row must remain the single highlighted row.
    fireEvent.click(screen.getByTestId("next-button"));
    rows = screen.getAllByTestId("line-number");
    expect(activeRowNumbers()).toHaveLength(1);
  });

  it("syncs gutter.scrollTop with the preview's scroll position", async () => {
    renderSystemHosts();
    await waitForContent();
    const pre = document.querySelector("pre") as HTMLPreElement;
    const gutter = screen.getByTestId("line-numbers");
    // jsdom doesn't implement scroll layout, so set scrollTop directly and
    // dispatch a scroll event — the component's onScroll handler must copy
    // the value across to the gutter.
    Object.defineProperty(pre, "scrollTop", { value: 42, configurable: true, writable: true });
    fireEvent.scroll(pre);
    expect((gutter as HTMLDivElement).scrollTop).toBe(42);
  });

  it("navigates prev/next with wrap-around", async () => {
    renderSystemHosts();
    await waitForContent();
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "example" } });

    // Initial: 1/2. Next → 2/2.
    fireEvent.click(screen.getByTestId("next-button"));
    expect(screen.getByText("2/2")).toBeInTheDocument();
    // Next again → wrap to 1/2.
    fireEvent.click(screen.getByTestId("next-button"));
    expect(screen.getByText("1/2")).toBeInTheDocument();
    // Prev → wrap to last (2/2).
    fireEvent.click(screen.getByTestId("prev-button"));
    expect(screen.getByText("2/2")).toBeInTheDocument();
  });

  it("renders <mark> with searchMatch + searchMatchActive classes", async () => {
    renderSystemHosts();
    await waitForContent();
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "example" } });

    const preview = document.querySelector("pre");
    expect(preview).not.toBeNull();
    // Both classes appear in the rendered HTML; the active match uses searchMatchActive
    // while inactive matches use searchMatch.
    expect(preview!.innerHTML).toMatch(/searchMatch/);
    expect(preview!.innerHTML).toMatch(/searchMatchActive/);
  });

  it("hides Replace controls in readOnly mode", async () => {
    renderSystemHosts();
    await waitForContent();
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    expect(screen.queryByTestId("toggle-replace")).not.toBeInTheDocument();
  });

  it("shows 0/0 when the query has no matches", async () => {
    renderSystemHosts();
    await waitForContent();
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "nonexistent" } });
    expect(screen.getByText("0/0")).toBeInTheDocument();
    // No <mark> elements should be rendered.
    const preview = document.querySelector("pre");
    expect(preview!.innerHTML).not.toMatch(/searchMatch/);
  });

  it("treats regex metacharacters as literals (e.g. 127.0.0.1)", async () => {
    renderSystemHosts();
    await waitForContent();
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "127.0.0.1" } });
    // The fixture contains "127.0.0.1" exactly once → 1/1.
    expect(screen.getByText("1/1")).toBeInTheDocument();
  });

  it("closes search bar and clears query on Escape outside the search input", async () => {
    renderSystemHosts();
    await waitForContent();
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "example" } });
    // Escape fired on document body (not inside the search bar) → close + clear.
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByTestId("search-input")).not.toBeInTheDocument();

    // Re-opening Cmd+F should produce an empty input.
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    expect(screen.getByTestId("search-input")).toHaveValue("");
  });

  it("closes search bar on Escape inside the search input (handled by SearchBar)", async () => {
    renderSystemHosts();
    await waitForContent();
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    const input = screen.getByTestId("search-input");
    fireEvent.change(input, { target: { value: "example" } });
    // Escape fired on the input element — SearchBar's onKeyDown handler closes the bar.
    fireEvent.keyDown(input, { key: "Escape" });
    expect(screen.queryByTestId("search-input")).not.toBeInTheDocument();
  });

  it("clamps currentMatchIndex when matches shrink (query edited shorter)", async () => {
    renderSystemHosts();
    await waitForContent();
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    // "example" matches "example.com" and "example.org" → 2 matches.
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "example" } });
    // Advance currentMatchIndex from 0 → 1 (the second match).
    fireEvent.click(screen.getByTestId("next-button"));
    // Narrow the query to a single match; currentMatchIndex 1 must clamp to 0.
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "localhost" } });
    expect(screen.getByText("1/1")).toBeInTheDocument();
  });

  it("renders the pre block (not the loading state) when hosts file is empty", async () => {
    mockReadSystemHosts.mockResolvedValue("");
    renderSystemHosts();
    // Wait one tick for the read to resolve and state to update.
    expect(await screen.findByText(/System Hosts Preview/)).toBeInTheDocument();
    // "Loading..." should be gone; <pre> should be present and empty.
    expect(screen.queryByText("Loading...")).not.toBeInTheDocument();
    const pre = document.querySelector("pre");
    expect(pre).not.toBeNull();
    expect(pre!.innerHTML).toBe("");
  });

  // Regression for issue #109: scrollIntoView must only fire when the active
  // match *index* changes (navigation), not on every keystroke. The `matches`
  // array gets a new reference on each query change, so the naive effect used
  // to re-fire the smooth-scroll animation while typing.
  it("does NOT call scrollIntoView while typing a query", async () => {
    // jsdom does not implement scrollIntoView, so define it as a mock. The
    // component guards on `typeof active.scrollIntoView === "function"`, which
    // this satisfies.
    const scrollSpy = vi.fn();
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = scrollSpy;
    try {
      renderSystemHosts();
      await waitForContent();
      fireEvent.keyDown(document, { key: "f", metaKey: true });
      // Type the query character-by-character. The active index stays at 0
      // throughout, so no scroll should occur.
      const input = screen.getByTestId("search-input");
      fireEvent.change(input, { target: { value: "e" } });
      fireEvent.change(input, { target: { value: "ex" } });
      fireEvent.change(input, { target: { value: "exa" } });
      fireEvent.change(input, { target: { value: "example" } });
      expect(scrollSpy).not.toHaveBeenCalled();
    } finally {
      Element.prototype.scrollIntoView = original;
    }
  });

  it("calls scrollIntoView when navigating with ↑ / ↓", async () => {
    const scrollSpy = vi.fn();
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = scrollSpy;
    try {
      renderSystemHosts();
      await waitForContent();
      fireEvent.keyDown(document, { key: "f", metaKey: true });
      // Two matches: "example.com" and "example.org". Typing must not scroll.
      fireEvent.change(screen.getByTestId("search-input"), {
        target: { value: "example" },
      });
      expect(scrollSpy).not.toHaveBeenCalled();
      // Navigating to the next match changes the index → scroll fires once.
      fireEvent.click(screen.getByTestId("next-button"));
      expect(scrollSpy).toHaveBeenCalledTimes(1);
      // Back to the previous match → another scroll.
      fireEvent.click(screen.getByTestId("prev-button"));
      expect(scrollSpy).toHaveBeenCalledTimes(2);
    } finally {
      Element.prototype.scrollIntoView = original;
    }
  });
});
