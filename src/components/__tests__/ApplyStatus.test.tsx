import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { Provider as JotaiProvider } from "jotai";
import { getDefaultStore } from "jotai";
import type { Profile, ApplyPlan } from "../../types";
import { profilesAtom } from "../../stores/profiles";

// Mock tauri invoke
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
const mockInvoke = vi.mocked(invoke);

// Import component after mock setup
import ApplyStatus from "../../components/ApplyStatus";

function makeProfile(overrides: Partial<Profile> = {}): Profile {
  return {
    id: "p1",
    name: "dev-profile",
    description: "Dev hosts",
    enabled: true,
    protected: false,
    tags: ["dev"],
    rules: [
      {
        id: "r1",
        ip: "127.0.0.1",
        domains: ["localhost", "app.local"],
        enabled: true,
        comment: null,
        source: { type: "Manual" },
      },
    ],
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

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
      removed: [],
      unchanged: ["127.0.0.1 localhost"],
    },
    backup_required: true,
    ...overrides,
  };
}

function Wrapper({ children }: { children: React.ReactNode }) {
  return <JotaiProvider store={getDefaultStore()}>{children}</JotaiProvider>;
}

describe("ApplyStatus", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(profilesAtom, []);
  });

  it("shows active profile name and rules", async () => {
    const profile = makeProfile();
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_managed_block_content") return "127.0.0.1 localhost\n127.0.0.1 app.local\n";
      if (cmd === "get_last_applied") return "2024-06-15T10:30:00Z";
      if (cmd === "generate_apply_plan") return makePlan();
      return null;
    });

    render(
      <Wrapper>
        <ApplyStatus />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("dev-profile")).toBeInTheDocument();
    });

    // Rules should be shown
    await waitFor(() => {
      expect(screen.getByText("127.0.0.1")).toBeInTheDocument();
      expect(screen.getByText("localhost, app.local")).toBeInTheDocument();
    });
  });

  it("shows managed block content", async () => {
    const profile = makeProfile();
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_managed_block_content") return "127.0.0.1 localhost\n127.0.0.1 app.local\n";
      if (cmd === "get_last_applied") return "2024-06-15T10:30:00Z";
      if (cmd === "generate_apply_plan") return makePlan();
      return null;
    });

    render(
      <Wrapper>
        <ApplyStatus />
      </Wrapper>,
    );

    await waitFor(() => {
      const pre = screen.getByText("Managed Block in Hosts").closest(".card")?.querySelector("pre");
      expect(pre).toBeTruthy();
      expect(pre!.textContent).toContain("127.0.0.1 localhost");
      expect(pre!.textContent).toContain("127.0.0.1 app.local");
    });
  });

  it('shows "no active profile" when none enabled', async () => {
    const profile = makeProfile({ enabled: false });
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_managed_block_content") return null;
      if (cmd === "get_last_applied") return null;
      if (cmd === "generate_apply_plan") return makePlan();
      return null;
    });

    render(
      <Wrapper>
        <ApplyStatus />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("No active profile")).toBeInTheDocument();
    });
  });

  it("shows conflict warnings", async () => {
    const profile = makeProfile();
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    const planWithConflicts = makePlan({
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
    });

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_managed_block_content") return "127.0.0.1 localhost\n";
      if (cmd === "get_last_applied") return "2024-06-15T10:30:00Z";
      if (cmd === "generate_apply_plan") return planWithConflicts;
      return null;
    });

    render(
      <Wrapper>
        <ApplyStatus />
      </Wrapper>,
    );

    await waitFor(() => {
      // Conflict text is split across elements, so check for the domain name
      // which appears in a dedicated span
      const allText = screen.getByText("Apply History").closest(".card")!.textContent!;
      expect(allText).toContain("Conflict");
      expect(allText).toContain("example.com");
    });
  });

  it('shows "pending changes" indicator', async () => {
    const profile = makeProfile();
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    const planWithChanges = makePlan({
      diff: {
        added: ["127.0.0.1 new.local"],
        removed: ["192.168.1.1 old.local"],
        unchanged: ["127.0.0.1 localhost"],
      },
    });

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_managed_block_content") return "127.0.0.1 localhost\n";
      if (cmd === "get_last_applied") return "2024-06-15T10:30:00Z";
      if (cmd === "generate_apply_plan") return planWithChanges;
      return null;
    });

    render(
      <Wrapper>
        <ApplyStatus />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText(/pending changes/i)).toBeInTheDocument();
    });
  });

  it('shows "Last Applied" timestamp', async () => {
    const profile = makeProfile();
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_managed_block_content") return "127.0.0.1 localhost\n";
      if (cmd === "get_last_applied") return "2024-06-15T10:30:00Z";
      if (cmd === "generate_apply_plan") return makePlan();
      return null;
    });

    render(
      <Wrapper>
        <ApplyStatus />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText(/Last Applied/i)).toBeInTheDocument();
    });
  });

  it("renders RollbackButton in Apply History card", async () => {
    const profile = makeProfile();
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_managed_block_content") return "127.0.0.1 localhost\n";
      if (cmd === "get_last_applied") return "2024-06-15T10:30:00Z";
      if (cmd === "generate_apply_plan") return makePlan();
      return null;
    });

    render(
      <Wrapper>
        <ApplyStatus />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("Apply History")).toBeInTheDocument();
    });

    const applyHistoryCard = screen.getByText("Apply History").closest(".card");
    expect(applyHistoryCard).toBeTruthy();
    const rollbackBtn = applyHistoryCard!.querySelector("button");
    expect(rollbackBtn).toBeTruthy();
    expect(rollbackBtn!.textContent).toMatch(/rollback/i);
  });

  it("hides pending changes when plan fails", async () => {
    const profile = makeProfile();
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_managed_block_content") return "127.0.0.1 localhost\n";
      if (cmd === "get_last_applied") return "2024-06-15T10:30:00Z";
      if (cmd === "generate_apply_plan") throw new Error("plan failed");
      return null;
    });

    render(
      <Wrapper>
        <ApplyStatus />
      </Wrapper>,
    );

    // Wait for component to finish loading
    await waitFor(() => {
      expect(screen.getByText("dev-profile")).toBeInTheDocument();
    });

    // Should NOT show pending changes
    expect(screen.queryByText(/pending changes/i)).not.toBeInTheDocument();
  });
});
