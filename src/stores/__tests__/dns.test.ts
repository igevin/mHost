import { describe, it, expect, vi, beforeEach } from "vitest";
import { getDefaultStore } from "jotai";

// vi.mock factory is hoisted — define the mock functions INSIDE the factory
// (no top-level references). The named imports below pick up the same
// vi.fn() instances through the mocked module.

vi.mock("../../lib/tauri", () => ({
  setDnsMode: vi.fn(),
  cancelDnsMode: vi.fn(),
  getDnsMode: vi.fn().mockResolvedValue(false),
  getDnsStatus: vi.fn().mockResolvedValue(null),
  // issue #153: toggleDnsModeAtom 的成功分支会 fire-and-forget 探测。
  // 这个 mock 是全量替换（无 importOriginal），漏掉它会让
  // `probeSystemDns` 变成 undefined 并抛 TypeError。
  probeSystemDns: vi.fn().mockResolvedValue(null),
}));

import {
  toggleDnsModeAtom,
  cancelActiveDnsToggle,
  fetchDnsModeAtom,
  probeSystemDnsAtom,
  dnsEnabledAtom,
  isDnsLoadingAtom,
  dnsErrorAtom,
  dnsStatusAtom,
  systemDnsAtom,
  dnsDiscrepancyAtom,
} from "../profiles";
import {
  setDnsMode,
  cancelDnsMode,
  getDnsMode,
  getDnsStatus,
  probeSystemDns,
} from "../../lib/tauri";

/**
 * Issue #149 — Settings page exposes a Cancel button while `set_dns_mode`
 * is awaiting an osascript sudo prompt. Clicking it aborts the IPC
 * promise and fires `cancel_dns_mode` so the backend rolls back. The
 * frontend must:
 *   1. NOT show the abort as an error toast
 *   2. NOT rethrow (callers should not need to handle AbortError)
 *   3. Clear `isDnsLoadingAtom` so the Cancel button hides itself
 *   4. Refetch backend truth so UI matches the rolled-back state
 *
 * The contract is exercised end-to-end here against mocked Tauri bindings.
 */
describe("toggleDnsModeAtom cancel path (issue #149)", () => {
  const store = getDefaultStore();

  beforeEach(() => {
    vi.clearAllMocks();
    store.set(dnsEnabledAtom, false);
    store.set(isDnsLoadingAtom, false);
    store.set(dnsErrorAtom, null);
    store.set(dnsStatusAtom, null);

    // Re-establish defaults after vi.clearAllMocks wipes them.
    (getDnsMode as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue(false);
    (getDnsStatus as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue(null);
    (cancelDnsMode as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue(undefined);
  });

  it("cancelActiveDnsToggle fires cancelDnsMode IPC and aborts the controller", async () => {
    // Simulate setDnsMode rejecting with the backend's Cancelled error
    // (this is what happens after the Rust rollback completes post-cancel).
    // Use a manually-controlled promise so the rejection doesn't surface
    // as a separate unhandled rejection — it must be observed via the
    // atom's try/catch.
    let rejectSet!: (err: unknown) => void;
    const setPromise = new Promise<void>((_, reject) => {
      rejectSet = reject;
    });
    // Attach a no-op catch on the inner promise so vitest's unhandled-
    // rejection tracker doesn't complain — the atom's own catch will
    // be the real handler.
    setPromise.catch(() => {
      /* swallowed — the atom's try/catch is the real handler */
    });
    (setDnsMode as unknown as { mockImplementation: (fn: unknown) => void })
      .mockImplementation(() => setPromise);

    // Kick off the toggle. We don't await — we want to abort mid-flight.
    const togglePromise = store.set(toggleDnsModeAtom, true);

    // Let microtask queue process so the controller is registered.
    await new Promise((r) => setTimeout(r, 0));

    // isDnsLoading should now be true.
    expect(store.get(isDnsLoadingAtom)).toBe(true);

    // Click Cancel.
    cancelActiveDnsToggle();

    // cancelDnsMode IPC should have fired (from the abort handler).
    expect(cancelDnsMode).toHaveBeenCalledTimes(1);

    // Now reject setDnsMode — the atom's await catches the rejection.
    rejectSet({ Cancelled: null });
    await togglePromise;

    // Post-conditions for the cancel path:
    //   - isDnsLoading back to false
    //   - no error toast (dnsError stays null)
    //   - getDnsMode + getDnsStatus fetched to refresh UI from backend truth
    //   - dnsEnabled reflects backend truth (mock returns false)
    expect(store.get(isDnsLoadingAtom)).toBe(false);
    expect(store.get(dnsErrorAtom)).toBeNull();
    expect(getDnsMode).toHaveBeenCalled();
    expect(getDnsStatus).toHaveBeenCalled();
    expect(store.get(dnsEnabledAtom)).toBe(false);
  });

  it("toggleDnsModeAtom does NOT throw when cancelled mid-flight", async () => {
    // setDnsMode that hangs forever.
    (setDnsMode as unknown as { mockImplementation: (fn: unknown) => void })
      .mockImplementation(
        () =>
          new Promise<void>(() => {
            /* never resolves */
          }),
      );

    const togglePromise = store.set(toggleDnsModeAtom, true);
    await new Promise((r) => setTimeout(r, 0));

    // Cancel mid-flight. After cancellation, setDnsMode will eventually
    // resolve/reject but the toggle should not throw because of cancel.
    cancelActiveDnsToggle();
    await new Promise((r) => setTimeout(r, 10));

    // The toggle should not have thrown.
    let rejected = false;
    togglePromise.catch(() => {
      rejected = true;
    });
    await new Promise((r) => setTimeout(r, 0));
    expect(rejected).toBe(false);
  });

  it("cancelActiveDnsToggle is a no-op when no toggle is in flight", () => {
    // No toggle running.
    expect(() => cancelActiveDnsToggle()).not.toThrow();
    expect(cancelDnsMode).not.toHaveBeenCalled();
  });

  it("real backend error: dnsErrorAtom is set and atom throws", async () => {
    // Simulate a real backend error (NOT cancellation).
    (setDnsMode as unknown as { mockRejectedValueOnce: (v: unknown) => void })
      .mockRejectedValueOnce(
        Object.assign(new Error("boom"), { kind: "InvalidInput" }),
      );

    await expect(store.set(toggleDnsModeAtom, true)).rejects.toThrow();

    expect(store.get(isDnsLoadingAtom)).toBe(false);
    // dnsError should be set to a non-null extracted message.
    expect(store.get(dnsErrorAtom)).not.toBeNull();
    // cancelDnsMode was NOT called because we didn't abort.
    expect(cancelDnsMode).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------
// Issue #153 — 系统 DNS 独立探测
// ---------------------------------------------------------------------------

/**
 * `dnsDiscrepancyAtom` 是 issue #153 的判定核心：它把「mHost 内存态」
 * 和「系统实际状态」两份数据源合成一个三态结果。
 *
 * 表格驱动覆盖 issue 原文那张真值表的全部四行，外加「探测不可用」
 * 这一行 —— 最后这行同样重要：`systemDnsAtom === null` 必须退回 null
 * （不报警），否则非 macOS / 断网的用户会看到一条无法处理的假警报。
 */
describe("dnsDiscrepancyAtom (issue #153)", () => {
  const store = getDefaultStore();

  beforeEach(() => {
    store.set(dnsEnabledAtom, false);
    store.set(systemDnsAtom, null);
  });

  interface Case {
    name: string;
    enabled: boolean;
    pointsAtLoopback: boolean;
    expected: string | null;
  }
  const cases: Case[] = [
    {
      name: "enabled + loopback = consistent",
      enabled: true,
      pointsAtLoopback: true,
      expected: null,
    },
    {
      name: "disabled + non-loopback = consistent",
      enabled: false,
      pointsAtLoopback: false,
      expected: null,
    },
    {
      name: "enabled + non-loopback = not_pointing",
      enabled: true,
      pointsAtLoopback: false,
      expected: "not_pointing",
    },
    {
      name: "disabled + loopback = stuck_at_loopback (the #152 bug)",
      enabled: false,
      pointsAtLoopback: true,
      expected: "stuck_at_loopback",
    },
  ];

  for (const c of cases) {
    it(`maps ${c.name} -> ${c.expected}`, () => {
      store.set(dnsEnabledAtom, c.enabled);
      store.set(systemDnsAtom, {
        interface: "Wi-Fi",
        servers: c.pointsAtLoopback ? ["127.0.0.1"] : ["8.8.8.8"],
        points_at_loopback: c.pointsAtLoopback,
      });
      expect(store.get(dnsDiscrepancyAtom)).toBe(c.expected);
    });
  }

  it("returns null when the probe is unavailable (no data source = no alarm)", () => {
    store.set(dnsEnabledAtom, false);
    store.set(systemDnsAtom, null);
    expect(store.get(dnsDiscrepancyAtom)).toBeNull();
  });
});

/**
 * `probeSystemDnsAtom` 的契约：**永不 reject，失败静默**。
 *
 * 静默的具体含义（三条都要守住）：
 * 1. reject 被吞掉 —— 调用方是 fire-and-forget 或并行 await
 * 2. `dnsErrorAtom` 不被写 —— 探测失败不是用户该看到的错误
 * 3. `isDnsLoadingAtom` 不被写 —— 探测不是用户发起的加载态
 *
 * 写 `dnsErrorAtom` 特别危险：Settings 页顶部有 `alert alert-error`
 * 区域，一个「route failed」会盖在页面上，而用户对此完全无能为力
 * （`route` 失败几乎总是因为没联网，用户自己知道）。
 */
describe("probeSystemDnsAtom (issue #153)", () => {
  const store = getDefaultStore();

  beforeEach(() => {
    vi.clearAllMocks();
    store.set(dnsEnabledAtom, false);
    store.set(systemDnsAtom, null);
    store.set(dnsErrorAtom, null);
    store.set(isDnsLoadingAtom, false);
    (probeSystemDns as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue(null);
  });

  it("stores the snapshot on success", async () => {
    (probeSystemDns as unknown as { mockResolvedValueOnce: (v: unknown) => void })
      .mockResolvedValueOnce({
        interface: "Wi-Fi",
        servers: ["127.0.0.1"],
        points_at_loopback: true,
      });

    await store.set(probeSystemDnsAtom);

    expect(store.get(systemDnsAtom)).toEqual({
      interface: "Wi-Fi",
      servers: ["127.0.0.1"],
      points_at_loopback: true,
    });
  });

  it("swallows rejection: clears snapshot, no error atom, no loading state", async () => {
    (probeSystemDns as unknown as { mockRejectedValueOnce: (v: unknown) => void })
      .mockRejectedValueOnce("unsupported platform: system DNS probe is only supported on macOS");

    // 关键：不抛。
    await expect(store.set(probeSystemDnsAtom)).resolves.toBeUndefined();

    expect(store.get(systemDnsAtom)).toBeNull();
    expect(store.get(dnsErrorAtom)).toBeNull();
    expect(store.get(isDnsLoadingAtom)).toBe(false);
    // 无从判断 → 不报警。
    expect(store.get(dnsDiscrepancyAtom)).toBeNull();
  });

  it("clears a stale snapshot when a later probe fails", async () => {
    // 先成功一次，让 UI 上已经挂着横幅。
    store.set(systemDnsAtom, {
      interface: "Wi-Fi",
      servers: ["127.0.0.1"],
      points_at_loopback: true,
    });
    expect(store.get(dnsDiscrepancyAtom)).toBe("stuck_at_loopback");

    // 再失败 —— 必须清掉，否则横幅会永久停在一个已经无法验证的结论上。
    (probeSystemDns as unknown as { mockRejectedValueOnce: (v: unknown) => void })
      .mockRejectedValueOnce(new Error("network gone"));

    await store.set(probeSystemDnsAtom);

    expect(store.get(systemDnsAtom)).toBeNull();
    expect(store.get(dnsDiscrepancyAtom)).toBeNull();
  });
});

/**
 * 回归测试：toggle 期间不得闪现假警报。
 *
 * 缺陷形态：disable 成功时 `dnsEnabledAtom` 先翻 false，而 `systemDnsAtom`
 * 还留着「DNS 开着时 points_at_loopback=true」的旧快照 —— 组合起来立刻
 * 推导出 `stuck_at_loopback`，横幅凭空闪现一个「系统 DNS 卡在 127.0.0.1 /
 * 点我恢复」的按钮。用户看到的是一次惊吓，而且他点的那个 Restore 按钮
 * 语义上完全错误（DNS 模式刚才是开着的，不存在卡住）。
 *
 * 契约：toggle 一开始就作废旧探测（置 null = 不知道 = 不报警），
 * 等新探测回来再决定横幅。
 */
describe("toggleDnsModeAtom clears stale probe (issue #153)", () => {
  const store = getDefaultStore();

  beforeEach(() => {
    vi.clearAllMocks();
    store.set(dnsEnabledAtom, true);
    store.set(dnsStatusAtom, null);
    store.set(dnsErrorAtom, null);
    store.set(isDnsLoadingAtom, false);
    (setDnsMode as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue(undefined);
    (getDnsStatus as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue(null);
    // 探测永远 pending：这样只断言「旧值被作废」，不受新值干扰。
    (probeSystemDns as unknown as { mockReturnValue: (v: unknown) => void })
      .mockReturnValue(new Promise(() => {}));
  });

  it("drops the stale snapshot at toggle start so no false banner flashes", async () => {
    // toggle 前：DNS 开着 + 系统指向 127.0.0.1 —— 完全一致，无横幅。
    store.set(systemDnsAtom, {
      interface: "Wi-Fi",
      servers: ["127.0.0.1"],
      points_at_loopback: true,
    });
    expect(store.get(dnsDiscrepancyAtom)).toBeNull();

    // toggle 之后、探测回来之前：内存态 false + 旧快照仍在
    // → 会推导出 stuck_at_loopback。契约要求此时是 null。
    await store.set(toggleDnsModeAtom, false);

    expect(store.get(systemDnsAtom)).toBeNull();
    expect(store.get(dnsDiscrepancyAtom)).toBeNull();
  });
});

/**
 * `fetchDnsModeAtom` 现在并行跑三件事：内存态 truth-fetch + status +
 * 系统 DNS 探测。探测失败**不能**污染主路径。
 */
describe("fetchDnsModeAtom with probe (issue #153)", () => {
  const store = getDefaultStore();

  beforeEach(() => {
    vi.clearAllMocks();
    store.set(dnsEnabledAtom, false);
    store.set(systemDnsAtom, null);
    store.set(dnsErrorAtom, null);
    store.set(dnsStatusAtom, null);
    (getDnsMode as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue(true);
    (getDnsStatus as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue({
        running: true,
        port: 1053,
        upstream: ["8.8.8.8"],
        original_dns: { kind: "dhcp_empty" },
        rule_count: 3,
        cache_capacity: 100,
      });
    (probeSystemDns as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue({
        interface: "Wi-Fi",
        servers: ["127.0.0.1"],
        points_at_loopback: true,
      });
  });

  it("populates all three atoms on the happy path", async () => {
    await store.set(fetchDnsModeAtom);

    expect(store.get(dnsEnabledAtom)).toBe(true);
    expect(store.get(systemDnsAtom)).not.toBeNull();
    expect(store.get(dnsDiscrepancyAtom)).toBeNull(); // 一致
    expect(store.get(dnsErrorAtom)).toBeNull();
  });

  it("a failing probe does not fail the truth-fetch", async () => {
    (probeSystemDns as unknown as { mockRejectedValueOnce: (v: unknown) => void })
      .mockRejectedValueOnce(new Error("networksetup exploded"));

    await store.set(fetchDnsModeAtom);

    // 主路径完整成功。
    expect(store.get(dnsEnabledAtom)).toBe(true);
    expect(store.get(dnsStatusAtom)).not.toBeNull();
    expect(store.get(dnsErrorAtom)).toBeNull();
    // 只有探测那部分降级。
    expect(store.get(systemDnsAtom)).toBeNull();
  });
});
