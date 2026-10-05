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

import type { SystemDnsSnapshot } from "../../types";
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
 * Issue #232（AI review 发现）：toggle **收尾**窗口的假 `stuck_at_loopback`。
 *
 * `toggleDnsModeAtom` 开头那次置 null 只护住「toggle 开始前」的快照。等
 * sudo 授权期间用户切走再切回，窗口 focus 会发一次探测，在 disable 真正
 * 生效前落地，带回 `points_at_loopback=true` —— 那一刻它是真的，但对即将
 * 结束的 toggle 已经过时。
 *
 * 于是 `set(dnsEnabledAtom, false)` 一落地就派生出 `stuck_at_loopback`：
 * 后端刚成功还原了系统 DNS，UI 却报「DNS broken」并指向 Restore 按钮。
 * 假警报要等收尾那次 `void runProbe()` 回来才消失。
 *
 * 修法：翻内存态之前再清一次快照（宣布「不知道」= 不报警）。
 *
 * 注意**不能**用 `isDnsLoadingAtom` 门控 —— `finally` 里的
 * `set(isDnsLoadingAtom, false)` 在 probe **发起**时就跑，不是落地时，
 * 盖不住探测延迟（configd 卡住时能挂几秒）。
 */
describe("toggleDnsModeAtom drops a mid-toggle probe (issue #232)", () => {
  const store = getDefaultStore();

  beforeEach(() => {
    vi.clearAllMocks();
    store.set(dnsEnabledAtom, true);
    store.set(dnsStatusAtom, null);
    store.set(dnsErrorAtom, null);
    store.set(isDnsLoadingAtom, false);
    // 收尾那次探测永远 pending：只断言「过时快照被作废」，不受新值干扰。
    (probeSystemDns as unknown as { mockReturnValue: (v: unknown) => void })
      .mockReturnValue(new Promise(() => {}));
    (getDnsStatus as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue(null);
    // 本 describe 必须自持。cancel 用例会触发 abort 监听器里的
    // `cancelDnsMode().catch(...)`；`vi.clearAllMocks()` 只清调用记录、
    // **不清实现**，所以此前是靠上面 #149 describe 的
    // `mockResolvedValue(undefined)` 漏过来才通过的。一旦那个 describe
    // 被删改或执行顺序变化，这里就会拿到 undefined，`.catch` 抛
    // TypeError。显式建一次，不依赖邻居。
    (cancelDnsMode as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue(undefined);
  });

  it("a focus probe landing mid-toggle does not resurrect a false stuck_at_loopback", async () => {
    // toggle 前的合法快照：DNS 开着 + 系统指向 127.0.0.1 → 一致。
    store.set(systemDnsAtom, {
      interface: "Wi-Fi",
      servers: ["127.0.0.1"],
      points_at_loopback: true,
    });
    expect(store.get(dnsDiscrepancyAtom)).toBeNull();

    // setDnsMode 挂起，模拟 sudo 授权还没完成。
    let resolveSetDnsMode!: (v: unknown) => void;
    (setDnsMode as unknown as { mockReturnValue: (v: unknown) => void })
      .mockReturnValue(new Promise((r) => {
        resolveSetDnsMode = r;
      }));

    const toggle = store.set(toggleDnsModeAtom, false);

    // toggle 还在飞：窗口 focus 的探测落地。它带回的
    // points_at_loopback=true 在此刻**是真的**（disable 还没生效），
    // 但对这个即将结束的 toggle 已经过时。
    store.set(systemDnsAtom, {
      interface: "Wi-Fi",
      servers: ["127.0.0.1"],
      points_at_loopback: true,
    });

    // 后端 disable 成功返回。
    resolveSetDnsMode(undefined);
    await toggle;

    expect(store.get(dnsEnabledAtom)).toBe(false);
    // 契约：内存态翻成 false 的同一刻，过时快照必须已被作废。
    expect(store.get(systemDnsAtom)).toBeNull();
    expect(store.get(dnsDiscrepancyAtom)).toBeNull();
  });

  /**
   * 上一条只覆盖了「置 null」。这一条覆盖**代数递增**那半步：光置 null
   * 的话，一个在置空之前发起、在 `await getDnsStatus()` 窗口里返回的
   * 探测，代数仍然匹配，`applyProbe` 会把已知过时的快照写回去 ——
   * 假警报被压缩而没有关死。
   *
   * 这里刻意走 `probeSystemDnsAtom` 真实发起一次探测（而不是直接
   * `store.set(systemDnsAtom, ...)`），因为代数守卫只有真跑一遍
   * `runProbe` / `applyProbe` 才会被验证到。
   */
  it("drops an in-flight probe that lands in the post-clear await window", async () => {
    store.set(dnsEnabledAtom, true);
    store.set(systemDnsAtom, {
      interface: "Wi-Fi",
      servers: ["127.0.0.1"],
      points_at_loopback: true,
    });

    // 控制两个异步点：getDnsStatus（toggle 成功路径上那个 yield）
    // 与在途探测的返回值。
    let resolveStatus!: (v: unknown) => void;
    (getDnsStatus as unknown as { mockReturnValue: (v: unknown) => void })
      .mockReturnValue(new Promise((r) => {
        resolveStatus = r;
      }));
    let resolveInFlight!: (v: unknown) => void;
    (probeSystemDns as unknown as { mockImplementationOnce: (fn: unknown) => void })
      .mockImplementationOnce(
        () => new Promise((r) => {
          resolveInFlight = r;
        }),
      );
    let resolveSetDnsMode!: (v: unknown) => void;
    (setDnsMode as unknown as { mockReturnValue: (v: unknown) => void })
      .mockReturnValue(new Promise((r) => {
        resolveSetDnsMode = r;
      }));

    // 第一次 probeSystemDns 调用 = 模拟窗口 focus 发起的探测（在途）。
    const focusProbe = store.set(probeSystemDnsAtom);
    // 第二次 = toggle 收尾那次 re-probe，让它永远 pending，不干扰断言。
    (probeSystemDns as unknown as { mockReturnValue: (v: unknown) => void })
      .mockReturnValue(new Promise(() => {}));

    const toggle = store.set(toggleDnsModeAtom, false);

    // 后端 disable 成功 → toggle 走到 invalidate + 翻转内存态，
    // 然后停在 `await getDnsStatus()`。
    resolveSetDnsMode(undefined);
    await vi.waitFor(() =>
      expect(store.get(dnsEnabledAtom)).toBe(false),
    );

    // 此刻在途探测才返回，带回过时结论。代数若被递增过，它写不进来。
    resolveInFlight({
      interface: "Wi-Fi",
      servers: ["127.0.0.1"],
      points_at_loopback: true,
    });
    await focusProbe;

    expect(store.get(systemDnsAtom)).toBeNull();
    expect(store.get(dnsDiscrepancyAtom)).toBeNull();

    // 放行 toggle 收尾。
    resolveStatus(null);
    await toggle;
  });

  /**
   * cancel 路径的同形状假警报（issue #232 第二轮）。
   *
   * 窗口比成功路径**更大**：`await getDnsMode()` 和随后的
   * `await getDnsStatus()` 两次 IPC 都在翻转 `dnsEnabledAtom` 之前，而
   * rollback 期间系统 DNS 恰恰最可能停在 127.0.0.1（#152 同款）。
   * 若 atom 里还留着一份 `points_at_loopback=true` 的旧快照，
   * `truth=false` 一落地就派生出假的 `stuck_at_loopback`。
   */
  it("a cancelled toggle does not flip to a false stuck_at_loopback", async () => {
    store.set(dnsEnabledAtom, true);
    // 用户点 Disable 前的合法快照：DNS 开着 + 指向 127.0.0.1 → 一致。
    store.set(systemDnsAtom, {
      interface: "Wi-Fi",
      servers: ["127.0.0.1"],
      points_at_loopback: true,
    });
    // 收尾 re-probe 永远 pending，不干扰断言。
    (probeSystemDns as unknown as { mockReturnValue: (v: unknown) => void })
      .mockReturnValue(new Promise(() => {}));
    // backend truth：cancel 后系统 DNS 已回到「没开」
    (getDnsMode as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue(false);

    let rejectSet!: (err: unknown) => void;
    const setPromise = new Promise<void>((_, reject) => {
      rejectSet = reject;
    });
    setPromise.catch(() => {
      /* swallowed — 真正的 handler 是 atom 自己的 catch */
    });
    (setDnsMode as unknown as { mockImplementation: (fn: unknown) => void })
      .mockImplementation(() => setPromise);

    const togglePromise = store.set(toggleDnsModeAtom, false);
    await new Promise((r) => setTimeout(r, 0));

    // cancel → 后端 rollback 完成 → setDnsMode 以 Cancelled 结束。
    cancelActiveDnsToggle();

    // **关键**：toggle 开头的作废已经清过一次，所以必须在这里重新注入
    // 一份过时快照，cancel 分支的作废才真的被踩到 —— 否则
    // `systemDnsAtom` 从头到尾都是 null，撤掉修复也照样通过。
    //
    // 这模拟窗口 focus 的探测在 rollback 期间落地：那一刻系统 DNS 的确
    // 还停在 127.0.0.1，但对即将用 backend truth 拨正内存态的这条路径
    // 已经过时。
    store.set(systemDnsAtom, {
      interface: "Wi-Fi",
      servers: ["127.0.0.1"],
      points_at_loopback: true,
    });

    rejectSet({ Cancelled: null });
    await togglePromise;

    expect(store.get(dnsEnabledAtom)).toBe(false);
    // 契约：truth 落地的同一刻，过时快照必须已被作废 → 不报警。
    expect(store.get(systemDnsAtom)).toBeNull();
    expect(store.get(dnsDiscrepancyAtom)).toBeNull();
  });

  /**
   * toggle **开头**那一次作废的代数递增（issue #232 第二轮）。
   *
   * 在 toggle 开始之前就在飞的探测，描述的是「toggle 之前的世界」。
   * 它必须在 toggle 期间回来时被丢掉，否则它会把一个已经作废的结论
   * 重新写进 atom —— 正是 issue #232 那条评论指出的「清快照 ≠ 作废在途
   * 探测」。
   */
  it("drops a probe that was already in flight when the toggle started", async () => {
    store.set(dnsEnabledAtom, true);
    store.set(systemDnsAtom, {
      interface: "Wi-Fi",
      servers: ["127.0.0.1"],
      points_at_loopback: true,
    });

    // 在 toggle 之前发起一次探测，让它挂在半空。
    let resolvePreToggle!: (v: unknown) => void;
    (probeSystemDns as unknown as { mockImplementationOnce: (fn: unknown) => void })
      .mockImplementationOnce(
        () => new Promise((r) => {
          resolvePreToggle = r;
        }),
      );
    const preToggleProbe = store.set(probeSystemDnsAtom);
    // toggle 收尾的 re-probe 永远 pending。
    (probeSystemDns as unknown as { mockReturnValue: (v: unknown) => void })
      .mockReturnValue(new Promise(() => {}));

    let resolveSetDnsMode!: (v: unknown) => void;
    (setDnsMode as unknown as { mockReturnValue: (v: unknown) => void })
      .mockReturnValue(new Promise((r) => {
        resolveSetDnsMode = r;
      }));

    const toggle = store.set(toggleDnsModeAtom, false);

    // toggle 开始后，那条在途探测才返回 —— 代数若已递增，它写不进来。
    resolvePreToggle({
      interface: "Wi-Fi",
      servers: ["127.0.0.1"],
      points_at_loopback: true,
    });
    await preToggleProbe;

    expect(store.get(systemDnsAtom)).toBeNull();

    resolveSetDnsMode(undefined);
    await toggle;
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

/**
 * 回归测试（issue #153，review #231 跟进）：晚到的旧代探测必须被丢弃。
 *
 * 竞态形态：启动时的探测 P1 因 wedged configd 卡住；期间用户完成一次
 * toggle，P2 已经拿到**新**快照并写入 atom；P1 随后返回，若无守卫就会用
 * **toggle 之前**的旧快照覆盖 P2，横幅短暂指向错误方向。
 *
 * 用可控 deferred promise 精确复现这个时序：让 P1 一直 pending，先跑
 * P2 落地，再让 P1 返回。
 */
describe("late-arriving probe is discarded (issue #153)", () => {
  const store = getDefaultStore();

  /** 一个由测试手动 resolve 的探测 promise。 */
  function deferred() {
    let resolve!: (v: SystemDnsSnapshot) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<SystemDnsSnapshot>((res, rej) => {
      resolve = res;
      reject = rej;
    });
    return { promise, resolve, reject };
  }

  beforeEach(() => {
    vi.clearAllMocks();
    store.set(dnsEnabledAtom, false);
    store.set(systemDnsAtom, null);
    store.set(dnsErrorAtom, null);
    store.set(isDnsLoadingAtom, false);
    (getDnsMode as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue(false);
    (getDnsStatus as unknown as { mockResolvedValue: (v: unknown) => void })
      .mockResolvedValue(null);
  });

  it("a slow older probe must not overwrite a newer probe's snapshot", async () => {
    const P1 = deferred(); // 启动探测：卡住（wedged configd）
    const P2 = deferred(); // toggle 后的 re-probe

    // 返回类型也要是 mock 本身，否则链式第二次调用过不了 tsc。
    const asProbeMock = probeSystemDns as unknown as {
      mockReturnValueOnce: (v: unknown) => { mockReturnValueOnce: (v: unknown) => void };
    };
    asProbeMock.mockReturnValueOnce(P1.promise).mockReturnValueOnce(P2.promise);

    // P1 起飞（不 await —— 它还没返回）。
    const p1Task = store.set(probeSystemDnsAtom);
    // P2 起飞并先落地：系统 DNS 现在指向 mHost。
    const p2Task = store.set(probeSystemDnsAtom);
    P2.resolve({
      interface: "Wi-Fi",
      servers: ["127.0.0.1"],
      points_at_loopback: true,
    });
    await p2Task;
    expect(store.get(systemDnsAtom)).toEqual({
      interface: "Wi-Fi",
      servers: ["127.0.0.1"],
      points_at_loopback: true,
    });

    // P1 现在才返回，带的是 toggle **之前**的状态：用户把 DNS 关了，
    // 系统回到公网 DNS。
    P1.resolve({
      interface: "Wi-Fi",
      servers: ["8.8.8.8"],
      points_at_loopback: false,
    });
    await p1Task;

    // 代数守卫必须让 P1 的结果作废 —— 否则这里会变成 8.8.8.8，
    // 横幅也会短暂指向 not_pointing 这个错误方向。
    expect(store.get(systemDnsAtom)).toEqual({
      interface: "Wi-Fi",
      servers: ["127.0.0.1"],
      points_at_loopback: true,
    });
  });

  it("a late *failure* must not wipe a newer probe's snapshot either", async () => {
    const P1 = deferred();
    const P2 = deferred();
    // 返回类型也要是 mock 本身，否则链式第二次调用过不了 tsc。
    const asProbeMock = probeSystemDns as unknown as {
      mockReturnValueOnce: (v: unknown) => { mockReturnValueOnce: (v: unknown) => void };
    };
    asProbeMock.mockReturnValueOnce(P1.promise).mockReturnValueOnce(P2.promise);

    const p1Task = store.set(probeSystemDnsAtom);
    const p2Task = store.set(probeSystemDnsAtom);
    P2.resolve({
      interface: "Wi-Fi",
      servers: ["127.0.0.1"],
      points_at_loopback: true,
    });
    await p2Task;

    // P1 失败（比如它启动时用户还没联网，probe 耗了几秒才返回错误）。
    // 无守卫的话这里走 catch 分支把 P2 的快照擦成 null，横幅无缘无故消失。
    P1.reject(new Error("route: no default route"));
    await p1Task;

    expect(store.get(systemDnsAtom)).toEqual({
      interface: "Wi-Fi",
      servers: ["127.0.0.1"],
      points_at_loopback: true,
    });
    expect(store.get(dnsErrorAtom)).toBeNull();
  });
});
