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
  cancelDnsMode,
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

// issue #123: Quick Apply path for Hosts profile toggles. Reuses the same
// Rust `enable_and_apply` command as `executeApplyAtom`; the only difference
// is that we skip the preview dialog and write directly when the user has
// the Quick Apply preference enabled.
//
// Surface state mirrors `executeApplyAtom` on purpose: errors land in
// `applyErrorAtom` and success in `applyResultAtom` so the existing
// ApplyConfirmDialog / ApplyStatus wiring lights up without any
// component-level changes. We deliberately reuse the standard apply
// feedback rather than introducing a new toast/result UI — the follow-up
// issue #127 will revisit structured feedback once real usage is in.
export const quickApplyToggleAtom = atom(
  null,
  async (_get, set, { id, enabled }: { id: string; enabled: boolean }) => {
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

/**
 * Module-level holder for the in-flight DNS toggle's AbortController
 * (issue #149). The Settings page Cancel button calls
 * {@link cancelActiveDnsToggle} which aborts it and fires the backend
 * `cancel_dns_mode` IPC to drive the Rust-side rollback.
 *
 * Only one DNS toggle can be in flight at a time (the backend serializes
 * via `dns_lock`), so a single slot suffices. Stored outside of Jotai
 * intentionally — the AbortController is mutable imperative state, not
 * something we want to track through atom subscribers (would cause every
 * component reading `isDnsLoadingAtom` to re-render on each `.abort()`
 * call).
 */
let activeDnsToggleController: AbortController | null = null;

/**
 * Abort the in-flight DNS toggle (issue #149 Settings cancel button).
 *
 * Fires both:
 *   1. `controller.abort()` — flips the local `cancelled` flag so the
 *      `toggleDnsModeAtom` catch path treats the eventual IPC return
 *      as a user cancel (no error toast, UI reverts).
 *   2. `cancelDnsMode()` IPC — fires the backend `CancellationToken`
 *      so the Rust side actually rolls back the in-flight enable/disable.
 *
 * Tauri 2's `invoke()` does NOT natively propagate AbortSignal to the
 * backend — the Rust future keeps running after abort. Both signals
 * are therefore required: the abort event handler on the controller
 * fires `cancelDnsMode()` (step 2), and the local flag (step 1) tells
 * the JS code path to treat the late IPC return as a cancellation.
 *
 * Safe to call when no toggle is in flight (no-op).
 */
export function cancelActiveDnsToggle(): void {
  const ctrl = activeDnsToggleController;
  if (!ctrl) return;
  ctrl.abort();
  activeDnsToggleController = null;
}

export const toggleDnsModeAtom = atom(null, async (_get, set, enabled: boolean) => {
  set(isDnsLoadingAtom, true);
  set(dnsErrorAtom, null);

  const ctrl = new AbortController();
  activeDnsToggleController = ctrl;

  // 跟踪是否被用户主动 cancel（issue #149）：abort 信号触发时记下
  // 这个 flag,在 catch 块里用它区分「用户 cancel」和「真错误」。
  // Tauri 2 invoke 不原生支持 signal,所以我们用本地 flag 而非依赖
  // DOMException(AbortError) 的 reject 类型。
  let cancelled = false;
  ctrl.signal.addEventListener("abort", () => {
    cancelled = true;
    // 同步触发后端 cancel_dns_mode IPC,Rust 端的 CancellationToken
    // 点亮后会走 rollback。后端最终返回 Err(Cancelled),但 JS 端
    // 不靠这个 reject 来识别 cancel —— 我们已经在 cancelled 标志里
    // 知道了,这里单独处理就行。
    cancelDnsMode().catch((e) => {
      // 后端拿不到 cancel 信号时 recovery marker 兜底,这里仅打日志
      console.error("[mHost] cancelDnsMode IPC failed:", e);
    });
  });

  try {
    await setDnsMode(enabled, { signal: ctrl.signal });
    set(dnsEnabledAtom, enabled);
    const status = await getDnsStatus();
    set(dnsStatusAtom, status);
  } catch (err) {
    if (cancelled) {
      // 用户主动 cancel —— issue #149：
      //   1) 不弹错误 toast（cancel 是用户的意图,不是失败）
      //   2) 不 throw —— 调用方(Settings)不需要走错误分支
      //   3) 后端可能还在跑 rollback（proxy self-cleanup），所以**不**
      //      主动写 dnsEnabledAtom —— 等下一次 fetchDnsModeAtom 从
      //      后端拉真值。这里兜底再 fetch 一次,如果 cancel 已经把
      //      后端清成之前的状态,UI 立刻拨正。
      set(dnsErrorAtom, null);
      try {
        const truth = await getDnsMode();
        set(dnsEnabledAtom, truth);
        const status = await getDnsStatus();
        set(dnsStatusAtom, status);
      } catch {
        // 后端 truth fetch 失败,保留旧 UI 状态,等下次 fetch。
      }
      return;
    }
    set(dnsErrorAtom, extractErrorMessage(err));
    set(dnsStatusAtom, null);
    throw err;
  } finally {
    if (activeDnsToggleController === ctrl) {
      activeDnsToggleController = null;
    }
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
