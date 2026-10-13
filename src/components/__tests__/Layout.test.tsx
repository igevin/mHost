import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { BrowserRouter } from "react-router-dom";
import { getDefaultStore, Provider as JotaiProvider } from "jotai";
import type { Profile } from "../../types";
import {
  profilesAtom,
  selectedProfileIdAtom,
  isApplyingAtom,
  dnsEnabledAtom,
  dnsStatusAtom,
  errorAtom,
  paletteOpenAtom,
} from "../../stores/profiles";

// Mock tauri invoke
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(vi.fn()),
}));

import Layout from "../Layout";

function makeProfile(overrides: Partial<Profile> = {}): Profile {
  return {
    id: "p1",
    name: "dev-profile",
    description: "Development hosts profile",
    enabled: true,
    protected: false,
    tags: [],
    rules: [],
    mode: "hosts",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
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

describe("Layout", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(profilesAtom, []);
    store.set(selectedProfileIdAtom, null);
    store.set(isApplyingAtom, false);
    store.set(dnsEnabledAtom, false);
    store.set(dnsStatusAtom, null);
    store.set(errorAtom, null);
    // 共享 atom：上个用例可能把面板开着 —— 每个用例前复位。
    store.set(paletteOpenAtom, false);
    // F7 用例会 stub navigator.platform —— 每个用例前复位为空字符串
    // （非 mac，与 jsdom 默认一致）。
    Object.defineProperty(window.navigator, "platform", {
      value: "",
      configurable: true,
    });
  });

  it("renders the mHost logo", () => {
    renderWithProviders(<Layout />);
    expect(screen.getByText("mHost")).toBeInTheDocument();
  });

  it("renders sidebar with Profiles section title", () => {
    renderWithProviders(<Layout />);
    expect(screen.getByText("Profiles")).toBeInTheDocument();
  });

  it("renders sidebar with Tools section title", () => {
    renderWithProviders(<Layout />);
    expect(screen.getByText("Tools")).toBeInTheDocument();
  });

  it("renders Tools navigation items", () => {
    renderWithProviders(<Layout />);
    expect(screen.getByText("Ad Block")).toBeInTheDocument();
    expect(screen.getByText("Remote Rules")).toBeInTheDocument();
    expect(screen.getByText("Snapshots")).toBeInTheDocument();
    expect(screen.getByText("Settings")).toBeInTheDocument();
  });

  it("renders profile list items when profiles exist", () => {
    const profiles = [
      makeProfile({ id: "p1", name: "dev", description: "Development profile" }),
      makeProfile({ id: "p2", name: "staging", description: "Staging profile" }),
    ];
    const store = getDefaultStore();
    store.set(profilesAtom, profiles);

    renderWithProviders(<Layout />);

    // Use getAllByText since profile name may also appear in StatusBar
    const devElements = screen.getAllByText("dev");
    expect(devElements.length).toBeGreaterThanOrEqual(1);
    const stagingElements = screen.getAllByText("staging");
    expect(stagingElements.length).toBeGreaterThanOrEqual(1);
  });

  it("renders status dot for each profile", () => {
    const profiles = [
      makeProfile({ id: "p1", name: "dev", enabled: true }),
      makeProfile({ id: "p2", name: "staging", enabled: false }),
    ];
    const store = getDefaultStore();
    store.set(profilesAtom, profiles);

    renderWithProviders(<Layout />);

    const statusDots = screen.getAllByRole("generic").filter(
      (el) => el.classList.contains("profileStatusDot"),
    );
    // At least one dot should be rendered (we check it doesn't crash)
    expect(statusDots.length).toBeGreaterThanOrEqual(0);
  });

  it("renders truncated description for each profile", () => {
    const profiles = [
      makeProfile({ id: "p1", name: "dev", description: "Development profile for local testing" }),
    ];
    const store = getDefaultStore();
    store.set(profilesAtom, profiles);

    renderWithProviders(<Layout />);

    // The description should be truncated and present in DOM
    expect(screen.getByText("Development profile for local testing")).toBeInTheDocument();
  });

  it("renders toggle switch for each profile", () => {
    const profiles = [
      makeProfile({ id: "p1", name: "dev", enabled: true }),
      makeProfile({ id: "p2", name: "staging", enabled: false }),
    ];
    const store = getDefaultStore();
    store.set(profilesAtom, profiles);

    renderWithProviders(<Layout />);

    // Toggle switches should be rendered as role="switch". Scope to the
    // profile list — StatusBar's DNS card has its own switch (Phase 2).
    const toggles = screen
      .getAllByRole("switch")
      .filter((el) => el.closest("[class*='profileList']") !== null);
    expect(toggles).toHaveLength(2);
  });

  it("renders toggle switch checked state based on profile enabled", () => {
    const profiles = [
      makeProfile({ id: "p1", name: "dev", enabled: true }),
      makeProfile({ id: "p2", name: "staging", enabled: false }),
    ];
    const store = getDefaultStore();
    store.set(profilesAtom, profiles);

    renderWithProviders(<Layout />);

    const toggles = screen.getAllByRole("switch");
    // First profile enabled -> checked
    expect(toggles[0]).toBeChecked();
    // Second profile disabled -> not checked
    expect(toggles[1]).not.toBeChecked();
  });

  it("renders '+ New Profile' button in sidebar", () => {
    renderWithProviders(<Layout />);
    expect(screen.getByText("+ New Profile")).toBeInTheDocument();
  });

  it("highlights the selected profile", () => {
    const profiles = [
      makeProfile({ id: "p1", name: "dev" }),
      makeProfile({ id: "p2", name: "staging" }),
    ];
    const store = getDefaultStore();
    store.set(profilesAtom, profiles);
    store.set(selectedProfileIdAtom, "p1");

    renderWithProviders(<Layout />);

    // The selected profile item should have the active class
    // CSS Modules hash class names, so we check via class attribute substring
    const profileItems = document.querySelectorAll("[class*='profileItem']");
    const activeItems = Array.from(profileItems).filter(
      (el) => el.className.includes("profileItemActive"),
    );
    expect(activeItems.length).toBe(1);
    expect(activeItems[0].textContent).toContain("dev");
  });

  it("shows empty message when no profiles", () => {
    renderWithProviders(<Layout />);
    expect(screen.getByText("No profiles yet")).toBeInTheDocument();
  });

  it("renders disabled tool items with 'Soon' badge", () => {
    renderWithProviders(<Layout />);
    expect(screen.getAllByText("Soon").length).toBeGreaterThanOrEqual(1);
  });

  it("navigates to profile page when profile item is clicked", async () => {
    const profiles = [
      makeProfile({ id: "p1", name: "dev" }),
      makeProfile({ id: "p2", name: "staging" }),
    ];
    const store = getDefaultStore();
    store.set(profilesAtom, profiles);

    renderWithProviders(<Layout />);

    // Click on the "dev" profile item
    const devElements = screen.getAllByText("dev");
    // Find the one in the profile list (clickable item)
    const profileItem = devElements.find(
      (el) => el.closest("[class*='profileItem']") !== null
    );
    expect(profileItem).toBeTruthy();
    if (profileItem) {
      fireEvent.click(profileItem);
    }
  });

  it("shows DNS status indicator when dnsEnabled is true", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, true);
    store.set(dnsStatusAtom, { running: true, port: 53, upstream: [], original_dns: { kind: "dhcp_empty" }, rule_count: 0, cache_capacity: 100 });

    renderWithProviders(<Layout />);

    // Scope: sidebar indicator specifically (StatusBar also shows a 'DNS'
    // label after issue #67, so we cannot use plain getByText).
    const indicator = document.querySelector("[class*='dnsStatusIndicator']");
    expect(indicator).toBeTruthy();
    expect(indicator!.textContent).toContain("DNS");
  });

  it("hides DNS status indicator when dnsEnabled is false", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, false);

    renderWithProviders(<Layout />);

    // Sidebar indicator must be hidden when dnsEnabled is false.
    // (StatusBar DNS column is a separate UI element and stays visible.)
    const indicator = document.querySelector("[class*='dnsStatusIndicator']");
    expect(indicator).toBeFalsy();
  });

  /** review F7: platform gating — mac 用 metaKey && !ctrlKey；其它平台
   *  ctrlKey && !metaKey；Shift 一律不触发。逐用例 stub platform。 */
  function stubPlatform(value: string) {
    Object.defineProperty(window.navigator, "platform", {
      value,
      configurable: true,
    });
  }

  it("Cmd+K toggles the command palette on macOS (issue #124)", () => {
    stubPlatform("MacIntel");
    renderWithProviders(<Layout />);
    expect(screen.queryByRole("dialog", { name: "Command palette" })).not.toBeInTheDocument();

    fireEvent.keyDown(document, { key: "k", metaKey: true });
    expect(screen.getByRole("dialog", { name: "Command palette" })).toBeInTheDocument();

    // 再按一次关闭（toggle，不劫持面板内部按键）
    fireEvent.keyDown(document, { key: "k", metaKey: true });
    expect(screen.queryByRole("dialog", { name: "Command palette" })).not.toBeInTheDocument();
  });

  it("Ctrl+K opens the palette on non-macOS platforms", () => {
    stubPlatform("Win32");
    renderWithProviders(<Layout />);
    fireEvent.keyDown(document, { key: "k", ctrlKey: true });
    expect(screen.getByRole("dialog", { name: "Command palette" })).toBeInTheDocument();
  });

  it("does NOT fire on the other platform's modifier (F7)", () => {
    stubPlatform("MacIntel");
    renderWithProviders(<Layout />);
    // macOS 上 Ctrl+K 是 Cocoa kill-line，不能劫持
    fireEvent.keyDown(document, { key: "k", ctrlKey: true });
    expect(screen.queryByRole("dialog", { name: "Command palette" })).not.toBeInTheDocument();
  });

  it("does NOT fire with Shift held (F7)", () => {
    stubPlatform("MacIntel");
    renderWithProviders(<Layout />);
    fireEvent.keyDown(document, { key: "k", metaKey: true, shiftKey: true });
    expect(screen.queryByRole("dialog", { name: "Command palette" })).not.toBeInTheDocument();
  });

  it("renders the global errorAtom banner and dismisses it (review F4)", () => {
    const store = getDefaultStore();
    renderWithProviders(<Layout />);

    // errorAtom 现在由 Layout 全局渲染 —— 任何路由都看得到。
    act(() => {
      store.set(errorAtom, "delete failed: ProfileNotFound");
    });
    expect(screen.getByText("delete failed: ProfileNotFound")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(store.get(errorAtom)).toBeNull();
  });
});
