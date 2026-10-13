import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act, within } from "@testing-library/react";
import { MemoryRouter, useLocation } from "react-router-dom";
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
const mockDuplicateProfile = vi.fn().mockResolvedValue(undefined);
const mockExportProfileToFile = vi.fn().mockResolvedValue(undefined);
vi.mock("../../lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/tauri")>();
  return {
    ...actual,
    listProfiles: vi.fn().mockImplementation(async () => {
      const store = getDefaultStore();
      return store.get(profilesAtom) ?? [];
    }),
    duplicateProfile: (...args: unknown[]) => mockDuplicateProfile(...args),
    exportProfileToFile: (...args: unknown[]) =>
      mockExportProfileToFile(...args),
  };
});

vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: vi.fn().mockResolvedValue("/tmp/export.hosts"),
}));
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
  // Scope to the profile list — StatusBar's DNS card has its own switch
  // (Phase 2), so a bare findByRole("switch") matches multiple elements.
  await screen.findAllByRole("switch");
  const switchEl = document.querySelector(
    "[class*='profileItem'] [role='switch']",
  );
  const label = switchEl?.closest("label");
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

/**
 * Phase 2: 侧栏即管理面。ManagementDrawer 删除后，搜索过滤 / 排序 /
 * ⋯ 菜单（Edit info / Duplicate / Export / Delete / Enable-Disable）
 * 都在 Sidebar 里。这些用例钉住新行为。
 */
describe("Sidebar management surface (Phase 2)", () => {
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

  /** 当前侧栏 profile 行的名字，按 DOM 顺序。 */
  function profileRowNames(): string[] {
    return Array.from(
      document.querySelectorAll("[class*='profileItem'] [class*='profileItemName']"),
    ).map((el) => el.textContent ?? "");
  }

  function menuButtonFor(name: string): HTMLElement {
    return screen.getByRole("button", {
      name: `Profile actions for ${name}`,
    });
  }

  it("filters profile rows by name (substring, case-insensitive)", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [
      makeProfile({ id: "p1", name: "dev" }),
      makeProfile({ id: "p2", name: "Staging" }),
      makeProfile({ id: "p3", name: "dev-ops" }),
    ]);

    renderLayout();

    fireEvent.change(screen.getByLabelText("Filter profiles"), {
      target: { value: "DEV" },
    });

    expect(profileRowNames()).toEqual(["dev", "dev-ops"]);

    // 清空过滤后所有行恢复
    fireEvent.change(screen.getByLabelText("Filter profiles"), {
      target: { value: "" },
    });
    expect(profileRowNames()).toHaveLength(3);
  });

  it("shows an empty-filter hint instead of rows when nothing matches", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", name: "dev" })]);

    renderLayout();

    fireEvent.change(screen.getByLabelText("Filter profiles"), {
      target: { value: "nomatch" },
    });

    expect(screen.getByText("No matching profiles")).toBeInTheDocument();
    expect(profileRowNames()).toHaveLength(0);
  });

  it("sorts enabled profiles first, then alphabetically by name", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [
      makeProfile({ id: "p1", name: "alpha", enabled: false }),
      makeProfile({ id: "p2", name: "zeta", enabled: true }),
      makeProfile({ id: "p3", name: "beta", enabled: true }),
      makeProfile({ id: "p4", name: "gamma", enabled: false }),
    ]);

    renderLayout();

    // 启用组内字母序：beta → zeta；然后禁用组：alpha → gamma
    expect(profileRowNames()).toEqual(["beta", "zeta", "alpha", "gamma"]);
  });

  it("opens the ⋯ menu with all management items", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", name: "dev" })]);

    renderLayout();

    fireEvent.click(menuButtonFor("dev"));

    expect(
      screen.getByRole("menuitem", { name: "Enable" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("menuitem", { name: "Edit info" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("menuitem", { name: "Duplicate" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("menuitem", { name: "Export" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("menuitem", { name: "Delete" }),
    ).toBeInTheDocument();
  });

  it("menu Enable/Disable item reflects current state and routes through the preview flow", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", name: "dev", enabled: true })]);
    const invoke = await setupInvoke();

    renderLayout();

    fireEvent.click(menuButtonFor("dev"));
    await act(async () => {
      fireEvent.click(screen.getByRole("menuitem", { name: "Disable" }));
    });

    expect(invoke).toHaveBeenCalledWith(
      "preview_apply_outcome",
      expect.objectContaining({ id: "p1", enabled: false }),
    );
  });

  it("Edit info navigates to the profile view", () => {
    function LocationProbe() {
      const location = useLocation();
      return <div data-testid="location">{location.pathname}</div>;
    }
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", name: "dev" })]);

    render(
      <JotaiProvider store={getDefaultStore()}>
        <MemoryRouter initialEntries={["/"]}>
          <Layout />
          <LocationProbe />
        </MemoryRouter>
      </JotaiProvider>,
    );

    fireEvent.click(menuButtonFor("dev"));
    fireEvent.click(screen.getByRole("menuitem", { name: "Edit info" }));

    expect(screen.getByTestId("location")).toHaveTextContent("/profiles/p1");
  });

  it("Duplicate calls duplicateProfile and refreshes the list", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", name: "dev" })]);

    renderLayout();

    fireEvent.click(menuButtonFor("dev"));
    await act(async () => {
      fireEvent.click(screen.getByRole("menuitem", { name: "Duplicate" }));
    });

    expect(mockDuplicateProfile).toHaveBeenCalledWith("p1", "dev (copy)");
  });

  it("Export goes through the native save dialog then exportProfileToFile", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", name: "dev" })]);

    renderLayout();

    fireEvent.click(menuButtonFor("dev"));
    await act(async () => {
      fireEvent.click(screen.getByRole("menuitem", { name: "Export" }));
    });

    expect(mockExportProfileToFile).toHaveBeenCalledWith(
      "p1",
      "hosts",
      "/tmp/export.hosts",
    );
  });

  it("Delete opens ConfirmDialog and only deletes on confirm", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", name: "dev" })]);
    const invoke = await setupInvoke();

    renderLayout();

    fireEvent.click(menuButtonFor("dev"));
    fireEvent.click(screen.getByRole("menuitem", { name: "Delete" }));

    // 菜单关闭，ConfirmDialog 接管确认
    expect(screen.queryByRole("menuitem")).not.toBeInTheDocument();
    expect(
      screen.getByText('Delete profile "dev"?'),
    ).toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalledWith("delete_profile", expect.anything());

    await act(async () => {
      fireEvent.click(
        within(screen.getByRole("dialog")).getByRole("button", {
          name: "Delete",
        }),
      );
    });

    expect(invoke).toHaveBeenCalledWith("delete_profile", { id: "p1" });
  });

  // review F5: 确认框在异步删除完成前保持打开（isLoading），成功才关闭。
  it("delete ConfirmDialog stays open (isLoading) until the delete finishes (F5)", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", name: "dev" })]);
    let resolveDelete: () => void;
    const invoke = await setupInvoke();
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === "delete_profile") {
        return new Promise<void>((r) => {
          resolveDelete = r;
        });
      }
      return null;
    });

    renderLayout();

    fireEvent.click(menuButtonFor("dev"));
    fireEvent.click(screen.getByRole("menuitem", { name: "Delete" }));
    await act(async () => {
      fireEvent.click(
        within(screen.getByRole("dialog")).getByRole("button", {
          name: "Delete",
        }),
      );
    });

    // 删除进行中：对话框还在，按钮呈 loading 态且被锁
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "Deleting...",
      }),
    ).toBeDisabled();

    // 完成后才关闭
    await act(async () => {
      resolveDelete!();
    });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("Delete menu item is disabled for protected profiles", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", name: "dev", protected: true })]);

    renderLayout();

    fireEvent.click(menuButtonFor("dev"));

    expect(screen.getByRole("menuitem", { name: "Delete" })).toBeDisabled();
  });

  it("closes the menu on Escape", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", name: "dev" })]);

    renderLayout();

    fireEvent.click(menuButtonFor("dev"));
    expect(screen.getByRole("menu")).toBeInTheDocument();

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("closes the menu on outside click (pointerdown+click on the overlay)", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", name: "dev" })]);

    renderLayout();

    fireEvent.click(menuButtonFor("dev"));
    expect(screen.getByRole("menu")).toBeInTheDocument();

    // PopupMenu 的 overlay 要求先看到它自己的 pointerdown 才响应 click —
    // 这是为了吞掉打开手势的 trailing click（见 PopupMenu 注释）。
    const overlay = document.querySelector("[class*='overlay']")!;
    fireEvent.pointerDown(overlay);
    fireEvent.click(overlay);

    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("ignores a bare overlay click with no preceding pointerdown (opening-gesture trailing click)", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", name: "dev" })]);

    renderLayout();

    fireEvent.click(menuButtonFor("dev"));
    const overlay = document.querySelector("[class*='overlay']")!;
    fireEvent.click(overlay);

    expect(screen.getByRole("menu")).toBeInTheDocument();
  });
});
