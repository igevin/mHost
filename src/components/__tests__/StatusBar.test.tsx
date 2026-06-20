import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Provider as JotaiProvider } from "jotai";
import { getDefaultStore } from "jotai";
import type { Profile, ApplyPlan } from "../../types";
import {
  profilesAtom,
  applyPlanAtom,
  isApplyingAtom,
} from "../../stores/profiles";

// Mock tauri invoke
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import StatusBar from "../../components/StatusBar";

function makeProfile(overrides: Partial<Profile> = {}): Profile {
  return {
    id: "p1",
    name: "dev-profile",
    description: null,
    enabled: true,
    protected: false,
    tags: [],
    rules: [],
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

function makePlan(overrides: Partial<ApplyPlan> = {}): ApplyPlan {
  return {
    rules: [],
    conflicts: [],
    diff: {
      added: ["127.0.0.1 new.local"],
      removed: [],
      unchanged: [],
    },
    backup_required: true,
    ...overrides,
  };
}

function Wrapper({ children }: { children: React.ReactNode }) {
  return <JotaiProvider store={getDefaultStore()}>{children}</JotaiProvider>;
}

describe("StatusBar (T3.3 enhanced)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(profilesAtom, []);
    store.set(applyPlanAtom, null);
    store.set(isApplyingAtom, false);
  });

  it("shows active profile name", () => {
    const profile = makeProfile();
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    expect(screen.getByText("dev-profile")).toBeInTheDocument();
  });

  it("shows 'None' when no profile enabled", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, []);

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    expect(screen.getByText("None")).toBeInTheDocument();
  });

  it("shows 'Applying...' when isApplying is true", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile()]);
    store.set(isApplyingAtom, true);

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    expect(screen.getByText("Applying...")).toBeInTheDocument();
  });

  it("shows 'Pending Changes' when apply plan has changes", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile()]);
    store.set(applyPlanAtom, makePlan());

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    expect(screen.getByText(/Pending Changes/i)).toBeInTheDocument();
  });

  it("does not show 'Pending Changes' when plan has no diff", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile()]);
    store.set(
      applyPlanAtom,
      makePlan({ diff: { added: [], removed: [], unchanged: [] } }),
    );

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    expect(screen.queryByText(/Pending Changes/i)).not.toBeInTheDocument();
  });

  it("shows Apply button when onApply is provided", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile()]);
    const onApply = vi.fn();

    render(
      <Wrapper>
        <StatusBar onApply={onApply} />
      </Wrapper>,
    );

    expect(
      screen.getByRole("button", { name: /apply changes/i }),
    ).toBeInTheDocument();
  });

  it("does not show Apply button when onApply is not provided", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile()]);

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    expect(
      screen.queryByRole("button", { name: /apply/i }),
    ).not.toBeInTheDocument();
  });

  it("calls onApply when Apply button is clicked", async () => {
    const user = userEvent.setup();
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile()]);
    const onApply = vi.fn();

    render(
      <Wrapper>
        <StatusBar onApply={onApply} />
      </Wrapper>,
    );

    const applyBtn = screen.getByRole("button", { name: /apply changes/i });
    await user.click(applyBtn);

    expect(onApply).toHaveBeenCalledTimes(1);
  });
});
