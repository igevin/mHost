import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { getDefaultStore, Provider as JotaiProvider } from "jotai";
import type { Profile, HostRule } from "../../types";
import {
  dnsProfilesAtom,
  dnsEnabledAtom,
  isDnsLoadingAtom,
  dnsErrorAtom,
} from "../../stores/profiles";

// Mock tauri
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(vi.fn()),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: vi.fn().mockResolvedValue("/tmp/export.dns"),
  confirm: vi.fn().mockResolvedValue(true),
  open: vi.fn(),
}));

// listDnsProfiles returns the input profiles (so derived atoms don't crash
// after a toggle's re-fetch). All other invoke() calls default to undefined.
vi.mock("../../lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/tauri")>();
  return {
    ...actual,
    listDnsProfiles: vi.fn().mockImplementation(async () => {
      const store = getDefaultStore();
      return store.get(dnsProfilesAtom) ?? [];
    }),
    duplicateProfile: vi.fn().mockResolvedValue(undefined),
    exportProfileToFile: vi.fn().mockResolvedValue(undefined),
  };
});

import DnsProfileList from "../DnsProfileList";

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
    id: "d1",
    name: "ads",
    description: "Block ads",
    enabled: true,
    protected: false,
    tags: ["ads"],
    rules: [makeRule({ id: "r1", ip: "0.0.0.0", domains: ["a.com"] })],
    mode: "dns",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-06-15T10:30:00Z",
    ...overrides,
  };
}

function renderWithRouter() {
  return render(
    <JotaiProvider store={getDefaultStore()}>
      <MemoryRouter initialEntries={["/dns-profiles"]}>
        <DnsProfileList />
      </MemoryRouter>
    </JotaiProvider>,
  );
}

describe("DnsProfileList", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, []);
    store.set(dnsEnabledAtom, false);
    store.set(isDnsLoadingAtom, false);
    store.set(dnsErrorAtom, null);
  });

  // ---- Header ----

  it("renders title and DNS mode badge", () => {
    renderWithRouter();
    expect(screen.getByText("DNS Profiles")).toBeInTheDocument();
    expect(screen.getByText("Mode Off")).toBeInTheDocument();
  });

  it("shows 'Mode On' badge when dnsEnabled is true", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, true);
    store.set(dnsProfilesAtom, [makeProfile()]);
    renderWithRouter();
    expect(screen.getByText("Mode On")).toBeInTheDocument();
  });

  // ---- Empty state ----

  it("renders empty state when no profiles", () => {
    renderWithRouter();
    expect(screen.getByText("No DNS profiles yet")).toBeInTheDocument();
    expect(screen.getByText(/Create a profile/i)).toBeInTheDocument();
  });

  it("empty state shows exactly one '+ New DNS Profile' button", () => {
    renderWithRouter();
    const buttons = screen.getAllByText("+ New DNS Profile");
    expect(buttons).toHaveLength(1);
  });

  // ---- Stats ----

  it("renders stats grid with Total / Enabled / Active Rules", () => {
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [
      makeProfile({
        id: "d1",
        enabled: true,
        rules: [
          makeRule({ id: "r1", ip: "0.0.0.0", domains: ["a.com"] }),
          makeRule({ id: "r2", ip: "0.0.0.0", domains: ["b.com"] }),
        ],
      }),
      makeProfile({
        id: "d2",
        enabled: false,
        rules: [makeRule({ id: "r3", ip: "127.0.0.1", domains: ["c.com"] })],
      }),
    ]);
    renderWithRouter();

    const stats = document.querySelector("[class*='statsGrid']");
    expect(stats).toBeTruthy();
    const cards = stats!.querySelectorAll("[class*='statCard']");
    expect(cards).toHaveLength(3);

    expect(cards[0].textContent).toContain("Total Profiles");
    expect(cards[0].textContent).toContain("2");

    expect(cards[1].textContent).toContain("Enabled");
    expect(cards[1].textContent).toContain("1");

    expect(cards[2].textContent).toContain("Active Rules");
    expect(cards[2].textContent).toContain("2");
  });

  it("Active Rules counts only rules with non-null ip across enabled profiles", () => {
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [
      makeProfile({
        id: "d1",
        enabled: true,
        rules: [
          makeRule({ id: "r1", ip: "0.0.0.0", domains: ["a.com"] }),
          // comment-only rule (no IP) — must NOT be counted
          makeRule({ id: "r2", ip: null, domains: [], enabled: true }),
        ],
      }),
    ]);
    renderWithRouter();

    const cards = document.querySelectorAll("[class*='statCard']");
    expect(cards[2].textContent).toContain("Active Rules");
    expect(cards[2].textContent).toContain("1");
  });

  // ---- Profile cards ----

  it("renders one card per profile", () => {
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [
      makeProfile({ id: "d1", name: "ads" }),
      makeProfile({ id: "d2", name: "trackers" }),
    ]);
    renderWithRouter();
    const headings = screen.getAllByRole("heading", { level: 3 });
    const names = headings.map((h) => h.textContent);
    expect(names).toContain("ads");
    expect(names).toContain("trackers");
  });

  it("card shows description and tags", () => {
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [
      makeProfile({
        id: "d1",
        name: "my-dns-profile",
        description: "Block ad domains",
        tags: ["ads", "tracking"],
      }),
    ]);
    renderWithRouter();
    expect(screen.getByText("Block ad domains")).toBeInTheDocument();
    expect(screen.getByText("ads")).toBeInTheDocument();
    expect(screen.getByText("tracking")).toBeInTheDocument();
  });

  it("card shows Enabled/Disabled badge based on profile state", () => {
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [
      makeProfile({ id: "d1", enabled: true }),
      makeProfile({ id: "d2", enabled: false }),
    ]);
    renderWithRouter();
    const profileCards = screen.getAllByTestId("dns-profile-card");
    expect(profileCards).toHaveLength(2);
    expect(profileCards[0].textContent).toContain("Enabled");
    expect(profileCards[0].textContent).not.toContain("Disabled");
    expect(profileCards[1].textContent).toContain("Disabled");
    expect(profileCards[1].textContent).not.toContain("Enabled");
  });

  it("card shows Protected badge for protected profile", () => {
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [makeProfile({ protected: true })]);
    renderWithRouter();
    expect(screen.getByText("Protected")).toBeInTheDocument();
  });

  // ---- Toggle ----

  it("inline switch reflects enabled state of each profile", () => {
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [
      makeProfile({ id: "d1", enabled: true }),
      makeProfile({ id: "d2", enabled: false }),
    ]);
    renderWithRouter();
    const switches = screen.getAllByRole("switch") as HTMLInputElement[];
    expect(switches).toHaveLength(2);
    const checked = switches.filter((s) => s.checked).length;
    expect(checked).toBe(1);
  });

  it("clicking the inline switch calls setProfileEnabled via toggle atom", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [makeProfile({ id: "d1", enabled: true })]);

    renderWithRouter();

    const switchEl = screen.getByRole("switch");
    // The actual click is on the wrapping label; fireEvent.click on the input
    // won't trigger label handlers. We fire on the label parent.
    const label = switchEl.closest("label")!;
    await act(async () => {
      fireEvent.click(label);
    });

    // setProfileEnabled(id, enabled) → invoke("set_profile_enabled", { id, enabled })
    expect(invoke).toHaveBeenCalledWith(
      "set_profile_enabled",
      expect.objectContaining({ id: "d1", enabled: false }),
    );
  });

  // ---- Edit ----

  it("clicking Edit on a card does not throw", () => {
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [makeProfile({ id: "d1", name: "ads" })]);
    renderWithRouter();

    const editButtons = screen.getAllByText("Edit");
    expect(editButtons.length).toBeGreaterThanOrEqual(1);
    fireEvent.click(editButtons[editButtons.length - 1]);
  });

  // ---- Mode-off banner ----

  it("shows mode-off banner when DNS is off but profiles exist", () => {
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [makeProfile()]);
    store.set(dnsEnabledAtom, false);

    renderWithRouter();

    expect(screen.getByText(/DNS mode is off/i)).toBeInTheDocument();
    expect(screen.getByText(/Enable in Settings/i)).toBeInTheDocument();
  });

  it("does not show mode-off banner when DNS is on", () => {
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [makeProfile()]);
    store.set(dnsEnabledAtom, true);

    renderWithRouter();

    expect(screen.queryByText(/DNS mode is off/i)).not.toBeInTheDocument();
  });

  it("does not show mode-off banner when no profiles exist", () => {
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, []);
    store.set(dnsEnabledAtom, false);

    renderWithRouter();

    expect(screen.queryByText(/DNS mode is off/i)).not.toBeInTheDocument();
  });

  // ---- Delete ----

  it("clicking Delete triggers confirm dialog", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [makeProfile({ id: "d1", protected: false })]);

    renderWithRouter();

    const deleteButton = screen.getByRole("button", { name: /delete/i });
    await act(async () => {
      fireEvent.click(deleteButton);
    });

    expect(confirm).toHaveBeenCalledWith("Delete this DNS profile?");
  });

  it("disables Delete button for protected profile", () => {
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [makeProfile({ protected: true })]);
    renderWithRouter();

    const deleteButton = screen.getByRole("button", { name: /delete/i });
    expect(deleteButton).toBeDisabled();
  });

  // ---- Error ----

  it("shows error banner when dnsErrorAtom is set", () => {
    const store = getDefaultStore();
    store.set(dnsErrorAtom, "DNS server unreachable");
    renderWithRouter();

    expect(screen.getByText("DNS server unreachable")).toBeInTheDocument();
  });

  // ---- Header button visibility ----

  it("header '+ New DNS Profile' button is visible when profiles exist", () => {
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [makeProfile()]);
    renderWithRouter();

    const buttons = screen.getAllByText("+ New DNS Profile");
    // 1 header button only (no empty-state when profiles exist)
    expect(buttons).toHaveLength(1);
  });

  // ---- Tags ----

  it("renders tags as 'tag' spans", () => {
    const store = getDefaultStore();
    store.set(dnsProfilesAtom, [
      makeProfile({ id: "d1", tags: ["ads", "trackers"] }),
    ]);
    renderWithRouter();

    const tagSpans = document.querySelectorAll(".tag");
    const tagTexts = Array.from(tagSpans).map((s) => s.textContent);
    expect(tagTexts).toContain("ads");
    expect(tagTexts).toContain("trackers");
  });
});
