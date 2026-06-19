import { atom } from "jotai";
import type { Profile, ApplyPlan } from "../types";
import {
  listProfiles,
  getProfile,
  createProfile,
  updateProfile,
  deleteProfile,
  setProfileEnabled,
  generateApplyPlan,
  applyHosts,
} from "../lib/tauri";

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

// ---- Async action atoms ----

export const fetchProfilesAtom = atom(null, async (_get, set) => {
  set(isLoadingAtom, true);
  set(errorAtom, null);
  try {
    const profiles = await listProfiles();
    set(profilesAtom, profiles);
  } catch (err) {
    set(errorAtom, err instanceof Error ? err.message : String(err));
  } finally {
    set(isLoadingAtom, false);
  }
});

export const fetchProfileAtom = atom(null, async (_get, set, id: string) => {
  set(isLoadingAtom, true);
  set(errorAtom, null);
  try {
    const profile = await getProfile(id);
    set(profilesAtom, (prev) => {
      const idx = prev.findIndex((p) => p.id === id);
      if (idx >= 0) {
        const next = [...prev];
        next[idx] = profile;
        return next;
      }
      return [...prev, profile];
    });
  } catch (err) {
    set(errorAtom, err instanceof Error ? err.message : String(err));
  } finally {
    set(isLoadingAtom, false);
  }
});

export const createProfileAtom = atom(null, async (_get, set, name: string) => {
  set(isLoadingAtom, true);
  set(errorAtom, null);
  try {
    const profile = await createProfile(name);
    set(profilesAtom, (prev) => [...prev, profile]);
    return profile;
  } catch (err) {
    set(errorAtom, err instanceof Error ? err.message : String(err));
    throw err;
  } finally {
    set(isLoadingAtom, false);
  }
});

export const updateProfileAtom = atom(
  null,
  async (_get, set, profile: Profile) => {
    set(isLoadingAtom, true);
    set(errorAtom, null);
    try {
      const updated = await updateProfile(profile);
      set(profilesAtom, (prev) =>
        prev.map((p) => (p.id === updated.id ? updated : p)),
      );
      return updated;
    } catch (err) {
      set(errorAtom, err instanceof Error ? err.message : String(err));
      throw err;
    } finally {
      set(isLoadingAtom, false);
    }
  },
);

export const deleteProfileAtom = atom(
  null,
  async (_get, set, id: string) => {
    set(isLoadingAtom, true);
    set(errorAtom, null);
    try {
      await deleteProfile(id);
      set(profilesAtom, (prev) => prev.filter((p) => p.id !== id));
      set(selectedProfileIdAtom, (prev) => (prev === id ? null : prev));
    } catch (err) {
      set(errorAtom, err instanceof Error ? err.message : String(err));
      throw err;
    } finally {
      set(isLoadingAtom, false);
    }
  },
);

export const toggleProfileEnabledAtom = atom(
  null,
  async (get, set, id: string) => {
    const profiles = get(profilesAtom);
    const target = profiles.find((p) => p.id === id);
    if (!target) return;

    const newEnabled = !target.enabled;

    // Phase 0: only one profile can be enabled at a time
    if (newEnabled) {
      set(profilesAtom, (prev) =>
        prev.map((p) => (p.id === id ? { ...p, enabled: true } : { ...p, enabled: false })),
      );
    } else {
      set(profilesAtom, (prev) =>
        prev.map((p) => (p.id === id ? { ...p, enabled: false } : p)),
      );
    }

    set(isLoadingAtom, true);
    set(errorAtom, null);
    try {
      const updated = await setProfileEnabled(id, newEnabled);
      set(profilesAtom, (prev) =>
        prev.map((p) => {
          if (p.id === updated.id) return updated;
          // Ensure only one enabled after server response
          if (newEnabled && p.id !== updated.id) return { ...p, enabled: false };
          return p;
        }),
      );
    } catch (err) {
      set(errorAtom, err instanceof Error ? err.message : String(err));
      // Revert optimistic update
      set(profilesAtom, (prev) =>
        prev.map((p) => (p.id === id ? target : p)),
      );
      throw err;
    } finally {
      set(isLoadingAtom, false);
    }
  },
);

export const generateApplyPlanActionAtom = atom(
  null,
  async (_get, set) => {
    set(isLoadingAtom, true);
    set(errorAtom, null);
    try {
      const plan = await generateApplyPlan();
      set(applyPlanAtom, plan);
      return plan;
    } catch (err) {
      set(errorAtom, err instanceof Error ? err.message : String(err));
      throw err;
    } finally {
      set(isLoadingAtom, false);
    }
  },
);

export const applyHostsActionAtom = atom(null, async (_get, set) => {
  set(isApplyingAtom, true);
  set(errorAtom, null);
  try {
    await applyHosts();
  } catch (err) {
    set(errorAtom, err instanceof Error ? err.message : String(err));
    throw err;
  } finally {
    set(isApplyingAtom, false);
  }
});
