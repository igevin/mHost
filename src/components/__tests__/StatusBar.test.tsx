import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { Provider as JotaiProvider } from "jotai";
import { getDefaultStore } from "jotai";
import { MemoryRouter } from "react-router-dom";
import type { Profile } from "../../types";
import {
  profilesAtom,
  isApplyingAtom,
  dnsEnabledAtom,
  dnsProfilesAtom,
  enabledDnsProfilesAtom,
  dnsRuleCountAtom,
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

describe("StatusBar", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(profilesAtom, []);
    store.set(isApplyingAtom, false);
    store.set(dnsEnabledAtom, false);
    store.set(dnsProfilesAtom, []);
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
