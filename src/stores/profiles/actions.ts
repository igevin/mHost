import { atom } from "jotai";
import type { Profile, AdBlockResponse } from "../../types";
import {
  listProfiles,
  getProfile,
  createProfile,
  updateProfile,
  deleteProfile,
  setProfileEnabled,
  enableAndApply,
  rollbackHosts,
  generatePreviewPlan,
  saveSnapshot,
  listSnapshots,
  loadSnapshot,
  deleteSnapshot,
  getDnsMode,
  getDnsStatus,
  setDnsMode,
  reloadDnsRules,
  listDnsProfiles,
  getAdBlockState,
  setAdBlockEnabled,
  setAdBlockRefreshInterval,
  listAdBlockSources,
  addAdBlockSource,
  removeAdBlockSource,
  setAdBlockSourceEnabled,
  setAdBlockSourceResponse,
  refreshAdBlockSource,
  refreshAllAdBlockSources,
  listAdBlockWhitelist,
  addAdBlockWhitelist,
  removeAdBlockWhitelist,
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
  snapshotsAtom,
  isLoadingSnapshotsAtom,
  snapshotErrorAtom,
  dnsProfilesAtom,
  dnsEnabledAtom,
  dnsStatusAtom,
  isDnsLoadingAtom,
  dnsErrorAtom,
  adBlockStateAtom,
  isAdBlockLoadingAtom,
  adBlockErrorAtom,
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
// 每个 mutating action 在 await 成功后立刻拉一次 `getAdBlockState` 重写
// `adBlockStateAtom`，避免前端手动维护 sources 列表的不变量。读操作的
// fetch（mount 时 / 路由进入时）走 `fetchAdBlockStateAtom`。

export const fetchAdBlockStateAtom = atom(null, async (_get, set) => {
  set(isAdBlockLoadingAtom, true);
  set(adBlockErrorAtom, null);
  try {
    const state = await getAdBlockState();
    set(adBlockStateAtom, state);
  } catch (err) {
    set(adBlockErrorAtom, extractErrorMessage(err));
    throw err;
  } finally {
    set(isAdBlockLoadingAtom, false);
  }
});

export const toggleAdBlockEnabledAtom = atom(
  null,
  async (_get, set, enabled: boolean) => {
    set(isAdBlockLoadingAtom, true);
    set(adBlockErrorAtom, null);
    try {
      await setAdBlockEnabled(enabled);
      const state = await getAdBlockState();
      set(adBlockStateAtom, state);
    } catch (err) {
      set(adBlockErrorAtom, extractErrorMessage(err));
      throw err;
    } finally {
      set(isAdBlockLoadingAtom, false);
    }
  },
);

export const setAdBlockIntervalAtom = atom(
  null,
  async (_get, set, hours: number) => {
    set(adBlockErrorAtom, null);
    try {
      await setAdBlockRefreshInterval(hours);
      const state = await getAdBlockState();
      set(adBlockStateAtom, state);
    } catch (err) {
      set(adBlockErrorAtom, extractErrorMessage(err));
      throw err;
    }
  },
);

export const addAdBlockSourceAtom = atom(
  null,
  async (_get, set, args: { name: string; url: string; response: AdBlockResponse }) => {
    set(isAdBlockLoadingAtom, true);
    set(adBlockErrorAtom, null);
    try {
      await addAdBlockSource(args.name, args.url, args.response);
      const state = await getAdBlockState();
      set(adBlockStateAtom, state);
    } catch (err) {
      set(adBlockErrorAtom, extractErrorMessage(err));
      throw err;
    } finally {
      set(isAdBlockLoadingAtom, false);
    }
  },
);

export const removeAdBlockSourceAtom = atom(
  null,
  async (_get, set, sourceId: string) => {
    set(adBlockErrorAtom, null);
    try {
      await removeAdBlockSource(sourceId);
      const state = await getAdBlockState();
      set(adBlockStateAtom, state);
    } catch (err) {
      set(adBlockErrorAtom, extractErrorMessage(err));
      throw err;
    }
  },
);

export const setAdBlockSourceEnabledAtom = atom(
  null,
  async (_get, set, args: { sourceId: string; enabled: boolean }) => {
    set(adBlockErrorAtom, null);
    try {
      await setAdBlockSourceEnabled(args.sourceId, args.enabled);
      const state = await getAdBlockState();
      set(adBlockStateAtom, state);
    } catch (err) {
      set(adBlockErrorAtom, extractErrorMessage(err));
      throw err;
    }
  },
);

export const setAdBlockSourceResponseAtom = atom(
  null,
  async (_get, set, args: { sourceId: string; response: AdBlockResponse }) => {
    set(adBlockErrorAtom, null);
    try {
      await setAdBlockSourceResponse(args.sourceId, args.response);
      const state = await getAdBlockState();
      set(adBlockStateAtom, state);
    } catch (err) {
      set(adBlockErrorAtom, extractErrorMessage(err));
      throw err;
    }
  },
);

export const refreshAdBlockSourceAtom = atom(
  null,
  async (_get, set, sourceId: string) => {
    set(isAdBlockLoadingAtom, true);
    set(adBlockErrorAtom, null);
    try {
      await refreshAdBlockSource(sourceId);
      const state = await getAdBlockState();
      set(adBlockStateAtom, state);
    } catch (err) {
      set(adBlockErrorAtom, extractErrorMessage(err));
      throw err;
    } finally {
      set(isAdBlockLoadingAtom, false);
    }
  },
);

export const refreshAllAdBlockSourcesAtom = atom(null, async (_get, set) => {
  set(isAdBlockLoadingAtom, true);
  set(adBlockErrorAtom, null);
  try {
    await refreshAllAdBlockSources();
    const state = await getAdBlockState();
    set(adBlockStateAtom, state);
  } catch (err) {
    set(adBlockErrorAtom, extractErrorMessage(err));
    throw err;
  } finally {
    set(isAdBlockLoadingAtom, false);
  }
});

export const addAdBlockWhitelistAtom = atom(
  null,
  async (_get, set, domain: string) => {
    set(adBlockErrorAtom, null);
    try {
      await addAdBlockWhitelist(domain);
      const state = await getAdBlockState();
      set(adBlockStateAtom, state);
    } catch (err) {
      set(adBlockErrorAtom, extractErrorMessage(err));
      throw err;
    }
  },
);

export const removeAdBlockWhitelistAtom = atom(
  null,
  async (_get, set, domain: string) => {
    set(adBlockErrorAtom, null);
    try {
      await removeAdBlockWhitelist(domain);
      const state = await getAdBlockState();
      set(adBlockStateAtom, state);
    } catch (err) {
      set(adBlockErrorAtom, extractErrorMessage(err));
      throw err;
    }
  },
);

// Re-export whitelist helpers that don't need state refresh.
export const fetchAdBlockWhitelistAtom = atom(null, async () => {
  return listAdBlockWhitelist();
});

export const fetchAdBlockSourcesAtom = atom(null, async () => {
  return listAdBlockSources();
});
