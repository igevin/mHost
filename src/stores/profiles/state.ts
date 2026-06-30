import { atom } from "jotai";
import type { Profile } from "../../types";

// ---- Base atoms ----

export const profilesAtom = atom<Profile[]>([]);
export const selectedProfileIdAtom = atom<string | null>(null);
export const isApplyingAtom = atom(false);
export const errorAtom = atom<string | null>(null);
export const isLoadingAtom = atom(false);

// ---- Derived atoms ----

export const selectedProfileAtom = atom((get) => {
  const profiles = get(profilesAtom);
  const id = get(selectedProfileIdAtom);
  return profiles.find((p) => p.id === id) ?? null;
});

export const enabledProfileAtom = atom((get) => {
  const profiles = get(profilesAtom);
  return profiles.find((p) => p.enabled) ?? null;
});

// ---- Apply confirm dialog atoms ----

export const applyConfirmOpenAtom = atom(false);
export const applyPlanAtom = atom<import("../../types").ApplyPlan | null>(null);
export const applyResultAtom = atom<"success" | "error" | null>(null);
export const applyErrorAtom = atom<string | null>(null);
export const applyTargetAtom = atom<{ id: string; enabled: boolean } | null>(null);

// ---- Snapshot atoms ----

export const snapshotsAtom = atom<import("../../types").SnapshotMeta[]>([]);
export const isLoadingSnapshotsAtom = atom(false);
export const snapshotErrorAtom = atom<string | null>(null);
