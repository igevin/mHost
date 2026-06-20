import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { Provider as JotaiProvider } from "jotai";
import { getDefaultStore } from "jotai";
import {
  isApplyingAtom,
} from "../../stores/profiles";

// Mock tauri invoke
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import ApplyConfirmDialog from "../../components/ApplyConfirmDialog";

function Wrapper({ children }: { children: React.ReactNode }) {
  return <JotaiProvider store={getDefaultStore()}>{children}</JotaiProvider>;
}

describe("ApplyConfirmDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(isApplyingAtom, false);
  });

  it("shows progress during apply", async () => {
    const store = getDefaultStore();
    store.set(isApplyingAtom, true);

    render(
      <Wrapper>
        <ApplyConfirmDialog open={true} onClose={vi.fn()} />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText(/applying/i)).toBeInTheDocument();
    });
  });

  it("shows success after apply", async () => {
    render(
      <Wrapper>
        <ApplyConfirmDialog open={true} onClose={vi.fn()} applyResult="success" />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText(/success/i)).toBeInTheDocument();
    });
  });

  it("shows error and rollback button on failure", async () => {
    render(
      <Wrapper>
        <ApplyConfirmDialog
          open={true}
          onClose={vi.fn()}
          applyResult="error"
          applyError="Permission denied"
          onRollback={vi.fn()}
        />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("Apply failed")).toBeInTheDocument();
      expect(screen.getByText("Permission denied")).toBeInTheDocument();
    });

    // Rollback button should be present
    expect(
      screen.getByRole("button", { name: /rollback/i }),
    ).toBeInTheDocument();
  });
});
