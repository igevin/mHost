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
    // Query matching 4 lines (each has an IP).
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "1" } });
    // Move to last match.
    fireEvent.click(screen.getByTestId("next-button"));
    fireEvent.click(screen.getByTestId("next-button"));
    fireEvent.click(screen.getByTestId("next-button"));
    fireEvent.click(screen.getByTestId("next-button"));
    fireEvent.click(screen.getByTestId("next-button"));
    // Now narrow the query to one match; currentMatchIndex must clamp to 0.
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "localhost" } });
    expect(screen.getByText("1/1")).toBeInTheDocument();
  });
});