import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { Provider as JotaiProvider } from "jotai";
import { getDefaultStore } from "jotai";
import { MemoryRouter, useLocation } from "react-router-dom";
import type { Profile } from "../../types";
import {
  profilesAtom,
  isApplyingAtom,
  dnsEnabledAtom,
  dnsProfilesAtom,
  enabledDnsProfilesAtom,
  dnsRuleCountAtom,
  systemDnsAtom,
} from "../../stores/profiles";

// Mock tauri invoke
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import StatusBar from "../../components/StatusBar";

function makeProfile(overrides: Partial<Profile> = {}): Profile {
  return {
    id: "p1",
    name: "dev-profile",
    description: null,
    enabled: true,
    protected: false,
    tags: [],
    rules: [],
    mode: "hosts",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

function makeDnsProfile(overrides: Partial<Profile> = {}): Profile {
  return {
    id: "d1",
    name: "dns-profile",
    description: null,
    enabled: true,
    protected: false,
    tags: [],
    rules: [],
    mode: "dns",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

function Wrapper({ children }: { children: React.ReactNode }) {
  return (
    <MemoryRouter>
      <JotaiProvider store={getDefaultStore()}>{children}</JotaiProvider>
    </MemoryRouter>
  );
}

/** issue #232: 让断言能看见路由落点。仓库里此前没有导航断言的先例，
 *  所以这里用最小探针而不是 mock `useNavigate` —— mock 掉之后
 *  `handleDnsClick` 里的分支就测不到了。 */
function LocationProbe() {
  const location = useLocation();
  return <div data-testid="location">{location.pathname}</div>;
}

function makeProbe(pointsAtLoopback: boolean) {
  return {
    interface: "Wi-Fi",
    servers: pointsAtLoopback ? ["127.0.0.1"] : ["192.168.1.1"],
    points_at_loopback: pointsAtLoopback,
  };
}

describe("StatusBar", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(profilesAtom, []);
    store.set(isApplyingAtom, false);
    store.set(dnsEnabledAtom, false);
    store.set(dnsProfilesAtom, []);
    // issue #232: 必须显式清空 —— 否则上一个用例留下的探测快照会漏进
    // 下一个用例，让 `dnsDiscrepancyAtom` 凭空变成 stuck，白底失败或
    // 更糟：假绿。
    store.set(systemDnsAtom, null);
  });

  // ---- Hosts column (v0.3 behavior) ----

  it("shows active profile name", () => {
    const profile = makeProfile();
    const store = getDefaultStore();
    store.set(profilesAtom, [profile]);

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    expect(screen.getByText("dev-profile")).toBeInTheDocument();
  });

  it("shows 'None' when no profile enabled", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, []);

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    expect(screen.getByText("None")).toBeInTheDocument();
  });

  it("shows 'Applying...' when isApplying is true", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile()]);
    store.set(isApplyingAtom, true);

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    expect(screen.getByText("Applying...")).toBeInTheDocument();
  });

  it("does not show 'Applying...' when isApplying is false", () => {
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile()]);
    store.set(isApplyingAtom, false);

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    expect(screen.queryByText("Applying...")).not.toBeInTheDocument();
  });

  // ---- DNS column (issue #67) ----

  it("shows DNS 'Off' when dnsEnabled is false", () => {
    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    expect(screen.getByText("Off")).toBeInTheDocument();
  });

  it("shows DNS summary 'enabled/total rules' when dnsEnabled is true", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, true);
    store.set(dnsProfilesAtom, [
      makeDnsProfile({ id: "d1", name: "ads", enabled: true }),
      makeDnsProfile({ id: "d2", name: "trackers", enabled: true }),
      makeDnsProfile({ id: "d3", name: "dev", enabled: false }),
    ]);

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    // 2 enabled / 3 total · 0 rules (no real rules in this test fixture)
    expect(screen.getByText("2/3 enabled · 0 rules")).toBeInTheDocument();
  });

  it("DNS column shows singular 'rule' when count is 1", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, true);
    const only = makeDnsProfile({
      id: "d1",
      enabled: true,
      rules: [
        {
          id: "r1",
          ip: "127.0.0.1",
          domains: ["x"],
          enabled: true,
          comment: null,
          source: { type: "Manual" },
        },
      ],
    });
    store.set(dnsProfilesAtom, [only]);

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    expect(screen.getByText("1/1 enabled · 1 rule")).toBeInTheDocument();
  });

  it("DNS summary reflects sum of real rules across enabled profiles", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, true);
    store.set(dnsProfilesAtom, [
      makeDnsProfile({
        id: "d1",
        enabled: true,
        rules: [
          {
            id: "r1",
            ip: "127.0.0.1",
            domains: ["a"],
            enabled: true,
            comment: null,
            source: { type: "Manual" },
          },
          {
            id: "r2",
            ip: "127.0.0.1",
            domains: ["b"],
            enabled: true,
            comment: null,
            source: { type: "Manual" },
          },
        ],
      }),
      makeDnsProfile({
        id: "d2",
        enabled: true,
        rules: [
          {
            id: "r3",
            ip: "0.0.0.0",
            domains: ["c"],
            enabled: true,
            comment: null,
            source: { type: "Manual" },
          },
        ],
      }),
      makeDnsProfile({
        id: "d3",
        enabled: false,
        rules: [
          {
            id: "r4",
            ip: "0.0.0.0",
            domains: ["d"],
            enabled: true,
            comment: null,
            source: { type: "Manual" },
          },
        ],
      }),
    ]);

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    // 2 enabled, 3 total · 3 active rules (r1+r2 from d1 + r3 from d2; d3 disabled → r4 not counted)
    expect(screen.getByText("2/3 enabled · 3 rules")).toBeInTheDocument();
  });

  it("uses 'enabledDnsProfilesAtom' / 'dnsRuleCountAtom' derived atoms", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, true);
    store.set(dnsProfilesAtom, [
      makeDnsProfile({ id: "d1", enabled: true }),
    ]);

    // Derived atoms should reflect the input
    expect(store.get(enabledDnsProfilesAtom)).toHaveLength(1);
    expect(store.get(dnsRuleCountAtom)).toBe(0);

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    expect(screen.getByText("1/1 enabled · 0 rules")).toBeInTheDocument();
  });
});

/**
 * Issue #232 —— `stuck_at_loopback` 的常驻警示入口。
 *
 * 背景：这个状态意味着用户的系统 DNS 仍指向 127.0.0.1，域名解析已经坏了。
 * 原来的警示只在 Settings 页（`Settings.test.tsx` 的
 * `dns-discrepancy-banner`），而用户在 mHost 里乱点时不会想到去 Settings。
 * 侧栏底栏在所有路由下常驻，所以警示搬到这里。
 *
 * 注意本组用例**不复用** Settings 的断言路径：侧栏只负责「让用户注意到」，
 * 解释与一键 Restore 仍然只在 Settings（issue #232「不在范围内」）。
 */
describe("StatusBar — stuck_at_loopback warning (issue #232)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(profilesAtom, []);
    store.set(isApplyingAtom, false);
    // 每个用例都显式 set 了 dnsEnabledAtom，但仍在 beforeEach 复位：
    // `dnsDiscrepancyAtom` 的判定同时依赖它和 systemDnsAtom，谁漏写一个
    // 就会让「警示没出现」变成一条**永远通过**的空断言。
    store.set(dnsEnabledAtom, false);
    store.set(dnsProfilesAtom, []);
    store.set(systemDnsAtom, null);
  });

  it("shows the 'DNS broken' warning when DNS is off but system points at loopback", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, false);
    store.set(systemDnsAtom, makeProbe(true));

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    const warning = screen.getByTestId("dns-stuck-warning");
    expect(warning).toHaveTextContent("DNS broken");
    // 不能只显示 "Off"：那正是这个 bug 的伪装形态，用户看不出异常。
    expect(screen.queryByText("Off")).not.toBeInTheDocument();
    expect(screen.getByTestId("dns-status-card")).toHaveAttribute(
      "data-dns-stuck",
      "true",
    );
  });

  it("replaces the 'Off' summary with a tooltip that names the actual failure", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, false);
    store.set(systemDnsAtom, makeProbe(true));

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    const title = screen.getByTestId("dns-status-card").getAttribute("title");
    expect(title).toMatch(/127\.0\.0\.1/);
    expect(title).toMatch(/resolution is likely broken/i);
    // 警示必须指向修复路径，否则只是制造焦虑。
    expect(title).toMatch(/Settings/i);
  });

  it("does not warn when the probe is unavailable (no data source = no alarm)", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, false);
    store.set(systemDnsAtom, null);

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    expect(screen.queryByTestId("dns-stuck-warning")).not.toBeInTheDocument();
    expect(screen.getByText("Off")).toBeInTheDocument();
  });

  it("does not warn for not_pointing (DNS still works — out of scope per #232)", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, true);
    store.set(systemDnsAtom, makeProbe(false));

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    expect(screen.queryByTestId("dns-stuck-warning")).not.toBeInTheDocument();
    // not_pointing 时侧栏维持既有的 DNS 摘要，不做任何升级。
    expect(screen.getByText("0/0 enabled · 0 rules")).toBeInTheDocument();
  });

  it("does not warn when mHost is off and the system is not at loopback", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, false);
    store.set(systemDnsAtom, makeProbe(false));

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );

    expect(screen.queryByTestId("dns-stuck-warning")).not.toBeInTheDocument();
    expect(screen.getByText("Off")).toBeInTheDocument();
  });

  it("clicking the warning lands on /settings, where the Restore button lives", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, false);
    store.set(systemDnsAtom, makeProbe(true));

    render(
      <Wrapper>
        <StatusBar />
        <LocationProbe />
      </Wrapper>,
    );

    expect(screen.getByTestId("location")).toHaveTextContent("/");
    fireEvent.click(screen.getByTestId("dns-status-card"));
    expect(screen.getByTestId("location")).toHaveTextContent("/settings");
  });

  it("Enter key on the warning also lands on /settings", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, false);
    store.set(systemDnsAtom, makeProbe(true));

    render(
      <Wrapper>
        <StatusBar />
        <LocationProbe />
      </Wrapper>,
    );

    fireEvent.keyDown(screen.getByTestId("dns-status-card"), { key: "Enter" });
    expect(screen.getByTestId("location")).toHaveTextContent("/settings");
  });

  it("clears the warning once a later probe reports a consistent state", () => {
    const store = getDefaultStore();
    store.set(dnsEnabledAtom, false);
    store.set(systemDnsAtom, makeProbe(true));

    render(
      <Wrapper>
        <StatusBar />
      </Wrapper>,
    );
    expect(screen.getByTestId("dns-stuck-warning")).toBeInTheDocument();

    // 用户点了 Settings 的 Restore（或自己在系统设置里改回）后，
    // focus 触发的 re-probe 落地 —— 警示必须随之消失。
    // 必须包在 act() 里：atom 写发生在 render 之后，裸 store.set 不会
    // 同步刷新订阅者，断言会读到上一次渲染的旧 DOM。
    act(() => {
      store.set(systemDnsAtom, makeProbe(false));
    });

    expect(screen.queryByTestId("dns-stuck-warning")).not.toBeInTheDocument();
    expect(screen.getByText("Off")).toBeInTheDocument();
  });
});
