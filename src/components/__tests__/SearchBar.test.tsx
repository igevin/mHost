import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import SearchBar from "../SearchBar";

function renderSearchBar(props: Partial<Parameters<typeof SearchBar>[0]> = {}) {
  const defaultProps = {
    visible: true,
    onClose: vi.fn(),
    query: "",
    onQueryChange: vi.fn(),
    replaceText: "",
    onReplaceTextChange: vi.fn(),
    matchCount: 0,
    currentMatchIndex: 0,
    onPrev: vi.fn(),
    onNext: vi.fn(),
    onReplace: vi.fn(),
    onReplaceAll: vi.fn(),
    readOnly: false,
  };
  return render(<SearchBar {...defaultProps} {...props} />);
}

describe("SearchBar", () => {
  it("renders nothing when not visible", () => {
    renderSearchBar({ visible: false });
    expect(screen.queryByTestId("search-input")).not.toBeInTheDocument();
  });

  it("renders search input and match count", () => {
    renderSearchBar({ query: "test", matchCount: 5, currentMatchIndex: 2 });
    expect(screen.getByTestId("search-input")).toHaveValue("test");
    expect(screen.getByText("3/5")).toBeInTheDocument();
  });

  it("calls onQueryChange when typing in search input", () => {
    const onQueryChange = vi.fn();
    renderSearchBar({ onQueryChange });
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "hello" } });
    expect(onQueryChange).toHaveBeenCalledWith("hello");
  });

  it("shows 0/0 when no matches", () => {
    renderSearchBar({ matchCount: 0, currentMatchIndex: 0 });
    expect(screen.getByText("0/0")).toBeInTheDocument();
  });

  it("calls onPrev when prev button clicked", () => {
    const onPrev = vi.fn();
    renderSearchBar({ onPrev, matchCount: 3, currentMatchIndex: 1 });
    fireEvent.click(screen.getByTestId("prev-button"));
    expect(onPrev).toHaveBeenCalled();
  });

  it("calls onNext when next button clicked", () => {
    const onNext = vi.fn();
    renderSearchBar({ onNext, matchCount: 3, currentMatchIndex: 1 });
    fireEvent.click(screen.getByTestId("next-button"));
    expect(onNext).toHaveBeenCalled();
  });

  it("calls onClose when close button clicked", () => {
    const onClose = vi.fn();
    renderSearchBar({ onClose });
    fireEvent.click(screen.getByTestId("close-button"));
    expect(onClose).toHaveBeenCalled();
  });

  it("toggles replace row when Replace button clicked", async () => {
    renderSearchBar();
    expect(screen.queryByTestId("replace-input")).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId("toggle-replace"));
    expect(screen.getByTestId("replace-input")).toBeInTheDocument();
    // Wait for toggle guard to release before second click
    await act(async () => {
      await new Promise((r) => setTimeout(r, 100));
    });
    fireEvent.click(screen.getByTestId("toggle-replace"));
    expect(screen.queryByTestId("replace-input")).not.toBeInTheDocument();
  });

  it("calls onReplaceTextChange when typing in replace input", () => {
    const onReplaceTextChange = vi.fn();
    renderSearchBar({ onReplaceTextChange });
    fireEvent.click(screen.getByTestId("toggle-replace"));
    fireEvent.change(screen.getByTestId("replace-input"), { target: { value: "replaced" } });
    expect(onReplaceTextChange).toHaveBeenCalledWith("replaced");
  });

  it("calls onReplace when Replace button clicked", () => {
    const onReplace = vi.fn();
    renderSearchBar({ onReplace, matchCount: 2 });
    fireEvent.click(screen.getByTestId("toggle-replace"));
    fireEvent.click(screen.getByTestId("replace-button"));
    expect(onReplace).toHaveBeenCalled();
  });

  it("calls onReplaceAll when Replace All button clicked", () => {
    const onReplaceAll = vi.fn();
    renderSearchBar({ onReplaceAll, matchCount: 2 });
    fireEvent.click(screen.getByTestId("toggle-replace"));
    fireEvent.click(screen.getByTestId("replace-all-button"));
    expect(onReplaceAll).toHaveBeenCalled();
  });

  it("disables Replace and Replace All when no matches", () => {
    renderSearchBar({ matchCount: 0 });
    fireEvent.click(screen.getByTestId("toggle-replace"));
    expect(screen.getByTestId("replace-button")).toBeDisabled();
    expect(screen.getByTestId("replace-all-button")).toBeDisabled();
  });

  it("hides replace functionality in readOnly mode", () => {
    renderSearchBar({ readOnly: true });
    expect(screen.queryByTestId("toggle-replace")).not.toBeInTheDocument();
  });

  it("calls onNext on Enter key", () => {
    const onNext = vi.fn();
    renderSearchBar({ onNext });
    fireEvent.keyDown(screen.getByTestId("search-input"), { key: "Enter", shiftKey: false });
    expect(onNext).toHaveBeenCalled();
  });

  it("calls onPrev on Shift+Enter key", () => {
    const onPrev = vi.fn();
    renderSearchBar({ onPrev });
    fireEvent.keyDown(screen.getByTestId("search-input"), { key: "Enter", shiftKey: true });
    expect(onPrev).toHaveBeenCalled();
  });

  it("calls onClose on Escape key", () => {
    const onClose = vi.fn();
    renderSearchBar({ onClose });
    fireEvent.keyDown(screen.getByTestId("search-input"), { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });
});
