import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { Provider as JotaiProvider } from "jotai";
import { getDefaultStore } from "jotai";
import type { ApplyPlan } from "../../types";
import {
  applyPlanAtom,
  isApplyingAtom,
  errorAtom,
} from "../../stores/profiles";

// Mock tauri invoke
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import ApplyConfirmDialog from "../../components/ApplyConfirmDialog";

function makePlan(overrides: Partial<ApplyPlan> = {}): ApplyPlan {
  return {
    rules: [
      {
        ip: "127.0.0.1",
        domain: "localhost",
        source_profile_id: "p1",
        source_profile_name: "dev-profile",
      },
    ],
    conflicts: [],
    diff: {
      added: ["127.0.0.1 app.local"],
      removed: ["192.168.1.1 old.local"],
      unchanged: ["127.0.0.1 localhost"],
    },
    backup_required: true,
    ...overrides,
  };
}

function Wrapper({ children }: { children: React.ReactNode }) {
  return <JotaiProvider store={getDefaultStore()}>{children}</JotaiProvider>;
}

describe("ApplyConfirmDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(applyPlanAtom, null);
    store.set(isApplyingAtom, false);
    store.set(errorAtom, null);
  });

  it("shows diff preview before applying", async () => {
    const store = getDefaultStore();
    store.set(applyPlanAtom, makePlan());

    render(
      <Wrapper>
        <ApplyConfirmDialog open={true} onClose={vi.fn()} />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("Apply Preview")).toBeInTheDocument();
    });

    // Should show added line
    expect(screen.getByText("127.0.0.1 app.local")).toBeInTheDocument();
    // Should show removed line
    expect(screen.getByText("192.168.1.1 old.local")).toBeInTheDocument();
    // Should show unchanged line
    expect(screen.getByText("127.0.0.1 localhost")).toBeInTheDocument();
  });

  it("shows added/removed/unchanged lines with color coding", async () => {
    const store = getDefaultStore();
    store.set(
      applyPlanAtom,
      makePlan({
        diff: {
          added: ["+ 127.0.0.1 new.local"],
          removed: ["- 192.168.1.1 old.local"],
          unchanged: ["  127.0.0.1 existing.local"],
        },
      }),
    );

    render(
      <Wrapper>
        <ApplyConfirmDialog open={true} onClose={vi.fn()} />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("Apply Preview")).toBeInTheDocument();
    });

    // Check that lines are rendered with appropriate classes
    const addedLine = screen.getByText("+ 127.0.0.1 new.local");
    expect(addedLine.className).toContain("diffAdded");

    const removedLine = screen.getByText("- 192.168.1.1 old.local");
    expect(removedLine.className).toContain("diffRemoved");

    // Leading spaces are normalized by testing-library, use a function matcher
    const unchangedLine = screen.getByText((_content, element) => {
      return element?.className.includes("diffUnchanged") ?? false;
    });
    expect(unchangedLine).toBeInTheDocument();
    expect(unchangedLine.className).toContain("diffUnchanged");
  });

  it("shows conflict warnings", async () => {
    const store = getDefaultStore();
    store.set(
      applyPlanAtom,
      makePlan({
        conflicts: [
          {
            domain: "example.com",
            rules: [
              {
                ip: "127.0.0.1",
                domain: "example.com",
                source_profile_id: "p1",
                source_profile_name: "dev-profile",
              },
              {
                ip: "192.168.1.1",
                domain: "example.com",
                source_profile_id: "p2",
                source_profile_name: "prod-profile",
              },
            ],
          },
        ],
      }),
    );

    render(
      <Wrapper>
        <ApplyConfirmDialog open={true} onClose={vi.fn()} />
      </Wrapper>,
    );

    await waitFor(() => {
      const dialogText = screen.getByText("Apply Preview").closest("[role='dialog']")!.textContent!;
      expect(dialogText).toContain("Conflict");
      expect(dialogText).toContain("example.com");
    });
  });

  it("blocks apply when conflicts exist", async () => {
    const store = getDefaultStore();
    store.set(
      applyPlanAtom,
      makePlan({
        conflicts: [
          {
            domain: "conflict.local",
            rules: [],
          },
        ],
      }),
    );

    render(
      <Wrapper>
        <ApplyConfirmDialog open={true} onClose={vi.fn()} />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("Apply Preview")).toBeInTheDocument();
    });

    // The confirm button should be disabled
    const confirmBtn = screen.getByRole("button", { name: /confirm/i });
    expect(confirmBtn).toBeDisabled();
  });

  it("shows progress during apply", async () => {
    const store = getDefaultStore();
    store.set(applyPlanAtom, makePlan());
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
    const store = getDefaultStore();
    store.set(applyPlanAtom, makePlan());
    // Simulate post-apply state: not applying, no error, plan still set
    store.set(isApplyingAtom, false);

    // We need a way to signal success. The dialog will use a local state.
    // Let's test by rendering with applyResult prop
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
    const store = getDefaultStore();
    store.set(applyPlanAtom, makePlan());
    store.set(isApplyingAtom, false);

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
