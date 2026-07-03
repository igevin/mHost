import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, act, fireEvent } from "@testing-library/react";
import ImportDialog from "../ImportDialog";
import type { Profile, ValidateResult } from "../../types";

// Mock tauri functions
const mockValidateHostsText = vi.fn();
const mockImportProfile = vi.fn();
vi.mock("../../lib/tauri", () => ({
  validateHostsText: (...args: unknown[]) => mockValidateHostsText(...args),
  importProfile: (...args: unknown[]) => mockImportProfile(...args),
}));

function makeProfile(overrides: Partial<Profile> = {}): Profile {
  return {
    id: "p-001",
    name: "test",
    description: null,
    enabled: false,
    protected: false,
    tags: [],
    rules: [],
    mode: "hosts",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

describe("ImportDialog", () => {
  const defaultProps = {
    open: true,
    onClose: vi.fn(),
    onImported: vi.fn(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("validates pasted hosts text before import", async () => {
    mockValidateHostsText.mockResolvedValue({
      rules: [{ id: "r1", ip: "127.0.0.1", domains: ["localhost"], enabled: true, comment: null, source: { type: "Manual" } }],
      errors: [],
      duplicates: [],
    } as ValidateResult);

    render(<ImportDialog {...defaultProps} />);

    // Enter a name
    const nameInput = screen.getByLabelText(/profile name/i);
    fireEvent.change(nameInput, { target: { value: "my-import" } });

    // Paste hosts text
    const textarea = screen.getByPlaceholderText(/paste/i);
    fireEvent.change(textarea, { target: { value: "127.0.0.1 localhost" } });

    // Advance debounce timer
    act(() => {
      vi.advanceTimersByTime(350);
    });

    // Flush async promise resolution
    await act(async () => {
      await vi.runAllTimersAsync();
    });

    // Should show preview with rule count
    expect(screen.getByText(/1 rule/)).toBeInTheDocument();

    // Confirm button should be enabled
    const confirmBtn = screen.getByRole("button", { name: /import/i });
    expect(confirmBtn).not.toBeDisabled();
  });

  it("shows errors for invalid hosts text", async () => {
    mockValidateHostsText.mockResolvedValue({
      rules: [],
      errors: [{ line_number: 1, error: "invalid format" }],
      duplicates: [],
    } as ValidateResult);

    render(<ImportDialog {...defaultProps} />);

    const nameInput = screen.getByLabelText(/profile name/i);
    fireEvent.change(nameInput, { target: { value: "bad-import" } });

    const textarea = screen.getByPlaceholderText(/paste/i);
    fireEvent.change(textarea, { target: { value: "invalid-line" } });

    act(() => {
      vi.advanceTimersByTime(350);
    });

    await act(async () => {
      await vi.runAllTimersAsync();
    });

    expect(screen.getByText(/Line 1.*invalid format/)).toBeInTheDocument();

    // Confirm button should be disabled when there are errors
    const confirmBtn = screen.getByRole("button", { name: /import/i });
    expect(confirmBtn).toBeDisabled();
  });

  it("creates profile on valid import", async () => {
    const importedProfile = makeProfile({ name: "my-import" });
    mockValidateHostsText.mockResolvedValue({
      rules: [{ id: "r1", ip: "10.0.0.1", domains: ["test.com"], enabled: true, comment: null, source: { type: "Manual" } }],
      errors: [],
      duplicates: [],
    } as ValidateResult);
    mockImportProfile.mockResolvedValue(importedProfile);

    const onImported = vi.fn();
    render(<ImportDialog {...defaultProps} onImported={onImported} />);

    const nameInput = screen.getByLabelText(/profile name/i);
    fireEvent.change(nameInput, { target: { value: "my-import" } });

    const textarea = screen.getByPlaceholderText(/paste/i);
    fireEvent.change(textarea, { target: { value: "10.0.0.1 test.com" } });

    act(() => {
      vi.advanceTimersByTime(350);
    });

    await act(async () => {
      await vi.runAllTimersAsync();
    });

    const confirmBtn = screen.getByRole("button", { name: /import/i });
    fireEvent.click(confirmBtn);

    await act(async () => {
      await vi.runAllTimersAsync();
    });

    expect(mockImportProfile).toHaveBeenCalledWith("my-import", "10.0.0.1 test.com");
    expect(onImported).toHaveBeenCalledWith(importedProfile);
  });

  it("disables confirm button when name is empty", () => {
    render(<ImportDialog {...defaultProps} />);

    const confirmBtn = screen.getByRole("button", { name: /import/i });
    expect(confirmBtn).toBeDisabled();
  });

  it("does not render when open is false", () => {
    render(<ImportDialog {...defaultProps} open={false} />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("calls onClose when cancel is clicked", () => {
    const onClose = vi.fn();
    render(<ImportDialog {...defaultProps} onClose={onClose} />);

    const cancelBtn = screen.getByRole("button", { name: /cancel/i });
    fireEvent.click(cancelBtn);

    expect(onClose).toHaveBeenCalled();
  });
});
