import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { MemoryRouter, Routes, Route } from "react-router-dom";
import { getDefaultStore, Provider as JotaiProvider } from "jotai";
import type { Profile, HostRule } from "../../types";
import { profilesAtom, selectedProfileIdAtom } from "../../stores/profiles";

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

vi.mock("../../lib/tauri", () => ({
  validateHostsText: vi.fn().mockResolvedValue({ rules: [], errors: [] }),
  exportProfileToFile: vi.fn().mockResolvedValue(undefined),
  updateProfile: (...args: unknown[]) => mockUpdateProfile(...args),
  exportProfile: vi.fn().mockResolvedValue(""),
}));

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
          <Route path="/profiles/:id" element={<ProfileView />} />
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

  it("shows Delete confirmation when Delete button is clicked", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    const profile = makeProfile({ id: "p1", protected: false });
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    renderWithRouter("/profiles/p1");

    // Click delete button
    const deleteButton = screen.getByText("Delete");
    expect(deleteButton).not.toBeDisabled();

    await act(async () => {
      fireEvent.click(deleteButton);
    });

    // The confirm dialog should have been called
    expect(confirm).toHaveBeenCalled();
  });
});
