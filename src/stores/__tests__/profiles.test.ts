import { describe, it, expect } from "vitest";
import { atom, getDefaultStore } from "jotai";
import {
  profilesAtom,
  selectedProfileIdAtom,
  applyPlanAtom,
  isApplyingAtom,
  errorAtom,
} from "../profiles";

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

  it("applyPlanAtom defaults to null", () => {
    const plan = store.get(applyPlanAtom);
    expect(plan).toBeNull();
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
    const testError = { type: "Storage", message: "test error" };
    store.set(errorAtom, testError);
    expect(store.get(errorAtom)).toEqual(testError);
  });
});
