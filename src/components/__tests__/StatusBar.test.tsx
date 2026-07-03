import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { Provider as JotaiProvider } from "jotai";
import { getDefaultStore } from "jotai";
import type { Profile } from "../../types";
import {
  profilesAtom,
  isApplyingAtom,
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

function Wrapper({ children }: { children: React.ReactNode }) {
  return <JotaiProvider store={getDefaultStore()}>{children}</JotaiProvider>;
}

describe("StatusBar", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(profilesAtom, []);
    store.set(isApplyingAtom, false);
  });

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
});
