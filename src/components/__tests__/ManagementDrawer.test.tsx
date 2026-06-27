import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { BrowserRouter } from "react-router-dom";
import { getDefaultStore, Provider as JotaiProvider } from "jotai";
import type { Profile } from "../../types";
import { profilesAtom } from "../../stores/profiles";

// Mock tauri invoke
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(vi.fn()),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: vi.fn().mockResolvedValue("/tmp/export.txt"),
  confirm: vi.fn().mockResolvedValue(true),
  open: vi.fn().mockResolvedValue("/tmp/import.txt"),
}));

import ManagementDrawer from "../ManagementDrawer";

function makeProfile(overrides: Partial<Profile> = {}): Profile {
  return {
    id: "p1",
    name: "dev-profile",
    description: "Development hosts profile",
    enabled: true,
    protected: false,
    tags: ["dev", "local"],
    rules: [
      { id: "r1", ip: "127.0.0.1", domains: ["localhost"], enabled: true, comment: null, source: { type: "Manual" } },
      { id: "r2", ip: "::1", domains: ["localhost6"], enabled: true, comment: null, source: { type: "Manual" } },
    ],
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-06-15T10:30:00Z",
    ...overrides,
  };
}

function renderWithProviders(ui: React.ReactElement) {
  return render(
    <JotaiProvider store={getDefaultStore()}>
      <BrowserRouter>{ui}</BrowserRouter>
    </JotaiProvider>,
  );
}

describe("ManagementDrawer", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(profilesAtom, []);
  });

  // 1. 抽屉关闭时不渲染
  it("does not render when closed", () => {
    renderWithProviders(
      <ManagementDrawer open={false} onClose={vi.fn()} />,
    );
    expect(screen.queryByText("Profile Management")).not.toBeInTheDocument();
  });

  // 2. 抽屉打开时渲染标题和关闭按钮
  it("renders title and close button when open", () => {
    renderWithProviders(
      <ManagementDrawer open={true} onClose={vi.fn()} />,
    );
    expect(screen.getByText("Profile Management")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /close/i })).toBeInTheDocument();
  });

  // 3. 统计面板显示正确的 Total/Enabled/Rules 数量
  it("shows correct stats: Total Profiles, Enabled, Total Rules", () => {
    const profiles = [
      makeProfile({
        id: "p1",
        name: "dev",
        enabled: true,
        rules: [
          { id: "r1", ip: "127.0.0.1", domains: ["a.local"], enabled: true, comment: null, source: { type: "Manual" } },
          { id: "r2", ip: "127.0.0.1", domains: ["b.local"], enabled: true, comment: null, source: { type: "Manual" } },
        ],
      }),
      makeProfile({
        id: "p2",
        name: "staging",
        enabled: false,
        rules: [
          { id: "r3", ip: "192.168.1.1", domains: ["c.local"], enabled: true, comment: null, source: { type: "Manual" } },
        ],
      }),
      makeProfile({
        id: "p3",
        name: "prod",
        enabled: true,
        rules: [], // comment-only rules or empty
      }),
    ];
    const store = getDefaultStore();
    store.set(profilesAtom, profiles);

    renderWithProviders(
      <ManagementDrawer open={true} onClose={vi.fn()} />,
    );

    // Stats: 3 total, 2 enabled, 3 real rules (2 + 1 + 0)
    // Use within the statsGrid to avoid matching badge text
    const statsGrid = document.querySelector("[class*='statsGrid']");
    expect(statsGrid).toBeTruthy();

    // All three stat labels exist
    expect(statsGrid!.textContent).toContain("Total Profiles");
    expect(statsGrid!.textContent).toContain("Enabled");
    expect(statsGrid!.textContent).toContain("Total Rules");

    // Values are correct
    const statCards = statsGrid!.querySelectorAll("[class*='statCard']");
    expect(statCards).toHaveLength(3);

    // Total Profiles = 3
    expect(statCards[0].textContent).toContain("Total Profiles");
    expect(statCards[0].textContent).toContain("3");

    // Enabled = 2
    expect(statCards[1].textContent).toContain("Enabled");
    expect(statCards[1].textContent).toContain("2");

    // Total Rules = 3 (2 + 1 + 0)
    expect(statCards[2].textContent).toContain("Total Rules");
    expect(statCards[2].textContent).toContain("3");
  });

  // 4. 渲染所有 Profile 卡片
  it("renders all profile cards", () => {
    const profiles = [
      makeProfile({ id: "p1", name: "dev", tags: [] }),
      makeProfile({ id: "p2", name: "staging", tags: [] }),
    ];
    const store = getDefaultStore();
    store.set(profilesAtom, profiles);

    renderWithProviders(
      <ManagementDrawer open={true} onClose={vi.fn()} />,
    );

    // Profile names appear in h3.profileName elements
    const nameHeaders = screen.getAllByRole("heading", { level: 3 });
    const names = nameHeaders.map((h) => h.textContent);
    expect(names).toContain("dev");
    expect(names).toContain("staging");
  });

  // 5. 点击关闭按钮关闭抽屉
  it("calls onClose when close button is clicked", () => {
    const onClose = vi.fn();
    renderWithProviders(
      <ManagementDrawer open={true} onClose={onClose} />,
    );

    fireEvent.click(screen.getByRole("button", { name: /close/i }));
    expect(onClose).toHaveBeenCalledOnce();
  });

  // 6. 点击遮罩层关闭抽屉
  it("calls onClose when overlay backdrop is clicked", () => {
    const onClose = vi.fn();
    renderWithProviders(
      <ManagementDrawer open={true} onClose={onClose} />,
    );

    // The overlay is the first child in the portal container
    const overlay = document.querySelector("[class*='drawerOverlay']");
    expect(overlay).toBeTruthy();
    if (overlay) {
      fireEvent.click(overlay);
      expect(onClose).toHaveBeenCalledOnce();
    }
  });

  // 7. "+ New Profile" 按钮存在
  it("renders '+ New Profile' button", () => {
    renderWithProviders(
      <ManagementDrawer open={true} onClose={vi.fn()} />,
    );
    expect(screen.getByText("+ New Profile")).toBeInTheDocument();
  });

  // 8. Import 按钮存在
  it("renders 'Import' button", () => {
    renderWithProviders(
      <ManagementDrawer open={true} onClose={vi.fn()} />,
    );
    expect(screen.getByText("Import")).toBeInTheDocument();
  });

  // 9. Profile 卡片显示名称、描述、标签
  it("displays profile name, description and tags on cards", () => {
    const profiles = [
      makeProfile({
        id: "p1",
        name: "my-profile",
        description: "A test profile for development",
        tags: ["dev", "local"],
        rules: [],
      }),
    ];
    const store = getDefaultStore();
    store.set(profilesAtom, profiles);

    renderWithProviders(
      <ManagementDrawer open={true} onClose={vi.fn()} />,
    );

    expect(screen.getByText("my-profile")).toBeInTheDocument();
    expect(screen.getByText("A test profile for development")).toBeInTheDocument();
    expect(screen.getByText("dev")).toBeInTheDocument();
    expect(screen.getByText("local")).toBeInTheDocument();
  });

  // 10. Delete 按钮存在
  it("renders Delete button on profile cards", () => {
    const profiles = [
      makeProfile({ id: "p1", name: "dev", protected: false, rules: [] }),
    ];
    const store = getDefaultStore();
    store.set(profilesAtom, profiles);

    renderWithProviders(
      <ManagementDrawer open={true} onClose={vi.fn()} />,
    );

    const deleteButtons = screen.getAllByRole("button", { name: /delete/i });
    expect(deleteButtons.length).toBeGreaterThanOrEqual(1);
  });

  // 11. Toggle button shows correct label based on profile enabled state
  it("shows Disable button for enabled profile and Enable button for disabled profile", () => {
    const profiles = [
      makeProfile({ id: "p1", name: "dev", enabled: true, rules: [] }),
      makeProfile({ id: "p2", name: "staging", enabled: false, rules: [] }),
    ];
    const store = getDefaultStore();
    store.set(profilesAtom, profiles);

    renderWithProviders(
      <ManagementDrawer open={true} onClose={vi.fn()} />,
    );

    // Enabled profile shows "Disable" button
    expect(screen.getByRole("button", { name: /disable/i })).toBeInTheDocument();
    // Disabled profile shows "Enable" button
    expect(screen.getByRole("button", { name: /enable/i })).toBeInTheDocument();
  });

  // 12. Delete button click triggers confirm dialog
  it("calls confirm dialog when Delete button is clicked", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    const profiles = [
      makeProfile({ id: "p1", name: "dev", protected: false, rules: [] }),
    ];
    const store = getDefaultStore();
    store.set(profilesAtom, profiles);

    renderWithProviders(
      <ManagementDrawer open={true} onClose={vi.fn()} />,
    );

    const deleteButton = screen.getByRole("button", { name: /delete/i });
    await act(async () => {
      fireEvent.click(deleteButton);
    });

    expect(confirm).toHaveBeenCalledWith("Delete this profile?");
  });
});
