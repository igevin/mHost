import { atom } from "jotai";
import type { Profile } from "../../types";
import {
  listProfiles,
  getProfile,
  createProfile,
  updateProfile,
  deleteProfile,
  enableAndApply,
  rollbackHosts,
  generatePreviewPlan,
} from "../../lib/tauri";
import { extractErrorMessage } from "../../lib/error";
import {
  profilesAtom,
  selectedProfileIdAtom,
  isApplyingAtom,
  errorAtom,
  isLoadingAtom,
  applyConfirmOpenAtom,
  applyPlanAtom,
  applyResultAtom,
  applyErrorAtom,
  applyTargetAtom,
} from "./state";

// ---- Async action atoms ----

export const fetchProfilesAtom = atom(null, async (_get, set) => {
  set(isLoadingAtom, true);
  set(errorAtom, null);
  try {
    const profiles = await listProfiles();
    set(profilesAtom, profiles);
  } catch (err) {
    set(errorAtom, extractErrorMessage(err));
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
    set(errorAtom, extractErrorMessage(err));
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
    set(errorAtom, extractErrorMessage(err));
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
      set(errorAtom, extractErrorMessage(err));
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
      set(errorAtom, extractErrorMessage(err));
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

    // Save a full snapshot of all profiles for rollback
    const previousProfiles = [...profiles];

    // Optimistic UI update
    if (newEnabled) {
      set(profilesAtom, (prev) =>
        prev.map((p) => (p.id === id ? { ...p, enabled: true } : { ...p, enabled: false })),
      );
    } else {
      set(profilesAtom, (prev) =>
        prev.map((p) => (p.id === id ? { ...p, enabled: false } : p)),
      );
    }

    set(isApplyingAtom, true);
    set(errorAtom, null);
    try {
      // Backend handles: toggle enabled + generate plan + write hosts + flush DNS
      // All in one atomic operation (triggers macOS auth prompt).
      await enableAndApply(id, newEnabled);

      // Reload profiles from backend to sync state
      const updated = await listProfiles();
      set(profilesAtom, updated);
    } catch (err) {
      set(errorAtom, extractErrorMessage(err));
      // Revert optimistic update: restore full profiles snapshot
      set(profilesAtom, previousProfiles);
      throw err;
    } finally {
      set(isApplyingAtom, false);
    }
  },
);

export const rollbackHostsActionAtom = atom(null, async (_get, set) => {
  try {
    await rollbackHosts();
    const profiles = await listProfiles();
    set(profilesAtom, profiles);
  } catch (e) {
    console.error("Rollback failed:", e);
    throw e;
  }
});

export const previewApplyAtom = atom(
  null,
  async (_get, set, { id, enabled }: { id: string; enabled: boolean }) => {
    set(applyResultAtom, null);
    set(applyErrorAtom, null);
    try {
      const plan = await generatePreviewPlan(id, enabled);
      set(applyPlanAtom, plan);
      set(applyTargetAtom, { id, enabled });
      set(applyConfirmOpenAtom, true);
    } catch (err) {
      set(applyErrorAtom, extractErrorMessage(err));
    }
  },
);

export const executeApplyAtom = atom(null, async (get, set) => {
  const target = get(applyTargetAtom);
  if (!target) return;
  const { id, enabled } = target;

  set(isApplyingAtom, true);
  set(applyResultAtom, null);
  set(applyErrorAtom, null);
  try {
    await enableAndApply(id, enabled);
    set(applyResultAtom, "success");
    const profiles = await listProfiles();
    set(profilesAtom, profiles);
  } catch (err) {
    set(applyResultAtom, "error");
    set(applyErrorAtom, extractErrorMessage(err));
  } finally {
    set(isApplyingAtom, false);
  }
});

export const closeApplyConfirmAtom = atom(null, (_get, set) => {
  set(applyConfirmOpenAtom, false);
  set(applyPlanAtom, null);
  set(applyResultAtom, null);
  set(applyErrorAtom, null);
  set(applyTargetAtom, null);
});
