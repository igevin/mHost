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
});
