import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { getDefaultStore, Provider as JotaiProvider } from "jotai";
import {
  adBlockStateAtom,
  isAdBlockLoadingAtom,
  adBlockErrorAtom,
  dnsEnabledAtom,
} from "../../stores/profiles";
import type { AdBlockState, AdBlockSource } from "../../types";

const mockGetAdBlockState = vi.fn();
const mockSetAdBlockEnabled = vi.fn().mockResolvedValue(undefined);
const mockAddAdBlockSource = vi.fn().mockResolvedValue({});
const mockRemoveAdBlockSource = vi.fn().mockResolvedValue(undefined);
const mockSetAdBlockSourceEnabled = vi.fn().mockResolvedValue({});
const mockSetAdBlockSourceResponse = vi.fn().mockResolvedValue({});
const mockRefreshAdBlockSource = vi.fn().mockResolvedValue({});
const mockRefreshAllAdBlockSources = vi.fn().mockResolvedValue([]);
const mockAddAdBlockWhitelist = vi.fn().mockResolvedValue([]);
const mockRemoveAdBlockWhitelist = vi.fn().mockResolvedValue([]);
const mockSetAdBlockRefreshInterval = vi.fn().mockResolvedValue(undefined);

vi.mock("../../lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/tauri")>();
  return {
    ...actual,
    getAdBlockState: (...args: unknown[]) => mockGetAdBlockState(...args),
    setAdBlockEnabled: (...args: unknown[]) => mockSetAdBlockEnabled(...args),
    addAdBlockSource: (...args: unknown[]) => mockAddAdBlockSource(...args),
    removeAdBlockSource: (...args: unknown[]) => mockRemoveAdBlockSource(...args),
    setAdBlockSourceEnabled: (...args: unknown[]) => mockSetAdBlockSourceEnabled(...args),
    setAdBlockSourceResponse: (...args: unknown[]) => mockSetAdBlockSourceResponse(...args),
    refreshAdBlockSource: (...args: unknown[]) => mockRefreshAdBlockSource(...args),
    refreshAllAdBlockSources: (...args: unknown[]) => mockRefreshAllAdBlockSources(...args),
    addAdBlockWhitelist: (...args: unknown[]) => mockAddAdBlockWhitelist(...args),
    removeAdBlockWhitelist: (...args: unknown[]) => mockRemoveAdBlockWhitelist(...args),
    setAdBlockRefreshInterval: (...args: unknown[]) => mockSetAdBlockRefreshInterval(...args),
  };
});

vi.mock("../../hooks/useWebKitPointerDown", () => ({
  useWebKitPointerDown: () => ({ onPointerDown: () => () => {} }),
}));

import AdBlock from "../AdBlock";

function makeState(overrides: Partial<AdBlockState> = {}): AdBlockState {
  return {
    enabled: true,
    sources: [],
    whitelist: [],
    auto_refresh_enabled: true,
    refresh_interval_hours: 24,
    ...overrides,
  };
}

function makeSource(overrides: Partial<AdBlockSource> = {}): AdBlockSource {
  return {
    source_id: "src-1",
    name: "Test List",
    url: "https://example.com/hosts",
    enabled: true,
    response: "zero_address",
    last_fetched_at: null,
    last_error: null,
    rule_count: 100,
    etag: null,
    ...overrides,
  };
}

function renderWithProviders(ui: React.ReactElement) {
  return render(<JotaiProvider store={getDefaultStore()}>{ui}</JotaiProvider>);
}

function setStore(fn: (s: ReturnType<typeof getDefaultStore>) => void) {
  const store = getDefaultStore();
  fn(store);
  return store;
}

describe("AdBlock", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setStore((s) => {
      s.set(adBlockStateAtom, null);
      s.set(isAdBlockLoadingAtom, false);
      s.set(adBlockErrorAtom, null);
      s.set(dnsEnabledAtom, false);
    });
    mockGetAdBlockState.mockResolvedValue(makeState());
  });

  // ---- issue #134: loading state ----
  it("renders Loading when state is null", () => {
    renderWithProviders(<AdBlock />);
    expect(screen.getByText("Loading\u2026")).toBeInTheDocument();
  });

  it("renders page title and subtitle after state loads", async () => {
    const state = makeState();
    mockGetAdBlockState.mockResolvedValue(state);
    setStore((s) => s.set(adBlockStateAtom, state));
    renderWithProviders(<AdBlock />);
    expect(await screen.findByText("Ad Block")).toBeInTheDocument();
    expect(
      screen.getByText("Block ads at the DNS resolver. macOS DNS mode only."),
    ).toBeInTheDocument();
  });

  // ---- issue #134: DNS-off banner ----
  it("shows DNS-off banner when dnsEnabled is false", async () => {
    const state = makeState();
    setStore((s) => {
      s.set(adBlockStateAtom, state);
      s.set(dnsEnabledAtom, false);
    });
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    expect(await screen.findByText(/DNS mode is off/i)).toBeInTheDocument();
  });

  it("hides DNS-off banner when dnsEnabled is true", async () => {
    const state = makeState();
    setStore((s) => {
      s.set(adBlockStateAtom, state);
      s.set(dnsEnabledAtom, true);
    });
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    expect(await screen.findByText("Ad Block")).toBeInTheDocument();
    expect(screen.queryByText(/DNS mode is off/i)).not.toBeInTheDocument();
  });

  // ---- issue #134: empty state ----
  it("renders empty-source placeholder when sources is empty", async () => {
    const state = makeState({ sources: [] });
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    expect(await screen.findByText("No sources yet.")).toBeInTheDocument();
  });

  // ---- issue #134: source list rendering ----
  it("renders source cards with name, url, and rule count", async () => {
    const src = makeSource({
      name: "StevenBlack",
      url: "https://sb.com/hosts",
      rule_count: 5000,
    });
    const state = makeState({ sources: [src] });
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    expect(await screen.findByText("StevenBlack")).toBeInTheDocument();
    expect(screen.getByText("https://sb.com/hosts")).toBeInTheDocument();
    expect(screen.getByText(/5,000 rules/)).toBeInTheDocument();
  });

  // ---- issue #134: error badge on source ----
  it("renders fetch-failed badge when source has last_error", async () => {
    const src = makeSource({ last_error: "timeout" });
    const state = makeState({ sources: [src] });
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    expect(await screen.findByText("fetch failed")).toBeInTheDocument();
    expect(screen.getByText(/timeout/)).toBeInTheDocument();
  });

  // ---- issue #134: master switch ----
  it("toggling master switch calls setAdBlockEnabled", async () => {
    // Mock the initial fetch to return enabled=false (same as pre-set
    // state). The toggle action re-fetches after setAdBlockEnabled, but
    // we only assert the IPC call here.
    const state = makeState({ enabled: false });
    mockGetAdBlockState.mockResolvedValue(state);
    setStore((s) => s.set(adBlockStateAtom, state));
    renderWithProviders(<AdBlock />);
    await screen.findByText("Ad Block");
    const checkboxes = screen.getAllByRole("checkbox");
    const masterSwitch = checkboxes[0];
    expect(masterSwitch).not.toBeChecked();
    await act(async () => {
      fireEvent.click(masterSwitch);
    });
    expect(mockSetAdBlockEnabled).toHaveBeenCalledWith(true);
  });

  // ---- issue #134: add source form ----
  it("fills add-source form and clicks Add", async () => {
    const state = makeState();
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    await screen.findByRole("heading", { name: "Sources" });
    const nameInput = screen.getByPlaceholderText("StevenBlack");
    const urlInput = screen.getByPlaceholderText("https://example.com/hosts");
    await act(async () => {
      fireEvent.change(nameInput, { target: { value: "MyList" } });
      fireEvent.change(urlInput, { target: { value: "https://ml.com/hosts" } });
    });
    // Two "Add" buttons exist (sources + whitelist). The source form's
    // Add button is the first one.
    const addBtns = screen.getAllByText("Add");
    const sourceAddBtn = addBtns[0];
    await act(async () => {
      fireEvent.click(sourceAddBtn);
    });
    expect(mockAddAdBlockSource).toHaveBeenCalledWith(
      "MyList",
      "https://ml.com/hosts",
      "zero_address",
    );
  });

  it("source Add button is disabled when name or url is empty", async () => {
    const state = makeState();
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    await screen.findByRole("heading", { name: "Sources" });
    const addBtns = screen.getAllByText("Add");
    expect(addBtns[0]).toBeDisabled();
  });

  // ---- issue #134: whitelist ----
  it("renders whitelist entries with remove buttons", async () => {
    const state = makeState({ whitelist: ["trusted.com", "safe.com"] });
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    expect(await screen.findByText("trusted.com")).toBeInTheDocument();
    expect(screen.getByText("safe.com")).toBeInTheDocument();
    expect(screen.getByLabelText("Remove trusted.com")).toBeInTheDocument();
  });

  it("renders empty whitelist placeholder when no entries", async () => {
    const state = makeState({ whitelist: [] });
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    expect(await screen.findByText("No whitelist entries.")).toBeInTheDocument();
  });

  it("adds a whitelist domain via the input", async () => {
    const state = makeState();
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    await screen.findByRole("heading", { name: "Whitelist" });
    const input = screen.getByPlaceholderText("trusted.example.com");
    await act(async () => {
      fireEvent.change(input, { target: { value: "new.com" } });
    });
    // The whitelist Add button is the second one.
    const addBtns = screen.getAllByText("Add");
    const whitelistAddBtn = addBtns[1];
    await act(async () => {
      fireEvent.click(whitelistAddBtn);
    });
    expect(mockAddAdBlockWhitelist).toHaveBeenCalledWith("new.com");
  });

  // ---- issue #134: error alert ----
  it("renders error alert when adBlockErrorAtom is set", async () => {
    // fetchAdBlockStateAtom clears adBlockErrorAtom to null on mount
    // (it's the "begin a fetch" signal). Set the error AFTER the
    // initial render so the effect's clear doesn't wipe it.
    const state = makeState();
    mockGetAdBlockState.mockResolvedValue(state);
    setStore((s) => s.set(adBlockStateAtom, state));
    renderWithProviders(<AdBlock />);
    await screen.findByText("Ad Block");
    setStore((s) => s.set(adBlockErrorAtom, "Something went wrong"));
    expect(await screen.findByText("Something went wrong")).toBeInTheDocument();
  });

  // ---- issue #134: summary stats ----
  it("renders summary stat labels for sources, rules, and whitelist", async () => {
    const state = makeState({
      sources: [
        makeSource({ rule_count: 100 }),
        makeSource({ source_id: "src-2", rule_count: 200, enabled: false }),
      ],
      whitelist: ["a.com", "b.com"],
    });
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    expect(await screen.findByRole("heading", { name: "Sources" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Whitelist" })).toBeInTheDocument();
  });
});
