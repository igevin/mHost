import { describe, it, expect, vi, beforeEach } from "vitest";
import { getDefaultStore } from "jotai";
import type { Profile } from "../../types";
import {
  profilesAtom,
  selectedProfileIdAtom,
  isApplyingAtom,
  errorAtom,
  applyResultAtom,
  applyErrorAtom,
  quickApplyToggleAtom,
} from "../profiles";

// Mock the Tauri binding so `quickApplyToggleAtom` can run end-to-end
// against deterministic `enableAndApply` / `listProfiles` responses.
// Each test overrides the per-call behavior via the spy's `.mock*` API.
vi.mock("../../lib/tauri", () => ({
  enableAndApply: vi.fn().mockResolvedValue(undefined),
  listProfiles: vi.fn().mockResolvedValue([]),
}));

import { enableAndApply, listProfiles } from "../../lib/tauri";

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

describe("quickApplyToggleAtom (issue #123)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(profilesAtom, []);
    store.set(isApplyingAtom, false);
    store.set(applyResultAtom, null);
    store.set(applyErrorAtom, null);
  });

  // Happy path mirrors `executeApplyAtom` — surface state transitions
  // (`isApplyingAtom` → true, `applyResultAtom` → 'success',
  // `profilesAtom` re-read via `listProfiles`) must all land after
  // `enable_and_apply` resolves.
  it("happy path: enableAndApply then listProfiles refreshes atoms", async () => {
    const store = getDefaultStore();
    const profile = makeProfile({ id: "p1", enabled: false });
    store.set(profilesAtom, [profile]);
    (enableAndApply as unknown as { mockResolvedValueOnce: (v: unknown) => void })
      .mockResolvedValueOnce(undefined);
    const refreshed = makeProfile({ id: "p1", enabled: true });
    (listProfiles as unknown as { mockResolvedValueOnce: (v: Profile[]) => void })
      .mockResolvedValueOnce([refreshed]);

    await store.set(quickApplyToggleAtom, { id: "p1", enabled: true });

    expect(enableAndApply).toHaveBeenCalledWith("p1", true);
    expect(listProfiles).toHaveBeenCalledTimes(1);
    expect(store.get(applyResultAtom)).toBe("success");
    expect(store.get(applyErrorAtom)).toBeNull();
    expect(store.get(isApplyingAtom)).toBe(false);
    expect(store.get(profilesAtom)).toEqual([refreshed]);
  });

  // Error path: any rejection from `enable_and_apply` must be caught,
  // surface as `applyResultAtom = 'error'` + `applyErrorAtom` carrying
  // the message, and `isApplyingAtom` must reset in `finally` so the UI
  // doesn't get stuck on a spinner.
  it("error path: enableAndApply rejects -> error state", async () => {
    const store = getDefaultStore();
    const profile = makeProfile({ id: "p1", enabled: false });
    store.set(profilesAtom, [profile]);
    (enableAndApply as unknown as { mockRejectedValueOnce: (v: unknown) => void })
      .mockRejectedValueOnce(new Error("permission denied"));

    await store.set(quickApplyToggleAtom, { id: "p1", enabled: true });

    expect(enableAndApply).toHaveBeenCalledWith("p1", true);
    expect(listProfiles).not.toHaveBeenCalled();
    expect(store.get(applyResultAtom)).toBe("error");
    expect(store.get(applyErrorAtom)).toBe("permission denied");
    expect(store.get(isApplyingAtom)).toBe(false);
    // profiles are untouched on failure.
    expect(store.get(profilesAtom)).toEqual([profile]);
  });
});
