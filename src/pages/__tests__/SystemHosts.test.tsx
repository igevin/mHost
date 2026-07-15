import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
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

function renderSystemHosts() {
  return render(
    <BrowserRouter>
      <SystemHosts />
    </BrowserRouter>,
  );
}

async function waitForContent() {
  // Loading state shows "Loading..." until readSystemHosts resolves.
  expect(await screen.findByText(/127\.0\.0\.1 localhost/)).toBeInTheDocument();
}

describe("SystemHosts", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockReadSystemHosts.mockResolvedValue(SAMPLE_HOSTS);
  });

  it("renders hosts content after async load", async () => {
    renderSystemHosts();
    await waitForContent();
    // All four fixture lines should be visible in the preview.
    expect(screen.getByText(/192\.168\.1\.1 example\.com example\.org/)).toBeInTheDocument();
    expect(screen.getByText(/# A comment line/)).toBeInTheDocument();
    expect(screen.getByText(/10\.0\.0\.1 api\.local/)).toBeInTheDocument();
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

  it("shows 'Line N of M' indicator next to the search bar with the active match's line", async () => {
    renderSystemHosts();
    await waitForContent();
    // SAMPLE_HOSTS has 4 lines; "example" matches line 2 (0-indexed 1).
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "example" } });
    const indicator = screen.getByTestId("active-line-info");
    expect(indicator).toHaveTextContent("Line 2 of 4");
    expect(indicator.dataset.activeLine).toBe("2");
    expect(indicator.dataset.totalLines).toBe("4");
  });

  it("hides 'Line N of M' when the search bar is closed", async () => {
    renderSystemHosts();
    await waitForContent();
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "example" } });
    expect(screen.getByTestId("active-line-info")).toBeInTheDocument();
    // Esc outside the search input → bar closes, indicator disappears.
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByTestId("active-line-info")).not.toBeInTheDocument();
  });

  it("hides 'Line N of M' when the query has no matches", async () => {
    renderSystemHosts();
    await waitForContent();
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "nonexistent" } });
    expect(screen.queryByTestId("active-line-info")).not.toBeInTheDocument();
  });

  it("updates 'Line N of M' as the user navigates between matches", async () => {
    renderSystemHosts();
    await waitForContent();
    fireEvent.keyDown(document, { key: "f", metaKey: true });
    // "1" matches several lines (the digit appears in lines 1, 2, 4 in the fixture).
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "1" } });
    const indicator = screen.getByTestId("active-line-info");
    // First match: "127.0.0.1" on line 1.
    expect(indicator).toHaveTextContent(/^Line 1 of 4$/);
    fireEvent.click(screen.getByTestId("next-button"));
    // Second match: still on line 1 (the second "1" in "127.0.0.1" is on the same line).
    // We don't assert exact line — just that the indicator updated.
    const after = screen.getByTestId("active-line-info");
    expect(after).toBeInTheDocument();
    // When we narrow the query to a unique match on a different line, the indicator
    // updates accordingly.
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "api.local" } });
    // "api.local" is on line 4 of the fixture.
    expect(screen.getByTestId("active-line-info")).toHaveTextContent("Line 4 of 4");
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