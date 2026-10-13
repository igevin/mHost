import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import ConfirmDialog from "../ConfirmDialog";

function renderDialog(props: Partial<Parameters<typeof ConfirmDialog>[0]> = {}) {
  const defaultProps = {
    open: true,
    title: "Delete Profile",
    message: "Delete this profile?",
    onConfirm: vi.fn(),
    onCancel: vi.fn(),
  };
  return {
    ...defaultProps,
    ...render(<ConfirmDialog {...defaultProps} {...props} />),
  };
}

describe("ConfirmDialog", () => {
  it("renders nothing when closed", () => {
    renderDialog({ open: false });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("renders title, message and default button labels", () => {
    renderDialog();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByText("Delete Profile")).toBeInTheDocument();
    expect(screen.getByText("Delete this profile?")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Confirm" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
  });

  it("calls onConfirm once per click (pointerdown + click dedupe)", () => {
    const onConfirm = vi.fn();
    renderDialog({ onConfirm });
    const confirmBtn = screen.getByRole("button", { name: "Confirm" });
    // Simulate the WebKit duplicate-event quirk: pointerdown then click.
    fireEvent.pointerDown(confirmBtn, { button: 0 });
    fireEvent.click(confirmBtn);
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it("calls onCancel when Cancel is clicked", () => {
    const onCancel = vi.fn();
    renderDialog({ onCancel });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("cancels on Escape", () => {
    const onCancel = vi.fn();
    renderDialog({ onCancel });
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("cancels on overlay click but not on dialog click", () => {
    const onCancel = vi.fn();
    renderDialog({ onCancel });
    fireEvent.click(screen.getByRole("dialog"));
    expect(onCancel).toHaveBeenCalledTimes(1);
    onCancel.mockClear();
    fireEvent.click(screen.getByText("Delete this profile?"));
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("uses custom labels and danger styling", () => {
    renderDialog({ confirmLabel: "Delete", danger: true });
    const confirmBtn = screen.getByRole("button", { name: "Delete" });
    expect(confirmBtn.className).toContain("btn-danger");
  });

  it("blocks cancel paths while loading", () => {
    const onCancel = vi.fn();
    renderDialog({ isLoading: true, loadingLabel: "Deleting...", onCancel });
    expect(screen.getByRole("button", { name: "Deleting..." })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
    fireEvent.keyDown(document, { key: "Escape" });
    fireEvent.click(screen.getByRole("dialog"));
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("renders children between message and actions", () => {
    renderDialog({ children: <div data-testid="extra">extra content</div> });
    expect(screen.getByTestId("extra")).toBeInTheDocument();
  });
});
