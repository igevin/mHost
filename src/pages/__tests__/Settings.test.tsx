import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { BrowserRouter } from "react-router-dom";
import { getDefaultStore, Provider as JotaiProvider } from "jotai";
import {
  dnsEnabledAtom,
  dnsStatusAtom,
  isDnsLoadingAtom,
  quickApplyOnToggleAtom,
  systemDnsAtom,
} from "../../stores/profiles";
import type { DnsStatus } from "../../types";
import { POINTER_DOWN_DEBOUNCE_MS } from "../../hooks/useWebKitPointerDown";

// Define global __APP_VERSION__ for tests
(globalThis as unknown as Record<string, string>).__APP_VERSION__ = "0.2.0";

const mockSetDnsMode = vi.fn().mockResolvedValue(undefined);
const mockGetDnsStatus = vi.fn().mockResolvedValue({
  running: true,
  port: 53,
  upstream: ["8.8.8.8"],
  original_dns: { kind: "manual", servers: ["192.168.31.1"] },
  rule_count: 10,
  cache_capacity: 100,
});

const mockProbeSystemDns = vi.fn().mockResolvedValue({
  interface: "Wi-Fi",
  servers: ["192.168.31.1"],
  points_at_loopback: false,
});

vi.mock("../../lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/tauri")>();
  return {
    ...actual,
    getDnsMode: vi.fn().mockResolvedValue(false),
    getDnsStatus: (...args: unknown[]) => mockGetDnsStatus(...args),
    setDnsMode: (...args: unknown[]) => mockSetDnsMode(...args),
    reloadDnsRules: vi.fn().mockResolvedValue(undefined),
    probeSystemDns: (...args: unknown[]) => mockProbeSystemDns(...args),
  };
});

import Settings from "../Settings";

function renderWithProviders(ui: React.ReactElement) {
  return render(
    <JotaiProvider store={getDefaultStore()}>
      <BrowserRouter>{ui}</BrowserRouter>
    </JotaiProvider>,
  );
}

describe("Settings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, false);
    store.set(dnsStatusAtom, null);
    store.set(isDnsLoadingAtom, false);
    // issue #153: 默认「探测不可用」→ 不显示不一致横幅。相关用例各自覆写。
    store.set(systemDnsAtom, null);
    mockProbeSystemDns.mockResolvedValue({
      interface: "Wi-Fi",
      servers: ["192.168.31.1"],
      points_at_loopback: false,
    });
  });

  it("renders Settings page title", () => {
    renderWithProviders(<Settings />);
    expect(screen.getByText("Settings")).toBeInTheDocument();
  });

  it("renders DNS Mode card with Stopped status when disabled", () => {
    renderWithProviders(<Settings />);
    expect(screen.getByText("DNS Mode")).toBeInTheDocument();
    expect(screen.getByText("Stopped")).toBeInTheDocument();
  });

  it("renders DNS Mode card with Running status when enabled", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, true);
    store.set(dnsStatusAtom, {
      running: true,
      port: 53,
      upstream: ["8.8.8.8"],
      original_dns: { kind: "manual", servers: ["192.168.31.1"] },
      rule_count: 10,
      cache_capacity: 100,
    });

    renderWithProviders(<Settings />);
    expect(screen.getByText("Running")).toBeInTheDocument();
  });

  it("renders manual original DNS snapshot by joining servers list", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, true);
    store.set(dnsStatusAtom, {
      running: true,
      port: 53,
      upstream: ["8.8.8.8"],
      original_dns: { kind: "manual", servers: ["192.168.31.1"] },
      rule_count: 10,
      cache_capacity: 100,
    });

    renderWithProviders(<Settings />);
    expect(screen.getByText(/Original DNS/)).toBeInTheDocument();
    expect(screen.getByText(/192\.168\.31\.1/)).toBeInTheDocument();
  });

  it("renders DhcpEmpty original DNS snapshot with 'DHCP default' label", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, true);
    store.set(dnsStatusAtom, {
      running: true,
      port: 53,
      upstream: ["8.8.8.8"],
      original_dns: { kind: "dhcp_empty" },
      rule_count: 10,
      cache_capacity: 100,
    });

    renderWithProviders(<Settings />);
    expect(screen.getByText(/Original DNS/)).toBeInTheDocument();
    expect(screen.getByText(/DHCP default/)).toBeInTheDocument();
  });

  it("clicks Enable DNS Mode button and triggers toggle", async () => {
    renderWithProviders(<Settings />);

    const enableButton = screen.getByText("Enable DNS Mode");
    expect(enableButton).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(enableButton);
    });

    // issue #149: setDnsMode now accepts an optional `{ signal }` for the
    // AbortController wired by toggleDnsModeAtom. The toggle intent
    // (true / false) is the first positional arg.
    expect(mockSetDnsMode).toHaveBeenCalledWith(
      true,
      expect.objectContaining({ signal: expect.anything() }),
    );
  });

  it("clicks Disable DNS Mode button and triggers toggle", async () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, true);
    store.set(dnsStatusAtom, {
      running: true,
      port: 53,
      upstream: ["8.8.8.8"],
      original_dns: { kind: "manual", servers: ["192.168.31.1"] },
      rule_count: 10,
      cache_capacity: 100,
    });

    renderWithProviders(<Settings />);

    const disableButton = screen.getByText("Disable DNS Mode");
    expect(disableButton).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(disableButton);
    });

    expect(mockSetDnsMode).toHaveBeenCalledWith(
      false,
      expect.objectContaining({ signal: expect.anything() }),
    );
  });

  // ---- issue #123: Quick Apply toggle on Settings page ----

  it("renders the Quick Apply card with the toggle defaults to off", () => {
    renderWithProviders(<Settings />);
    const toggle = screen.getByTestId("quick-apply-toggle") as HTMLLabelElement;
    const input = toggle.querySelector("input") as HTMLInputElement;
    expect(input).toBeInTheDocument();
    expect(input.checked).toBe(false);
  });

  it("clicking the Quick Apply toggle flips the persisted atom + checkbox", async () => {
    const store = getDefaultStore();
    store.set(quickApplyOnToggleAtom, false);
    renderWithProviders(<Settings />);
    const toggle = screen.getByTestId("quick-apply-toggle") as HTMLLabelElement;
    const input = toggle.querySelector("input") as HTMLInputElement;

    await act(async () => {
      fireEvent.click(input);
    });

    // Clicking the input directly toggles the native checkbox.
    expect(input.checked).toBe(true);
    // The atom has also been updated through useSetAtom.
    expect(store.get(quickApplyOnToggleAtom)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Issue #153 — 系统 DNS 不一致横幅
// ---------------------------------------------------------------------------

/**
 * 横幅只在「mHost 内存态」与「系统 DNS 实际状态」**分歧**时出现。
 *
 * 两种方向的用户含义截然不同，因此断言也不同：
 * - `stuck_at_loopback` = #152 那个 bug 的样子（DNS 已经坏了）→ 给一键恢复
 * - `not_pointing`       = 只是 mHost 规则没生效（DNS 还能用）→ 只提示
 *
 * 一致态和「探测不可用」都必须**不**显示横幅 —— 后者尤其重要：
 * 非 macOS / 断网时探测会失败，用户不该看到一条自己无法处理的假警报。
 */
describe("Settings — system DNS discrepancy banner (issue #153)", () => {
  const banner = () => screen.queryByTestId("dns-discrepancy-banner");
  const restoreButton = () => screen.queryByTestId("dns-restore-button");

  function setProbe(
    pointsAtLoopback: boolean,
    servers: string[] = pointsAtLoopback ? ["127.0.0.1"] : ["8.8.8.8", "1.1.1.1"],
  ) {
    getDefaultStore().set(systemDnsAtom, {
      interface: "Wi-Fi",
      servers,
      points_at_loopback: pointsAtLoopback,
    });
  }

  const status: DnsStatus = {
    running: true,
    port: 1053,
    upstream: ["8.8.8.8"],
    // `as const` on the discriminator — a widened `{ kind: string }` is not
    // assignable to the OriginalDns union.
    original_dns: { kind: "manual", servers: ["192.168.31.1"] },
    rule_count: 4,
    cache_capacity: 100,
  };

  // ---- consistent: no banner ----

  it("hides the banner when DNS mode is on and system DNS points at mHost", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, true);
    store.set(dnsStatusAtom, status);
    setProbe(true);

    renderWithProviders(<Settings />);
    expect(banner()).not.toBeInTheDocument();
  });

  it("hides the banner when DNS mode is off and system DNS is elsewhere", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, false);
    setProbe(false);

    renderWithProviders(<Settings />);
    expect(banner()).not.toBeInTheDocument();
  });

  it("hides the banner when the probe is unavailable", () => {
    // systemDnsAtom 保持 null（探测失败 / 非 macOS / 断网）
    getDefaultStore().set(dnsEnabledAtom, false);

    renderWithProviders(<Settings />);
    expect(banner()).not.toBeInTheDocument();
  });

  // ---- stuck_at_loopback: the dangerous direction ----

  it("shows a recovery banner when DNS mode is off but system DNS is 127.0.0.1", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, false);
    setProbe(true, ["127.0.0.1"]);

    renderWithProviders(<Settings />);

    const el = banner();
    expect(el).toBeInTheDocument();
    expect(el).toHaveAttribute("data-discrepancy", "stuck_at_loopback");
    // 标题刻意不写死 "127.0.0.1"：IPv6-only 环境下 body 会显示 "::1"，
    // 写死的标题会和正文自相矛盾。精确值由正文的 servers 承担。
    expect(
      screen.getByText("System DNS still points at mHost"),
    ).toBeInTheDocument();
    expect(screen.getByText("127.0.0.1")).toBeInTheDocument();
    expect(screen.getByText(/Domain resolution may be/)).toBeInTheDocument();
    // 危险方向必须有恢复按钮。
    expect(restoreButton()).toBeInTheDocument();
  });

  it("Restore system DNS triggers toggleDnsMode(false)", async () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, false);
    setProbe(true, ["127.0.0.1"]);

    renderWithProviders(<Settings />);

    await act(async () => {
      fireEvent.click(restoreButton()!);
    });

    // 内存态已经是 false 也要真的调 set_dns_mode(false) —— 后端的
    // set_dns_mode_disable 没有「已禁用就短路」的分支，会执行真正的
    // 系统 DNS 还原。这正是 #153 最危险方向需要的能力。
    expect(mockSetDnsMode).toHaveBeenCalledWith(
      false,
      expect.objectContaining({ signal: expect.anything() }),
    );
  });

  /**
   * 回归测试：Restore 按钮**不能是一次性的**。
   *
   * `useWebKitPointerDown` 的 `firedRef` 只在 `releaseSoon()` 里复位。
   * 如果这个按钮自己调 `fire()` 却不 release，guard 会永久 latch ——
   * 用户点第一次恢复了系统 DNS（横幅消失）；等他下次遇到同样的问题、
   * 横幅在**同一个 Settings 挂载内**重新出现时，按钮已经死了。
   * 症状极具迷惑性：「按钮坏了」，而真正原因在另一个组件的 hook 里。
   *
   * 关键：两次点击必须发生在**同一个挂载**里。每次重新 render 都会拿到
   * 一个全新的 `useWebKitPointerDown`（`firedRef` 是 useRef），latch 就
   * 被掩盖了 —— 那样这个测试测不到任何东西。
   *
   * 正确写法是复用主开关的 `handleToggleDns`（fire + releaseSoon 齐全）。
   *
   * 用**增量**断言：全文件共用 `getDefaultStore()`，上一个测试的
   * `toggleDnsModeAtom` 续链可能跨过 `vi.clearAllMocks()` 才落地，
   * 绝对计数会假失败。
   */
  it("Restore stays usable after a previous use (fire/release latch)", async () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, false);
    setProbe(true, ["127.0.0.1"]);

    // 一个挂载，两次故障 —— 真实用户路径。
    renderWithProviders(<Settings />);

    await act(async () => {});
    const baseline = mockSetDnsMode.mock.calls.length;

    await act(async () => {
      fireEvent.click(screen.getByTestId("dns-restore-button"));
    });
    // 点击后 toggle 会用新探测覆盖快照 → 横幅消失。
    expect(screen.queryByTestId("dns-discrepancy-banner")).not.toBeInTheDocument();

    // 第二次故障：横幅在同一个挂载内重新出现。
    // store.set 必须包在 act 里，否则 React 不会刷这条订阅更新。
    await act(async () => {
      store.set(systemDnsAtom, {
        interface: "Wi-Fi",
        servers: ["127.0.0.1"],
        points_at_loopback: true,
      });
      // 让 `releaseSoon()` 的复位定时器走完 —— 真实用户两次点击间隔以秒计，
      // 而 `fire()` 的 guard 在这段时间内是故意 latch 的。不等就会把
      // 「正常的防抖」误判成「latch bug」。
      await new Promise((r) => setTimeout(r, POINTER_DOWN_DEBOUNCE_MS + 20));
    });
    expect(screen.getByTestId("dns-restore-button")).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(screen.getByTestId("dns-restore-button"));
    });

    // latch 存在时这里会是 1（只有第一次点了）。
    expect(mockSetDnsMode.mock.calls.length - baseline).toBe(2);
  });

  it("disables the Restore button while a DNS operation is in flight", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, false);
    store.set(isDnsLoadingAtom, true);
    setProbe(true, ["127.0.0.1"]);

    renderWithProviders(<Settings />);

    expect(restoreButton()).toBeDisabled();
    expect(restoreButton()).toHaveTextContent("Restoring…");
  });

  // ---- not_pointing: informational only ----

  it("shows an informational banner (no action button) when DNS mode is on but system DNS isn't mHost", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, true);
    store.set(dnsStatusAtom, status);
    setProbe(false, ["8.8.8.8", "1.1.1.1"]);

    renderWithProviders(<Settings />);

    const el = banner();
    expect(el).toBeInTheDocument();
    expect(el).toHaveAttribute("data-discrepancy", "not_pointing");
    expect(
      screen.getByText("System DNS does not point at mHost"),
    ).toBeInTheDocument();
    // 展示实际读到的 servers，让用户能自己判断是不是他改的。
    expect(screen.getByText(/8\.8\.8\.8, 1\.1\.1\.1/)).toBeInTheDocument();
    // 非危险方向**不**提供一键修复：修它要重启 enable 流程（两次 sudo），
    // 不值得为省一次手动开关在特权路径上加代码。
    expect(restoreButton()).not.toBeInTheDocument();
  });

  it("falls back to 'system default' wording when the probe returned no servers", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, true);
    store.set(dnsStatusAtom, status);
    // DHCP 默认：networksetup 返回 "There aren't any DNS Servers set"
    setProbe(false, []);

    renderWithProviders(<Settings />);

    expect(banner()).toBeInTheDocument();
    expect(screen.getByText(/the system default/)).toBeInTheDocument();
  });
});
