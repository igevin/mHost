import { describe, it, expect, vi, beforeEach } from "vitest";
import { getDefaultStore } from "jotai";
import type { ApplyOutcome, ApplyPlan, Profile } from "../../types";
import {
  profilesAtom,
  selectedProfileIdAtom,
  isApplyingAtom,
  errorAtom,
  applyResultAtom,
  applyErrorAtom,
  applyConfirmOpenAtom,
  applyPlanAtom,
  applyTargetAtom,
  quickApplyOutcomeAtom,
  isQuickApplyToastOpenAtom,
  quickApplyToggleAtom,
} from "../profiles";

// Refs #127: mock Tauri bindings so `quickApplyToggleAtom` runs end-to-end
// against deterministic `previewApplyOutcome` / `enableAndApply` /
// `listProfiles` responses. Each test overrides the per-call behavior
// via the spy's `.mock*` API.
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
  added_count: 1,
  removed_count: 0,
  unchanged_count: 0,
  disabled_profile_ids: [],
  has_conflicts: false,
  snapshot_id: "snap-1",
  backup_path: "/tmp/backups/hosts.bak",
};

vi.mock("../../lib/tauri", () => {
  // vi.mock is hoisted to top of file — define defaults inline so the
  // factory doesn't depend on module-level consts that haven't initialized.
  const inlinePlan = {
    rules: [],
    conflicts: [],
    diff: { added: ["1.1.1.1 dev.local"], removed: [], unchanged: [] },
    backup_required: true,
  };
  const inlineOutcome = {
    plan: inlinePlan,
    added_count: 1,
    removed_count: 0,
    unchanged_count: 0,
    disabled_profile_ids: [],
    has_conflicts: false,
    snapshot_id: "snap-1",
    backup_path: "/tmp/backups/hosts.bak",
  };
  return {
    previewApplyOutcome: vi.fn().mockResolvedValue(inlineOutcome),
    enableAndApply: vi.fn().mockResolvedValue(inlineOutcome),
    listProfiles: vi.fn().mockResolvedValue([]),
  };
});

import {
  previewApplyOutcome,
  enableAndApply,
  listProfiles,
} from "../../lib/tauri";

function makeProfile(overrides: Partial<Profile> = {}): Profile {
  return {
    id: "p1",
    name: "dev",
    description: null,
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
describe("Profile store atoms", () => {
  const store = getDefaultStore();

  it("profilesAtom defaults to empty array", () => {
    const profiles = store.get(profilesAtom);
    expect(profiles).toEqual([]);
  });

  it("selectedProfileIdAtom defaults to null", () => {
    const id = store.get(selectedProfileIdAtom);
    expect(id).toBeNull();
  });

  it("isApplyingAtom defaults to false", () => {
    const applying = store.get(isApplyingAtom);
    expect(applying).toBe(false);
  });

  it("errorAtom defaults to null", () => {
    const error = store.get(errorAtom);
    expect(error).toBeNull();
  });

  it("can set profiles", () => {
    const testProfiles = [
      {
        id: "550e8400-e29b-41d4-a716-446655440000",
        name: "dev",
        description: null,
        enabled: true,
        protected: false,
        tags: [],
        rules: [],
        mode: "hosts" as const,
        created_at: "2024-01-01T00:00:00Z",
        updated_at: "2024-01-01T00:00:00Z",
      },
    ];
    store.set(profilesAtom, testProfiles);
    expect(store.get(profilesAtom)).toHaveLength(1);
    expect(store.get(profilesAtom)[0].name).toBe("dev");
  });

  it("can set selected profile id", () => {
    store.set(selectedProfileIdAtom, "550e8400-e29b-41d4-a716-446655440000");
    expect(store.get(selectedProfileIdAtom)).toBe(
      "550e8400-e29b-41d4-a716-446655440000"
    );
  });

  it("can set applying state", () => {
    store.set(isApplyingAtom, true);
    expect(store.get(isApplyingAtom)).toBe(true);
  });

  it("can set error", () => {
    const testError = "Storage: test error";
    store.set(errorAtom, testError);
    expect(store.get(errorAtom)).toEqual(testError);
  });
});

describe("quickApplyToggleAtom (issue #127)", () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(profilesAtom, []);
    store.set(isApplyingAtom, false);
    store.set(applyResultAtom, null);
    store.set(applyErrorAtom, null);
    store.set(applyConfirmOpenAtom, false);
    store.set(applyPlanAtom, null);
    store.set(applyTargetAtom, null);
    store.set(quickApplyOutcomeAtom, null);
    store.set(isQuickApplyToastOpenAtom, false);
    // Re-establish safe default outcomes (vi.clearAllMocks wipes them).
    (previewApplyOutcome as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue(safeOutcome);
    (enableAndApply as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue(safeOutcome);
    (listProfiles as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue([]);
  });

  // Happy path (Quick Apply route): preview returns a safe outcome,
  // decideApplyMode returns 'quick_apply', enableAndApply fires,
  // quickApplyOutcomeAtom + isQuickApplyToastOpenAtom land.
  it("happy path: safe preview -> enableAndApply -> toast opens", async () => {
    const store = getDefaultStore();
    const profile = makeProfile({ id: "p1", enabled: false });
    store.set(profilesAtom, [profile]);
    const refreshed = makeProfile({ id: "p1", enabled: true });
    (listProfiles as unknown as { mockResolvedValueOnce: (v: Profile[]) => void })
      .mockResolvedValueOnce([refreshed]);

    await store.set(quickApplyToggleAtom, { id: "p1", enabled: true });

    expect(previewApplyOutcome).toHaveBeenCalledWith("p1", true);
    expect(enableAndApply).toHaveBeenCalledWith("p1", true, true);
    expect(listProfiles).toHaveBeenCalledTimes(1);
    expect(store.get(applyResultAtom)).toBe("success");
    expect(store.get(applyErrorAtom)).toBeNull();
    expect(store.get(isApplyingAtom)).toBe(false);
    expect(store.get(profilesAtom)).toEqual([refreshed]);
    expect(store.get(quickApplyOutcomeAtom)).toEqual(safeOutcome);
    expect(store.get(isQuickApplyToastOpenAtom)).toBe(true);
    // Dialog must not have opened on the quick path.
    expect(store.get(applyConfirmOpenAtom)).toBe(false);
  });

  // Error path: preview rejects (e.g. backend error). No enableAndApply,
  // error state surfaces, isApplying resets.
  it("error path: previewApplyOutcome rejects -> error state", async () => {
    const store = getDefaultStore();
    const profile = makeProfile({ id: "p1", enabled: false });
    store.set(profilesAtom, [profile]);
    (previewApplyOutcome as unknown as { mockRejectedValueOnce: (v: unknown) => void })
      .mockRejectedValueOnce(new Error("permission denied"));

    await store.set(quickApplyToggleAtom, { id: "p1", enabled: true });

    expect(previewApplyOutcome).toHaveBeenCalledWith("p1", true);
    expect(enableAndApply).not.toHaveBeenCalled();
    expect(listProfiles).not.toHaveBeenCalled();
    expect(store.get(applyResultAtom)).toBe("error");
    expect(store.get(applyErrorAtom)).toBe("permission denied");
    expect(store.get(isApplyingAtom)).toBe(false);
    expect(store.get(profilesAtom)).toEqual([profile]);
    expect(store.get(isQuickApplyToastOpenAtom)).toBe(false);
  });

  // Policy: conflicts in preview -> require_preview -> dialog opens,
  // no enableAndApply call.
  it("conflicts in preview -> opens dialog, no write", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    const conflictOutcome: ApplyOutcome = {
      ...safeOutcome,
      has_conflicts: true,
      plan: {
        ...safePlan,
        conflicts: [{ domain: "x.example", rules: [] }],
      },
    };
    (previewApplyOutcome as unknown as { mockResolvedValueOnce: (v: unknown) => void })
      .mockResolvedValueOnce(conflictOutcome);

    await store.set(quickApplyToggleAtom, { id: "p1", enabled: true });

    expect(previewApplyOutcome).toHaveBeenCalled();
    expect(enableAndApply).not.toHaveBeenCalled();
    expect(store.get(applyConfirmOpenAtom)).toBe(true);
    expect(store.get(applyPlanAtom)).toEqual(conflictOutcome.plan);
    expect(store.get(applyTargetAtom)).toEqual({ id: "p1", enabled: true });
    expect(store.get(isApplyingAtom)).toBe(false);
    expect(store.get(isQuickApplyToastOpenAtom)).toBe(false);
  });

  // Policy: bulk changes (>100 added+removed) -> require_preview.
  it("bulk changes (>100) in preview -> opens dialog", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    const bulkOutcome: ApplyOutcome = {
      ...safeOutcome,
      added_count: 80,
      removed_count: 25, // 105 > DESTRUCTIVE_THRESHOLD (100)
    };
    (previewApplyOutcome as unknown as { mockResolvedValueOnce: (v: unknown) => void })
      .mockResolvedValueOnce(bulkOutcome);

    await store.set(quickApplyToggleAtom, { id: "p1", enabled: true });

    expect(previewApplyOutcome).toHaveBeenCalled();
    expect(enableAndApply).not.toHaveBeenCalled();
    expect(store.get(applyConfirmOpenAtom)).toBe(true);
  });

  // Policy: would disable another hosts profile -> require_preview.
  it("disabled_profile_ids in preview -> opens dialog", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    const wouldDisable: ApplyOutcome = {
      ...safeOutcome,
      disabled_profile_ids: ["other-id"],
    };
    (previewApplyOutcome as unknown as { mockResolvedValueOnce: (v: unknown) => void })
      .mockResolvedValueOnce(wouldDisable);

    await store.set(quickApplyToggleAtom, { id: "p1", enabled: true });

    expect(previewApplyOutcome).toHaveBeenCalled();
    expect(enableAndApply).not.toHaveBeenCalled();
    expect(store.get(applyConfirmOpenAtom)).toBe(true);
  });

  // Modifier: forcePreview (Cmd/Option) overrides policy — opens the
  // dialog even when the preview outcome would have been quick-applyable.
  it("forcePreview modifier opens dialog even when preview is safe", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);

    await store.set(quickApplyToggleAtom, {
      id: "p1",
      enabled: true,
      forcePreview: true,
    });

    expect(previewApplyOutcome).toHaveBeenCalled();
    expect(enableAndApply).not.toHaveBeenCalled();
    expect(store.get(applyConfirmOpenAtom)).toBe(true);
    expect(store.get(applyTargetAtom)).toEqual({ id: "p1", enabled: true });
  });

  // Refs #127: server-side TOCTOU backstop. The unlocked preview looked
  // safe (quick_apply), so enableAndApply is called — but the server
  // re-checked under the lock, found the change destructive, and rejected
  // with MhostError::PreviewRequired ({ PreviewRequired: "..." }). The atom
  // must fall back to the dialog with a fresh preview, not surface an error.
  it("server rejects with PreviewRequired -> refetches preview and opens dialog", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);

    // First preview (pre-write) is safe → decideApplyMode returns quick_apply.
    // Refetch (post-rejection) returns the now-conflicting outcome.
    const conflictOutcome: ApplyOutcome = {
      ...safeOutcome,
      has_conflicts: true,
      plan: { ...safePlan, conflicts: [{ domain: "x.example", rules: [] }] },
    };
    const previewMock = previewApplyOutcome as unknown as {
      mockResolvedValueOnce: (v: unknown) => typeof previewMock;
    };
    previewMock.mockResolvedValueOnce(safeOutcome); // pre-write preview: safe
    previewMock.mockResolvedValueOnce(conflictOutcome); // refetch after rejection
    (enableAndApply as unknown as { mockRejectedValueOnce: (v: unknown) => void })
      .mockRejectedValueOnce({ PreviewRequired: "conflicts detected" });

    await store.set(quickApplyToggleAtom, { id: "p1", enabled: true });

    expect(enableAndApply).toHaveBeenCalledWith("p1", true, true);
    // Preview fetched twice: once pre-write, once after the rejection.
    expect(previewApplyOutcome).toHaveBeenCalledTimes(2);
    expect(store.get(applyConfirmOpenAtom)).toBe(true);
    expect(store.get(applyPlanAtom)).toEqual(conflictOutcome.plan);
    expect(store.get(applyTargetAtom)).toEqual({ id: "p1", enabled: true });
    // Not surfaced as an error, and no success toast.
    expect(store.get(applyErrorAtom)).toBeNull();
    expect(store.get(isQuickApplyToastOpenAtom)).toBe(false);
    expect(store.get(isApplyingAtom)).toBe(false);
  });

  // PreviewRequired fallback but the refetch itself fails → surface an error,
  // don't get stuck applying.
  it("PreviewRequired then refetch failure -> error state, isApplying reset", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);

    const previewMock = previewApplyOutcome as unknown as {
      mockResolvedValueOnce: (v: unknown) => typeof previewMock;
      mockRejectedValueOnce: (v: unknown) => typeof previewMock;
    };
    previewMock.mockResolvedValueOnce(safeOutcome); // pre-write preview: safe
    previewMock.mockRejectedValueOnce(new Error("refetch boom")); // refetch fails
    (enableAndApply as unknown as { mockRejectedValueOnce: (v: unknown) => void })
      .mockRejectedValueOnce({ PreviewRequired: "conflicts detected" });

    await store.set(quickApplyToggleAtom, { id: "p1", enabled: true });

    expect(store.get(applyConfirmOpenAtom)).toBe(false);
    expect(store.get(applyResultAtom)).toBe("error");
    expect(store.get(applyErrorAtom)).toBe("refetch boom");
    expect(store.get(isApplyingAtom)).toBe(false);
  });

  // M2: opening the preview dialog (require_preview path) must dismiss any
  // lingering toast so it can't overlay the dialog.
  it("dismisses a stale toast when the preview path opens the dialog", async () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", enabled: false })]);
    // Simulate a leftover open toast from a previous quick apply.
    store.set(isQuickApplyToastOpenAtom, true);

    const conflictOutcome: ApplyOutcome = {
      ...safeOutcome,
      has_conflicts: true,
      plan: { ...safePlan, conflicts: [{ domain: "x.example", rules: [] }] },
    };
    (previewApplyOutcome as unknown as { mockResolvedValueOnce: (v: unknown) => void })
      .mockResolvedValueOnce(conflictOutcome);

    await store.set(quickApplyToggleAtom, { id: "p1", enabled: true });

    expect(store.get(applyConfirmOpenAtom)).toBe(true);
    expect(store.get(isQuickApplyToastOpenAtom)).toBe(false);
  });
});
