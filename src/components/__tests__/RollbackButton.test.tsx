import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Provider as JotaiProvider } from "jotai";
import { getDefaultStore } from "jotai";
import { errorAtom } from "../../stores/profiles";

// Mock tauri invoke
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import RollbackButton from "../../components/RollbackButton";

function Wrapper({ children }: { children: React.ReactNode }) {
  return <JotaiProvider store={getDefaultStore()}>{children}</JotaiProvider>;
}

describe("RollbackButton", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(errorAtom, null);
  });

  it("shows confirmation before rollback", async () => {
    const user = userEvent.setup();
    const onRollback = vi.fn();

    render(
      <Wrapper>
        <RollbackButton onRollback={onRollback} />
      </Wrapper>,
    );

    // Click the rollback button
    const rollbackBtn = screen.getByRole("button", { name: /rollback/i });
    await user.click(rollbackBtn);

    // Confirmation dialog should appear
    await waitFor(() => {
      expect(screen.getByText(/are you sure/i)).toBeInTheDocument();
    });

    // Cancel button should be present
    expect(
      screen.getByRole("button", { name: /cancel/i }),
    ).toBeInTheDocument();
  });

  it("rolls back on confirm", async () => {
    const user = userEvent.setup();
    const onRollback = vi.fn();

    render(
      <Wrapper>
        <RollbackButton onRollback={onRollback} />
      </Wrapper>,
    );

    // Click rollback button
    await user.click(screen.getByRole("button", { name: /rollback/i }));

    // Wait for confirmation
    await waitFor(() => {
      expect(screen.getByText(/are you sure/i)).toBeInTheDocument();
    });

    // Click confirm
    await user.click(screen.getByRole("button", { name: /confirm/i }));

    // onRollback should have been called
    expect(onRollback).toHaveBeenCalledTimes(1);
  });

  it("shows error when no backup exists", async () => {
    const user = userEvent.setup();
    const onRollback = vi.fn().mockRejectedValue(
      new Error("No backup available"),
    );

    render(
      <Wrapper>
        <RollbackButton onRollback={onRollback} />
      </Wrapper>,
    );

    // Click rollback button
    await user.click(screen.getByRole("button", { name: /rollback/i }));

    // Wait for confirmation
    await waitFor(() => {
      expect(screen.getByText(/are you sure/i)).toBeInTheDocument();
    });

    // Click confirm
    await user.click(screen.getByRole("button", { name: /confirm/i }));

    // Error should be shown
    await waitFor(() => {
      expect(screen.getByText(/no backup available/i)).toBeInTheDocument();
    });
  });
});
