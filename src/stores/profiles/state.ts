import { atom } from "jotai";
import type { Profile, ApplyPlan } from "../../types";

// ---- Base atoms ----

export const profilesAtom = atom<Profile[]>([]);
export const selectedProfileIdAtom = atom<string | null>(null);
export const applyPlanAtom = atom<ApplyPlan | null>(null);
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
