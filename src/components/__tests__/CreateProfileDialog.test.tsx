import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import CreateProfileDialog from "../CreateProfileDialog";

describe("CreateProfileDialog", () => {
  it("creates on first click", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    render(
      <CreateProfileDialog open={true} onClose={vi.fn()} onCreate={onCreate} isLoading={false} />,
    );

    const input = screen.getByPlaceholderText("Profile name");
    await act(async () => {
      fireEvent.change(input, { target: { value: "test" } });
    });

    const createButton = screen.getByText("Create");
    await act(async () => {
      fireEvent.click(createButton);
    });

    expect(onCreate).toHaveBeenCalledTimes(1);
    expect(onCreate).toHaveBeenCalledWith("test");
  });

  it("does not call onCreate twice when double-clicked", async () => {
    const onCreate = vi.fn().mockImplementation(() => new Promise((resolve) => setTimeout(resolve, 100)));
    render(
      <CreateProfileDialog open={true} onClose={vi.fn()} onCreate={onCreate} isLoading={false} />,
    );

    const input = screen.getByPlaceholderText("Profile name");
    await act(async () => {
      fireEvent.change(input, { target: { value: "test" } });
    });

    const createButton = screen.getByText("Create");
    await act(async () => {
      fireEvent.click(createButton);
      fireEvent.click(createButton);
    });

    await waitFor(() => expect(onCreate).toHaveBeenCalledTimes(1));
  });

  it("closes on first Cancel click", async () => {
    const onClose = vi.fn();
    render(
      <CreateProfileDialog open={true} onClose={onClose} onCreate={vi.fn()} isLoading={false} />,
    );

    const cancelButton = screen.getByText("Cancel");
    await act(async () => {
      fireEvent.click(cancelButton);
    });

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("does not call onCreate with empty or whitespace-only input", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    render(
      <CreateProfileDialog open={true} onClose={vi.fn()} onCreate={onCreate} isLoading={false} />,
    );

    const input = screen.getByPlaceholderText("Profile name");
    const createButton = screen.getByText("Create");

    // Empty input – button is already disabled, but simulate click anyway
    await act(async () => {
      fireEvent.click(createButton);
    });
    expect(onCreate).not.toHaveBeenCalled();

    // Whitespace-only input
    await act(async () => {
      fireEvent.change(input, { target: { value: "   " } });
    });
    await act(async () => {
      fireEvent.click(createButton);
    });
    expect(onCreate).not.toHaveBeenCalled();
  });

  it("disables Cancel while creating", async () => {
    const onCreate = vi.fn().mockImplementation(() => new Promise((resolve) => setTimeout(resolve, 200)));
    render(
      <CreateProfileDialog open={true} onClose={vi.fn()} onCreate={onCreate} isLoading={false} />,
    );

    const input = screen.getByPlaceholderText("Profile name");
    await act(async () => {
      fireEvent.change(input, { target: { value: "test" } });
    });

    const createButton = screen.getByText("Create");
    await act(async () => {
      fireEvent.click(createButton);
    });

    // While creating, button should show "Creating..."
    expect(screen.getByText("Creating...")).toBeInTheDocument();

    // Cancel should be disabled during creation
    const cancelButton = screen.getByText("Cancel");
    expect(cancelButton).toBeDisabled();
  });

  it("triggers create on Enter key", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    render(
      <CreateProfileDialog open={true} onClose={vi.fn()} onCreate={onCreate} isLoading={false} />,
    );

    const input = screen.getByPlaceholderText("Profile name");
    await act(async () => {
      fireEvent.change(input, { target: { value: "test" } });
    });
    await act(async () => {
      fireEvent.keyDown(input, { key: "Enter", code: "Enter" });
    });

    expect(onCreate).toHaveBeenCalledTimes(1);
    expect(onCreate).toHaveBeenCalledWith("test");
  });

  it("re-enables the button after onCreate throws", async () => {
    const onCreate = vi.fn().mockRejectedValue(new Error("boom"));
    render(
      <CreateProfileDialog open={true} onClose={vi.fn()} onCreate={onCreate} isLoading={false} />,
    );

    const input = screen.getByPlaceholderText("Profile name");
    await act(async () => {
      fireEvent.change(input, { target: { value: "test" } });
    });

    const createButton = screen.getByText("Create");
    await act(async () => {
      fireEvent.click(createButton);
    });

    await waitFor(() => expect(screen.getByText("Create")).toBeInTheDocument());
    expect(createButton).not.toBeDisabled();
  });
});
