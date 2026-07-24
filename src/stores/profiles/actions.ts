import { atom } from "jotai";
import type { Profile } from "../../types";
import {
  listProfiles,
  getProfile,
  createProfile,
  updateProfile,
  deleteProfile,
  setProfileEnabled,
  enableAndApply,
  previewApplyOutcome,
  rollbackHosts,
  saveSnapshot,
  listSnapshots,
  loadSnapshot,
  deleteSnapshot,
  getDnsMode,
  getDnsStatus,
  setDnsMode,
  reloadDnsRules,
  listDnsProfiles,
} from "../../lib/tauri";
import { extractErrorMessage, isPreviewRequired } from "../../lib/error";
import { decideApplyMode } from "../../lib/applyPolicy";
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
  snapshotsAtom,
  isLoadingSnapshotsAtom,
  snapshotErrorAtom,
  dnsProfilesAtom,
  dnsEnabledAtom,
  dnsStatusAtom,
  isDnsLoadingAtom,
  dnsErrorAtom,
  quickApplyOutcomeAtom,
  isQuickApplyToastOpenAtom,
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
      const outcome = await previewApplyOutcome(id, enabled);
      set(applyPlanAtom, outcome.plan);
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
    const outcome = await enableAndApply(id, enabled);
    set(applyResultAtom, "success");
    // Refs #127: surface outcome to QuickApplyToast for summary + View Diff + Rollback.
    set(quickApplyOutcomeAtom, outcome);
    set(isQuickApplyToastOpenAtom, true);
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

// Refs #127: Quick Apply hosts toggle. Preview → decide → write OR dialog.
//
// 1. Always call `previewApplyOutcome` first (read-only, no /etc/hosts write).
// 2. Run client-side `decideApplyMode` to classify the outcome (fast path:
//    destructive toggles open the dialog WITHOUT attempting a write).
// 3. If `require_preview`, or if the user held Cmd/Option (forcePreview),
//    open the existing preview dialog with the plan — the user confirms
//    via `executeApplyAtom`.
// 4. Otherwise, call `enableAndApply(id, enabled, requireSafe=true)`. The
//    Rust side re-checks the policy UNDER the apply lock; if state changed
//    since the unlocked preview (concurrent tray toggle, external
//    /etc/hosts edit) it rejects with `PreviewRequired`, and we fall back
//    to the dialog with a freshly-fetched plan. This closes the
//    preview/apply TOCTOU while keeping the common path a single write.
//
// Surface state mirrors `executeApplyAtom` so the existing
// ApplyConfirmDialog / ApplyStatus wiring still lights up.
export const quickApplyToggleAtom = atom(
  null,
  async (
    _get,
    set,
    {
      id,
      enabled,
      forcePreview = false,
    }: { id: string; enabled: boolean; forcePreview?: boolean },
  ) => {
    set(isApplyingAtom, true);
    set(applyResultAtom, null);
    set(applyErrorAtom, null);
    // Dismiss any lingering toast from a previous quick apply so it can't
    // overlay the preview dialog we may open below (toast z-index > dialog).
    set(isQuickApplyToastOpenAtom, false);
    try {
      const preview = await previewApplyOutcome(id, enabled);
      const mode = decideApplyMode(preview);

      if (mode === "require_preview" || forcePreview) {
        // Open the preview dialog. User confirms → executeApplyAtom writes
        // (which also surfaces the outcome to the toast).
        set(applyPlanAtom, preview.plan);
        set(applyTargetAtom, { id, enabled });
        set(applyConfirmOpenAtom, true);
        return;
      }

      // QuickApply path: write directly (server re-checks policy under lock).
      const outcome = await enableAndApply(id, enabled, true);
      set(applyResultAtom, "success");
      set(quickApplyOutcomeAtom, outcome);
      set(isQuickApplyToastOpenAtom, true);
      const profiles = await listProfiles();
      set(profilesAtom, profiles);
    } catch (err) {
      if (isPreviewRequired(err)) {
        // Server rejected the quick apply under the lock (state changed since
        // the unlocked preview). Fall back to the dialog with a fresh plan.
        try {
          const fresh = await previewApplyOutcome(id, enabled);
          set(applyPlanAtom, fresh.plan);
          set(applyTargetAtom, { id, enabled });
          set(applyConfirmOpenAtom, true);
        } catch (refetchErr) {
          set(applyResultAtom, "error");
          set(applyErrorAtom, extractErrorMessage(refetchErr));
        }
        return;
      }
      set(applyResultAtom, "error");
      set(applyErrorAtom, extractErrorMessage(err));
    } finally {
      set(isApplyingAtom, false);
    }
  },
);

// ---- Snapshot action atoms ----

export const fetchSnapshotsAtom = atom(null, async (_get, set) => {
  set(isLoadingSnapshotsAtom, true);
  set(snapshotErrorAtom, null);
  try {
    const snapshots = await listSnapshots();
    set(snapshotsAtom, snapshots);
  } catch (err) {
    set(snapshotErrorAtom, extractErrorMessage(err));
  } finally {
    set(isLoadingSnapshotsAtom, false);
  }
});

/** Maximum snapshots to keep in the in-memory list. Older snapshots beyond
 * this cap are dropped from the atom (but the on-disk files in mhost's
 * storage backend are kept — see apply logic for cleanup if needed).
 *
 * **fix (P-F5, issue #90)**: was unbounded; long-running mHost installs
 * would accumulate hundreds of snapshots and SnapshotPanel would render
 * every one. 50 is a reasonable cap — covers ~weeks of frequent backups
 * without dominating UI list render cost.
 */
const MAX_SNAPSHOTS = 50;

export const saveSnapshotAtom = atom(null, async (get, set, { name, description }: { name: string; description?: string }) => {
  set(snapshotErrorAtom, null);
  try {
    const meta = await saveSnapshot(name, description);
    set(snapshotsAtom, [meta, ...get(snapshotsAtom)].slice(0, MAX_SNAPSHOTS));
  } catch (err) {
    set(snapshotErrorAtom, extractErrorMessage(err));
    throw err;
  }
});

export const loadSnapshotAtom = atom(null, async (_get, set, id: string) => {
  set(snapshotErrorAtom, null);
  try {
    await loadSnapshot(id);
    // Refresh profiles after rollback
    const profiles = await listProfiles();
    set(profilesAtom, profiles);
  } catch (err) {
    set(snapshotErrorAtom, extractErrorMessage(err));
    throw err;
  }
});

export const deleteSnapshotAtom = atom(null, async (get, set, id: string) => {
  set(snapshotErrorAtom, null);
  try {
    await deleteSnapshot(id);
    set(snapshotsAtom, get(snapshotsAtom).filter((s) => s.id !== id));
  } catch (err) {
    set(snapshotErrorAtom, extractErrorMessage(err));
    throw err;
  }
});

// ---- DNS action atoms ----

export const fetchDnsModeAtom = atom(null, async (_get, set) => {
  set(isDnsLoadingAtom, true);
  set(dnsErrorAtom, null);
  try {
    const enabled = await getDnsMode();
    set(dnsEnabledAtom, enabled);
    const status = await getDnsStatus();
    set(dnsStatusAtom, status);
  } catch (err) {
    set(dnsErrorAtom, extractErrorMessage(err));
    set(dnsStatusAtom, null);
  } finally {
    set(isDnsLoadingAtom, false);
  }
});

export const toggleDnsModeAtom = atom(null, async (_get, set, enabled: boolean) => {
  set(isDnsLoadingAtom, true);
  set(dnsErrorAtom, null);
  try {
    await setDnsMode(enabled);
    set(dnsEnabledAtom, enabled);
    const status = await getDnsStatus();
    set(dnsStatusAtom, status);
  } catch (err) {
    set(dnsErrorAtom, extractErrorMessage(err));
    set(dnsStatusAtom, null);
    throw err;
  } finally {
    set(isDnsLoadingAtom, false);
  }
});

export const fetchDnsProfilesAtom = atom(null, async (_get, set) => {
  set(isDnsLoadingAtom, true);
  set(dnsErrorAtom, null);
  try {
    const profiles = await listDnsProfiles();
    set(dnsProfilesAtom, profiles);
  } catch (err) {
    set(dnsErrorAtom, extractErrorMessage(err));
  } finally {
    set(isDnsLoadingAtom, false);
  }
});

export const createDnsProfileAtom = atom(null, async (_get, set, name: string) => {
  set(isDnsLoadingAtom, true);
  set(dnsErrorAtom, null);
  try {
    const profile = await createProfile(name, "dns");
    set(dnsProfilesAtom, (prev) => [...prev, profile]);
    return profile;
  } catch (err) {
    set(dnsErrorAtom, extractErrorMessage(err));
    throw err;
  } finally {
    set(isDnsLoadingAtom, false);
  }
});

export const reloadDnsRulesAtom = atom(null, async (_get, set) => {
  set(isDnsLoadingAtom, true);
  set(dnsErrorAtom, null);
  try {
    await reloadDnsRules();
    const status = await getDnsStatus();
    set(dnsStatusAtom, status);
  } catch (err) {
    set(dnsErrorAtom, extractErrorMessage(err));
    set(dnsStatusAtom, null);
    throw err;
  } finally {
    set(isDnsLoadingAtom, false);
  }
});

export const updateDnsProfileAtom = atom(
  null,
  async (_get, set, profile: Profile) => {
    set(isDnsLoadingAtom, true);
    set(dnsErrorAtom, null);
    try {
      const updated = await updateProfile(profile);
      set(dnsProfilesAtom, (prev) =>
        prev.map((p) => (p.id === updated.id ? updated : p)),
      );
      return updated;
    } catch (err) {
      set(dnsErrorAtom, extractErrorMessage(err));
      throw err;
    } finally {
      set(isDnsLoadingAtom, false);
    }
  },
);

export const deleteDnsProfileAtom = atom(
  null,
  async (_get, set, id: string) => {
    set(isDnsLoadingAtom, true);
    set(dnsErrorAtom, null);
    try {
      await deleteProfile(id);
      set(dnsProfilesAtom, (prev) => prev.filter((p) => p.id !== id));
    } catch (err) {
      set(dnsErrorAtom, extractErrorMessage(err));
      throw err;
    } finally {
      set(isDnsLoadingAtom, false);
    }
  },
);

export const toggleDnsProfileEnabledAtom = atom(
  null,
  async (_get, set, { id, enabled }: { id: string; enabled: boolean }) => {
    set(isDnsLoadingAtom, true);
    set(dnsErrorAtom, null);
    try {
      await setProfileEnabled(id, enabled);
      const profiles = await listDnsProfiles();
      set(dnsProfilesAtom, profiles);
      const status = await getDnsStatus();
      set(dnsStatusAtom, status);
    } catch (err) {
      set(dnsErrorAtom, extractErrorMessage(err));
      throw err;
    } finally {
      set(isDnsLoadingAtom, false);
    }
  },
);
