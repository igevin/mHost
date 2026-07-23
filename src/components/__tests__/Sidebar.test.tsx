import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { getDefaultStore, Provider as JotaiProvider } from "jotai";
import type { ApplyOutcome, ApplyPlan, Profile } from "../../types";
import {
  profilesAtom,
  selectedProfileIdAtom,
  quickApplyOnToggleAtom,
  applyConfirmOpenAtom,
  applyResultAtom,
  applyErrorAtom,
  isApplyingAtom,
} from "../../stores/profiles";

// Default safe `ApplyOutcome` — `preview_apply_outcome` and
// `enable_and_apply` both return this. Tests override per-call via
// `mockResolvedValueOnce` when they need a different shape (conflicts,
// bulk changes, disabled IDs, errors).
const safePlan: ApplyPlan = {
  rules: [],
  conflicts: [],
  diff: {
    added: ["1.1.1.1 dev.local"],
    removed: [],
    unchanged: [],
  },
  backup_required: true,
};
const safeOutcome: ApplyOutcome = {
  plan: safePlan,
  added_count: safePlan.diff.added.length,
  removed_count: safePlan.diff.removed.length,
  unchanged_count: safePlan.diff.unchanged.length,
  disabled_profile_ids: [],
  has_conflicts: false,
  snapshot_id: "snap-1",
  backup_path: "/tmp/backups/hosts.bak",
};

// Only mock the underlying `invoke` (used by tauri.ts wrappers).
// `previewApplyOutcome` / `enableAndApply` in tauri.ts are NOT mocked —
// they call the real `invoke` which we control. This lets us assert
// call shapes via the `invoke` spy (matches #123's original pattern).
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(vi.fn()),
}));

// `quickApplyToggleAtom` calls `listProfiles()` to refresh atoms after
// apply. Provide a store-aware mock so it returns the current profiles
// instead of `undefined` (which would clobber `profilesAtom`).
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

/** Set up invoke to dispatch by command name. Returns the `invoke` spy. */
async function setupInvoke(
  overrides: Partial<Record<string, unknown>> = {},
): Promise<ReturnType<typeof vi.fn>> {
  const { invoke } = await import("@tauri-apps/api/core");
  const spy = invoke as unknown as ReturnType<typeof vi.fn>;
  spy.mockReset();
  spy.mockImplementation(async (cmd: string) => {
    if (cmd === "preview_apply_outcome") return overrides.preview_apply_outcome ?? safeOutcome;
    if (cmd === "enable_and_apply") return overrides.enable_and_apply ?? safeOutcome;
    return null;
  });
  return spy;
}

describe("Sidebar toggle (issue #123/127)", () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(profilesAtom, []);
    store.set(selectedProfileIdAtom, null);
    store.set(quickApplyOnToggleAtom, false);
    store.set(applyConfirmOpenAtom, false);
    store.set(applyResultAtom, null);
    store.set(applyErrorAtom, null);
    store.set(isApplyingAtom, false);
    await setupInvoke();
  });

  it("with Quick Apply OFF, clicking sidebar toggle opens the Preview path", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    const invoke = await setupInvoke();

    renderLayout();
    const label = await getToggleLabel();
    await act(async () => {
      fireEvent.click(label);
    });

    expect(invoke).toHaveBeenCalledWith(
      "preview_apply_outcome",
      expect.objectContaining({ id: "p1", enabled: true }),
    );
    expect(invoke).not.toHaveBeenCalledWith(
      "enable_and_apply",
      expect.anything(),
    );
  });

  it("with Quick Apply ON, clicking sidebar toggle calls enable_and_apply after preview", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    store.set(quickApplyOnToggleAtom, true);
    const invoke = await setupInvoke();

    renderLayout();
    const label = await getToggleLabel();
    await act(async () => {
      fireEvent.click(label);
    });

    expect(invoke).toHaveBeenCalledWith(
      "preview_apply_outcome",
      expect.objectContaining({ id: "p1", enabled: true }),
    );
    expect(invoke).toHaveBeenCalledWith(
      "enable_and_apply",
      expect.objectContaining({ id: "p1", enabled: true }),
    );
  });

  it("with Quick Apply ON but Cmd held, falls back to Preview dialog (no enable_and_apply)", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    store.set(quickApplyOnToggleAtom, true);
    const invoke = await setupInvoke();

    renderLayout();
    const label = await getToggleLabel();
    await act(async () => {
      // `metaKey` on the label click — Sidebar reads `e.metaKey || e.altKey`.
      fireEvent.click(label, { metaKey: true });
    });

    expect(invoke).toHaveBeenCalledWith(
      "preview_apply_outcome",
      expect.objectContaining({ id: "p1", enabled: true }),
    );
    expect(invoke).not.toHaveBeenCalledWith(
      "enable_and_apply",
      expect.anything(),
    );
  });

  it("with Quick Apply ON but Option held, falls back to Preview dialog (no enable_and_apply)", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    store.set(quickApplyOnToggleAtom, true);
    const invoke = await setupInvoke();

    renderLayout();
    const label = await getToggleLabel();
    await act(async () => {
      fireEvent.click(label, { altKey: true });
    });

    expect(invoke).toHaveBeenCalledWith(
      "preview_apply_outcome",
      expect.objectContaining({ id: "p1", enabled: true }),
    );
    expect(invoke).not.toHaveBeenCalledWith(
      "enable_and_apply",
      expect.anything(),
    );
  });

  // Refs #127: when the preview reports conflicts, the policy classifies
  // the apply as require_preview and the toggle opens the dialog instead
  // of writing.
  it("with Quick Apply ON and conflicts in preview, falls back to Preview dialog", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    store.set(quickApplyOnToggleAtom, true);
    const conflictOutcome: ApplyOutcome = {
      ...safeOutcome,
      has_conflicts: true,
      plan: { ...safePlan, conflicts: [{ domain: "x.example", rules: [] }] },
    };
    const invoke = await setupInvoke({ preview_apply_outcome: conflictOutcome });

    renderLayout();
    const label = await getToggleLabel();
    await act(async () => {
      fireEvent.click(label);
    });

    expect(invoke).toHaveBeenCalledWith(
      "preview_apply_outcome",
      expect.anything(),
    );
    expect(invoke).not.toHaveBeenCalledWith(
      "enable_and_apply",
      expect.anything(),
    );
  });

  // Refs #127: bulk changes (>100 added+removed) trip the destructive
  // threshold and force the preview path.
  it("with Quick Apply ON and bulk changes (>100) in preview, falls back to Preview dialog", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    store.set(quickApplyOnToggleAtom, true);
    const bulkOutcome: ApplyOutcome = {
      ...safeOutcome,
      added_count: 80,
      removed_count: 25, // total 105 > DESTRUCTIVE_THRESHOLD (100)
    };
    const invoke = await setupInvoke({ preview_apply_outcome: bulkOutcome });

    renderLayout();
    const label = await getToggleLabel();
    await act(async () => {
      fireEvent.click(label);
    });

    expect(invoke).not.toHaveBeenCalledWith(
      "enable_and_apply",
      expect.anything(),
    );
  });

  // Refs #127: enabling a hosts profile while another is already enabled
  // would auto-disable the other — destructive side effect → preview.
  it("with Quick Apply ON and another profile would be disabled, falls back to Preview dialog", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    store.set(quickApplyOnToggleAtom, true);
    const wouldDisable: ApplyOutcome = {
      ...safeOutcome,
      disabled_profile_ids: ["other-profile-id"],
    };
    const invoke = await setupInvoke({ preview_apply_outcome: wouldDisable });

    renderLayout();
    const label = await getToggleLabel();
    await act(async () => {
      fireEvent.click(label);
    });

    expect(invoke).not.toHaveBeenCalledWith(
      "enable_and_apply",
      expect.anything(),
    );
  });

  // Regression: WebKit/Tauri fires BOTH `pointerdown` and `click` for the
  // same user click. Without the firedRef guard in `useWebKitPointerDown`,
  // Quick Apply would run `preview_apply_outcome` (and on success,
  // `enable_and_apply`) twice — two backups, two writes to /etc/hosts —
  // per single user gesture.
  it("Quick Apply is debounced — pointerdown + click only fires enable_and_apply once", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    store.set(quickApplyOnToggleAtom, true);
    const invoke = await setupInvoke();

    renderLayout();
    const label = await getToggleLabel();
    await act(async () => {
      fireEvent.pointerDown(label);
      fireEvent.click(label);
    });

    const applyCalls = invoke.mock.calls.filter(
      (c: unknown[]) => c[0] === "enable_and_apply",
    );
    expect(applyCalls).toHaveLength(1);
  });
});