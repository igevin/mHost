import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { getDefaultStore, Provider as JotaiProvider } from "jotai";
import type { Profile } from "../../types";
import {
  profilesAtom,
  selectedProfileIdAtom,
  quickApplyOnToggleAtom,
  applyConfirmOpenAtom,
  applyResultAtom,
  applyErrorAtom,
  isApplyingAtom,
} from "../../stores/profiles";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(vi.fn()),
}));

// `quickApplyToggleAtom` (and `executeApplyAtom`) call `await listProfiles()`
// after `enable_and_apply` resolves so the in-memory atom mirrors disk.
// With the bare `invoke: vi.fn()` mock that call would resolve to
// `undefined` and overwrite `profilesAtom` with `undefined`, throwing
// `Cannot read properties of undefined (reading 'length')` on the next
// render. Mirror the DnsProfileList pattern: read the current store value
// (defaulting to `[]` so tests that don't set profiles still render).
vi.mock("../../lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/tauri")>();
  return {
    ...actual,
    listProfiles: vi.fn().mockImplementation(async () => {
      const store = getDefaultStore();
      return store.get(profilesAtom) ?? [];
    }),
  };
});
import Layout from "../Layout";

function makeProfile(overrides: Partial<Profile> = {}): Profile {
  return {
    id: "p1",
    name: "dev",
    description: "Development hosts profile",
    enabled: false,
    protected: false,
    tags: [],
    rules: [],
    mode: "hosts",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

function renderLayout() {
  return render(
    <JotaiProvider store={getDefaultStore()}>
      <MemoryRouter initialEntries={["/"]}>
        <Layout />
      </MemoryRouter>
    </JotaiProvider>,
  );
}

/** Wait for the sidebar switch to be available, then return its label. */
async function getToggleLabel(): Promise<HTMLLabelElement> {
  const switchEl = await screen.findByRole("switch");
  const label = switchEl.closest("label");
  if (!label) throw new Error("toggle label not found");
  return label as HTMLLabelElement;
}

describe("Sidebar toggle (issue #123)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(profilesAtom, []);
    store.set(selectedProfileIdAtom, null);
    store.set(quickApplyOnToggleAtom, false);
    store.set(applyConfirmOpenAtom, false);
    store.set(applyResultAtom, null);
    store.set(applyErrorAtom, null);
    store.set(isApplyingAtom, false);
  });

  it("with Quick Apply OFF, clicking sidebar toggle opens the Preview path", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    const { invoke } = await import("@tauri-apps/api/core");

    renderLayout();
    const label = await getToggleLabel();
    await act(async () => {
      fireEvent.click(label);
    });

    expect(invoke).toHaveBeenCalledWith(
      "generate_preview_plan",
      expect.objectContaining({ id: "p1", enabled: true }),
    );
    expect(invoke).not.toHaveBeenCalledWith(
      "enable_and_apply",
      expect.anything(),
    );
  });

  it("with Quick Apply ON, clicking sidebar toggle calls enable_and_apply directly", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    store.set(quickApplyOnToggleAtom, true);
    const { invoke } = await import("@tauri-apps/api/core");

    renderLayout();
    const label = await getToggleLabel();
    await act(async () => {
      fireEvent.click(label);
    });

    expect(invoke).toHaveBeenCalledWith(
      "enable_and_apply",
      expect.objectContaining({ id: "p1", enabled: true }),
    );
  });

  it("with Quick Apply ON but Cmd held, falls back to Preview", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    store.set(quickApplyOnToggleAtom, true);
    const { invoke } = await import("@tauri-apps/api/core");

    renderLayout();
    const label = await getToggleLabel();
    await act(async () => {
      // `metaKey` on the label click — Sidebar reads `e.metaKey || e.altKey`.
      fireEvent.click(label, { metaKey: true });
    });

    expect(invoke).toHaveBeenCalledWith(
      "generate_preview_plan",
      expect.objectContaining({ id: "p1", enabled: true }),
    );
    expect(invoke).not.toHaveBeenCalledWith(
      "enable_and_apply",
      expect.anything(),
    );
  });

  it("with Quick Apply ON but Option held, falls back to Preview", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    store.set(quickApplyOnToggleAtom, true);
    const { invoke } = await import("@tauri-apps/api/core");

    renderLayout();
    const label = await getToggleLabel();
    await act(async () => {
      fireEvent.click(label, { altKey: true });
    });

    expect(invoke).toHaveBeenCalledWith(
      "generate_preview_plan",
      expect.objectContaining({ id: "p1", enabled: true }),
    );
    expect(invoke).not.toHaveBeenCalledWith(
      "enable_and_apply",
      expect.anything(),
    );
  });

  // Regression: WebKit/Tauri fires BOTH `pointerdown` and `click` for the
  // same user click. Without the firedRef guard in `useWebKitPointerDown`,
  // Quick Apply would run `enable_and_apply` twice — two backups, two
  // writes to /etc/hosts — per single user gesture.
  it("Quick Apply is debounced — pointerdown + click only fires enable_and_apply once", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    store.set(quickApplyOnToggleAtom, true);
    const { invoke } = await import("@tauri-apps/api/core");

    renderLayout();
    const label = await getToggleLabel();
    await act(async () => {
      fireEvent.pointerDown(label);
      fireEvent.click(label);
    });

    const applyCalls = (invoke as unknown as { mock: { calls: unknown[][] } }).mock.calls.filter(
      (c: unknown[]) => c[0] === "enable_and_apply",
    );
    expect(applyCalls).toHaveLength(1);
  });
});
