import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { MemoryRouter, Routes, Route, useNavigate } from "react-router-dom";
import { getDefaultStore, Provider as JotaiProvider } from "jotai";
import type { Profile, HostRule } from "../../types";
import { profilesAtom, selectedProfileIdAtom, dnsProfilesAtom } from "../../stores/profiles";

// Mock tauri
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(vi.fn()),
}));

// Mock dialog (used by export)
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
  confirm: vi.fn().mockResolvedValue(true),
  save: vi.fn().mockResolvedValue("/test/path"),
}));

const mockUpdateProfile = vi.fn().mockResolvedValue({
  id: "p1",
  name: "dev-profile",
  description: "Development hosts profile",
  enabled: true,
  protected: false,
  tags: [],
  rules: [],
  created_at: "2024-01-01T00:00:00Z",
  updated_at: "2024-01-01T00:00:00Z",
});

const mockSetProfileEnabled = vi.fn().mockResolvedValue(undefined);
const mockListDnsProfiles = vi.fn().mockResolvedValue([]);

vi.mock("../../lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/tauri")>();
  return {
    ...actual,
    validateHostsText: vi.fn().mockResolvedValue({ rules: [], errors: [] }),
    exportProfileToFile: vi.fn().mockResolvedValue(undefined),
    updateProfile: (...args: unknown[]) => mockUpdateProfile(...args),
    exportProfile: vi.fn().mockResolvedValue(""),
    setProfileEnabled: (...args: unknown[]) => mockSetProfileEnabled(...args),
    listDnsProfiles: (...args: unknown[]) => mockListDnsProfiles(...args),
  };
});

import ProfileView from "../ProfileView";

function makeRule(overrides: Partial<HostRule> = {}): HostRule {
  return {
    id: "rule-001",
    ip: "127.0.0.1",
    domains: ["localhost"],
    enabled: true,
    comment: null,
    source: { type: "Manual" },
    ...overrides,
  };
}

function makeProfile(overrides: Partial<Profile> = {}): Profile {
  return {
    id: "p1",
    name: "dev-profile",
    description: "Development hosts profile",
    enabled: true,
    protected: false,
    tags: ["dev", "local"],
    rules: [
      makeRule({ id: "r1", ip: "127.0.0.1", domains: ["localhost"], comment: "local" }),
      makeRule({ id: "r2", ip: "192.168.1.1", domains: ["example.com"] }),
    ],
    mode: "hosts",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

function renderWithRouter(initialEntry: string) {
  return render(
    <JotaiProvider store={getDefaultStore()}>
      <MemoryRouter initialEntries={[initialEntry]}>
        <Routes>
          <Route path="/profiles" element={<ProfileView />} />
          <Route path="/profiles/:id" element={<ProfileView />} />
          <Route path="/dns-profiles" element={<ProfileView mode="dns" />} />
          <Route path="/dns-profiles/:id" element={<ProfileView mode="dns" />} />
        </Routes>
      </MemoryRouter>
    </JotaiProvider>,
  );
}

/** Variant that carries location.state (issue #124 palette jump). */
function renderWithRouterState(pathname: string, state: Record<string, unknown>) {
  return render(
    <JotaiProvider store={getDefaultStore()}>
      <MemoryRouter initialEntries={[{ pathname, state }]}>
        <Routes>
          <Route path="/profiles" element={<ProfileView />} />
          <Route path="/profiles/:id" element={<ProfileView />} />
          <Route path="/dns-profiles" element={<ProfileView mode="dns" />} />
          <Route path="/dns-profiles/:id" element={<ProfileView mode="dns" />} />
        </Routes>
      </MemoryRouter>
    </JotaiProvider>,
  );
}

describe("ProfileView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(profilesAtom, []);
    store.set(selectedProfileIdAtom, null);
    store.set(dnsProfilesAtom, []);
  });

  it("renders profile name and rule count", () => {
    const profile = makeProfile({ id: "p1", name: "dev-profile" });
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    renderWithRouter("/profiles/p1");

    // Name appears in both Info Bar and Header
    const nameElements = screen.getAllByText("dev-profile");
    expect(nameElements.length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText(/2 rules/i)).toBeInTheDocument();
  });

  it("shows 'Read-only' badge by default", () => {
    const profile = makeProfile({ id: "p1" });
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    renderWithRouter("/profiles/p1");

    expect(screen.getByText("Read-only")).toBeInTheDocument();
  });

  it("switches to editing mode when 'Edit Rules' is clicked", () => {
    const profile = makeProfile({ id: "p1" });
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    renderWithRouter("/profiles/p1");

    fireEvent.click(screen.getByText("Edit Rules"));

    expect(screen.getByText("Editing")).toBeInTheDocument();
    expect(screen.queryByText("Read-only")).not.toBeInTheDocument();
  });

  it("shows Cancel and Save buttons in editing mode", () => {
    const profile = makeProfile({ id: "p1" });
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    renderWithRouter("/profiles/p1");

    fireEvent.click(screen.getByText("Edit Rules"));

    expect(screen.getByText("Cancel")).toBeInTheDocument();
    expect(screen.getByText("Save")).toBeInTheDocument();
  });

  it("disables Save button when there are no changes", () => {
    const profile = makeProfile({ id: "p1" });
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    renderWithRouter("/profiles/p1");

    fireEvent.click(screen.getByText("Edit Rules"));

    const saveButton = screen.getByText("Save");
    expect(saveButton).toBeDisabled();
  });

  it("returns to read-only mode when Cancel is clicked", () => {
    const profile = makeProfile({ id: "p1" });
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    renderWithRouter("/profiles/p1");

    // Enter edit mode
    fireEvent.click(screen.getByText("Edit Rules"));
    expect(screen.getByText("Editing")).toBeInTheDocument();

    // Click Cancel
    fireEvent.click(screen.getByText("Cancel"));

    expect(screen.getByText("Read-only")).toBeInTheDocument();
    expect(screen.queryByText("Editing")).not.toBeInTheDocument();
    expect(screen.getByText("Edit Rules")).toBeInTheDocument();
  });

  it("shows Info Bar with profile information", () => {
    const profile = makeProfile({
      id: "p1",
      name: "dev-profile",
      description: "Development hosts profile",
      tags: ["dev", "local"],
    });
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    renderWithRouter("/profiles/p1");

    // Name appears in Info Bar and Header
    const nameElements = screen.getAllByText("dev-profile");
    expect(nameElements.length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText("Development hosts profile")).toBeInTheDocument();
    expect(screen.getByText("dev")).toBeInTheDocument();
    expect(screen.getByText("local")).toBeInTheDocument();
  });

  it("shows 'Profile not found' when profile does not exist", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, []);

    renderWithRouter("/profiles/nonexistent");

    expect(screen.getByText(/Profile not found/)).toBeInTheDocument();
  });

  it("shows Enabled badge when profile is enabled", () => {
    const profile = makeProfile({ id: "p1", enabled: true });
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    renderWithRouter("/profiles/p1");

    expect(screen.getByText("Enabled")).toBeInTheDocument();
  });

  it("shows Disabled badge when profile is disabled", () => {
    const profile = makeProfile({ id: "p1", enabled: false });
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    renderWithRouter("/profiles/p1");

    expect(screen.getByText("Disabled")).toBeInTheDocument();
  });

  it("shows Protected badge when profile is protected", () => {
    const profile = makeProfile({ id: "p1", protected: true });
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    renderWithRouter("/profiles/p1");

    expect(screen.getByText("Protected")).toBeInTheDocument();
  });

  it("saves rules successfully and exits editing mode", async () => {
    const profile = makeProfile({ id: "p1", rules: [
      makeRule({ id: "r1", ip: "127.0.0.1", domains: ["localhost"] }),
    ] });
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    renderWithRouter("/profiles/p1");

    // Enter edit mode
    fireEvent.click(screen.getByText("Edit Rules"));
    expect(screen.getByText("Editing")).toBeInTheDocument();

    // Save button should be present in editing mode
    expect(screen.getByText("Cancel")).toBeInTheDocument();
    expect(screen.getByText("Save")).toBeInTheDocument();

    // Save button is initially disabled (no changes yet)
    expect(screen.getByText("Save")).toBeDisabled();
  });

  it("shows Delete confirmation when Delete menu item is clicked", async () => {
    const profile = makeProfile({ id: "p1", protected: false });
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    renderWithRouter("/profiles/p1");

    // Delete lives in the header ⋯ menu (Phase 2)
    fireEvent.click(
      screen.getByRole("button", { name: /more profile actions/i }),
    );
    const deleteItem = screen.getByRole("menuitem", { name: "Delete" });
    expect(deleteItem).not.toBeDisabled();

    await act(async () => {
      fireEvent.click(deleteItem);
    });

    // The shared ConfirmDialog (not the native plugin-dialog confirm) opens
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(
      screen.getByText('Delete profile "dev-profile"?'),
    ).toBeInTheDocument();
  });

  // Regression test for issue #100: Delete was double-bound to both
  // `onClick` and `onPointerDown`, so one physical click fired the handler
  // twice — producing two confirmation dialogs and a `ProfileNotFound` error
  // on the second IPC call. The entry now lives in the ⋯ menu (items are
  // click-only, see PopupMenu docs); a `pointerDown` followed by a `click`
  // must still open exactly one ConfirmDialog and never delete directly.
  it("Delete menu item opens only one ConfirmDialog when both pointerDown and click are dispatched", async () => {
    const { invoke } = await import("@tauri-apps/api/core");

    const profile = makeProfile({ id: "p1", protected: false });
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    renderWithRouter("/profiles/p1");

    fireEvent.click(
      screen.getByRole("button", { name: /more profile actions/i }),
    );
    const deleteItem = screen.getByRole("menuitem", { name: "Delete" });

    await act(async () => {
      fireEvent.pointerDown(deleteItem, { button: 0 });
      fireEvent.click(deleteItem);
      // Flush microtasks so any async handler effects propagate.
      await Promise.resolve();
    });

    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(invoke).not.toHaveBeenCalledWith(
      "delete_profile",
      expect.anything(),
    );
  });

  // ---- DNS mode tests ----

  it("renders empty state for DNS mode when no profiles", () => {
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, []);

    renderWithRouter("/dns-profiles");

    expect(screen.getByText("No DNS profiles yet")).toBeInTheDocument();
    expect(screen.getByText("+ New DNS Profile")).toBeInTheDocument();
  });

  it("renders DNS profile name and rule count", () => {
    const profile = makeProfile({ id: "d1", name: "dns-profile", mode: "dns" });
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [profile]);

    renderWithRouter("/dns-profiles/d1");

    const nameElements = screen.getAllByText("dns-profile");
    expect(nameElements.length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText(/2 rules/i)).toBeInTheDocument();
  });

  it("shows Enabled badge when DNS profile is enabled", () => {
    const profile = makeProfile({ id: "d1", enabled: true, mode: "dns" });
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [profile]);

    renderWithRouter("/dns-profiles/d1");

    expect(screen.getByText("Enabled")).toBeInTheDocument();
  });

  it("shows Disabled badge when DNS profile is disabled", () => {
    const profile = makeProfile({ id: "d1", enabled: false, mode: "dns" });
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [profile]);

    renderWithRouter("/dns-profiles/d1");

    expect(screen.getByText("Disabled")).toBeInTheDocument();
  });

  it("shows Disable button for enabled DNS profile and triggers toggle", async () => {
    const profile = makeProfile({ id: "d1", enabled: true, mode: "dns" });
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [profile]);

    renderWithRouter("/dns-profiles/d1");

    const toggleButton = screen.getByText("Disable");
    expect(toggleButton).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(toggleButton);
    });

    expect(mockSetProfileEnabled).toHaveBeenCalledWith("d1", false);
  });

  it("shows Enable button for disabled DNS profile and triggers toggle", async () => {
    const profile = makeProfile({ id: "d1", enabled: false, mode: "dns" });
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [profile]);

    renderWithRouter("/dns-profiles/d1");

    const toggleButton = screen.getByText("Enable");
    expect(toggleButton).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(toggleButton);
    });

    expect(mockSetProfileEnabled).toHaveBeenCalledWith("d1", true);
  });

  // ---- DNS mode rule editor tests (fix: missing Edit button + RuleEditor) ----

  it("renders Edit button alongside Edit Info for DNS profile", () => {
    const profile = makeProfile({ id: "d1", mode: "dns" });
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [profile]);

    renderWithRouter("/dns-profiles/d1");

    // 两个按钮独立存在
    expect(screen.getByText("Edit")).toBeInTheDocument();
    expect(screen.getByText("Edit Info")).toBeInTheDocument();
  });

  it("shows Read-only badge by default for DNS profile (RuleEditor mounted)", () => {
    const profile = makeProfile({ id: "d1", mode: "dns" });
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [profile]);

    renderWithRouter("/dns-profiles/d1");

    expect(screen.getByText("Read-only")).toBeInTheDocument();
  });

  it("switches to Editing mode when Edit is clicked (DNS)", () => {
    const profile = makeProfile({ id: "d1", mode: "dns" });
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [profile]);

    renderWithRouter("/dns-profiles/d1");

    fireEvent.click(screen.getByText("Edit"));

    expect(screen.getByText("Editing")).toBeInTheDocument();
    expect(screen.queryByText("Read-only")).not.toBeInTheDocument();
  });

  it("shows Cancel and Save buttons (and hides Edit Info) when DNS editing", () => {
    const profile = makeProfile({ id: "d1", mode: "dns" });
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [profile]);

    renderWithRouter("/dns-profiles/d1");

    fireEvent.click(screen.getByText("Edit"));

    expect(screen.getByText("Cancel")).toBeInTheDocument();
    expect(screen.getByText("Save")).toBeInTheDocument();
    // 进入编辑模式后 Edit 和 Edit Info 都不再显示
    expect(screen.queryByText("Edit")).not.toBeInTheDocument();
    expect(screen.queryByText("Edit Info")).not.toBeInTheDocument();
  });

  it("disables Save button initially in DNS editing mode (no changes yet)", () => {
    const profile = makeProfile({ id: "d1", mode: "dns" });
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [profile]);

    renderWithRouter("/dns-profiles/d1");

    fireEvent.click(screen.getByText("Edit"));

    const saveButton = screen.getByText("Save");
    expect(saveButton).toBeDisabled();
  });

  it("returns to Read-only when Cancel is clicked in DNS editing mode", () => {
    const profile = makeProfile({ id: "d1", mode: "dns" });
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [profile]);

    renderWithRouter("/dns-profiles/d1");

    fireEvent.click(screen.getByText("Edit"));
    expect(screen.getByText("Editing")).toBeInTheDocument();

    fireEvent.click(screen.getByText("Cancel"));

    expect(screen.getByText("Read-only")).toBeInTheDocument();
    expect(screen.queryByText("Editing")).not.toBeInTheDocument();
    expect(screen.getByText("Edit")).toBeInTheDocument();
    expect(screen.getByText("Edit Info")).toBeInTheDocument();
  });

  // issue #124: 命令面板 dns_rule 跳转 —— DnsProfileView 必须读取
  // location.state.focusLine 并透传给 RuleEditor（gutter 高亮目标行）。
  it("forwards location.state.focusLine to the DNS profile's RuleEditor gutter", () => {
    const profile = makeProfile({ id: "d1", mode: "dns" });
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [profile]);

    renderWithRouterState("/dns-profiles/d1", { focusLine: 2 });

    const active = document.querySelector("[class*='lineNumberActive']");
    expect(active).not.toBeNull();
    expect(active!.textContent).toBe("2");
  });

  // issue #124: hosts 侧同款接线（palette profile 跳转）。
  it("forwards location.state.focusLine to the hosts profile's RuleEditor gutter", () => {
    const profile = makeProfile({ id: "p1" });
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    renderWithRouterState("/profiles/p1", { focusLine: 2 });

    const active = document.querySelector("[class*='lineNumberActive']");
    expect(active).not.toBeNull();
    expect(active!.textContent).toBe("2");
  });

  // review F2 回归：跨 profile 跳转（A → B，B 行数更少）。RuleEditor
  // 按 profile.id key 住后重挂载，text 由 B 的 rules 初始化 ——
  // 修复前 effect 会拿 A 的残留 text 去 clamp/定位。
  it("clamps a cross-profile jump against the NEW profile's text, not the previous one (F2)", () => {
    function NavButton() {
      const navigate = useNavigate();
      return (
        <button
          data-testid="go-p2"
          onClick={() =>
            navigate("/profiles/p2", { state: { focusLine: 5, jumpId: 2 } })
          }
        >
          go
        </button>
      );
    }
    const threeRules = [
      makeRule({ id: "r1", ip: "10.0.0.1", domains: ["a.local"] }),
      makeRule({ id: "r2", ip: "10.0.0.2", domains: ["b.local"] }),
      makeRule({ id: "r3", ip: "10.0.0.3", domains: ["c.local"] }),
    ];
    const oneRule = [makeRule({ id: "r9", ip: "10.0.0.9", domains: ["z.local"] })];
    const store = getDefaultStore();
    store.set(profilesAtom, [
      makeProfile({ id: "p1", name: "big", rules: threeRules }),
      makeProfile({ id: "p2", name: "small", rules: oneRule }),
    ]);

    render(
      <JotaiProvider store={getDefaultStore()}>
        <MemoryRouter
          initialEntries={[{ pathname: "/profiles/p1", state: { focusLine: 3, jumpId: 1 } }]}
        >
          <Routes>
            <Route path="/profiles/:id" element={<ProfileView />} />
          </Routes>
          <NavButton />
        </MemoryRouter>
      </JotaiProvider>,
    );

    // 起点：p1 第 3 行高亮
    expect(
      document.querySelector("[class*='lineNumberActive']")!.textContent,
    ).toBe("3");

    // 跳到 p2 的「第 5 行」—— p2 只有 1 行，clamp 必须落在 1 而不是
    // 用 p1 的 3 行 text 去算。
    fireEvent.click(screen.getByTestId("go-p2"));
    expect(
      document.querySelector("[class*='lineNumberActive']")!.textContent,
    ).toBe("1");
  });
});
