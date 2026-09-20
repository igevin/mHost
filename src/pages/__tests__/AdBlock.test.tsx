import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { getDefaultStore, Provider as JotaiProvider } from "jotai";
import {
  adBlockStateAtom,
  isAdBlockLoadingAtom,
  adBlockErrorAtom,
  adBlockLimitsAtom,
  dnsEnabledAtom,
} from "../../stores/profiles";
import type { AdBlockState, AdBlockSource } from "../../types";

const mockGetAdBlockState = vi.fn();
const mockGetAdBlockLimits = vi.fn().mockResolvedValue({
  rules_per_source_default: 500000,
  rules_per_source_absolute_max: 2000000,
});
const mockSetAdBlockEnabled = vi.fn().mockResolvedValue(undefined);
const mockAddAdBlockSource = vi.fn().mockResolvedValue({});
const mockRemoveAdBlockSource = vi.fn().mockResolvedValue(undefined);
const mockSetAdBlockSourceEnabled = vi.fn().mockResolvedValue({});
const mockSetAdBlockSourceResponse = vi.fn().mockResolvedValue({});
const mockRefreshAdBlockSource = vi.fn().mockResolvedValue({});
const mockReorderAdBlockSources = vi.fn().mockResolvedValue([]);
const mockGetAdBlockOverlaps = vi.fn().mockResolvedValue({ per_source: [], details: {} });
const mockRefreshAllAdBlockSources = vi.fn().mockResolvedValue([]);
const mockAddAdBlockWhitelist = vi.fn().mockResolvedValue([]);
const mockAddAdBlockWhitelistMany = vi
  .fn()
  .mockResolvedValue({ whitelist: [], rejected: [] });
const mockRemoveAdBlockWhitelist = vi.fn().mockResolvedValue([]);
const mockRemoveAdBlockWhitelistMany = vi.fn().mockResolvedValue([]);
const mockSetAdBlockRefreshInterval = vi.fn().mockResolvedValue(undefined);
const mockSetAdBlockAutoRefreshEnabled = vi.fn().mockResolvedValue(undefined);
const mockSetAdBlockSourceRulesLimitOverride = vi
  .fn()
  .mockResolvedValue({});

vi.mock("../../lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/tauri")>();
  return {
    ...actual,
    getAdBlockState: (...args: unknown[]) => mockGetAdBlockState(...args),
    getAdBlockLimits: (...args: unknown[]) => mockGetAdBlockLimits(...args),
    setAdBlockEnabled: (...args: unknown[]) => mockSetAdBlockEnabled(...args),
    addAdBlockSource: (...args: unknown[]) => mockAddAdBlockSource(...args),
    removeAdBlockSource: (...args: unknown[]) => mockRemoveAdBlockSource(...args),
    setAdBlockSourceEnabled: (...args: unknown[]) => mockSetAdBlockSourceEnabled(...args),
    setAdBlockSourceResponse: (...args: unknown[]) => mockSetAdBlockSourceResponse(...args),
    setAdBlockSourceRulesLimitOverride: (...args: unknown[]) =>
      mockSetAdBlockSourceRulesLimitOverride(...args),
    refreshAdBlockSource: (...args: unknown[]) => mockRefreshAdBlockSource(...args),
    reorderAdBlockSources: (...args: unknown[]) => mockReorderAdBlockSources(...args),
    getAdBlockOverlaps: (...args: unknown[]) => mockGetAdBlockOverlaps(...args),
    refreshAllAdBlockSources: (...args: unknown[]) => mockRefreshAllAdBlockSources(...args),
    addAdBlockWhitelist: (...args: unknown[]) => mockAddAdBlockWhitelist(...args),
    addAdBlockWhitelistMany: (...args: unknown[]) =>
      mockAddAdBlockWhitelistMany(...args),
    removeAdBlockWhitelist: (...args: unknown[]) => mockRemoveAdBlockWhitelist(...args),
    removeAdBlockWhitelistMany: (...args: unknown[]) =>
      mockRemoveAdBlockWhitelistMany(...args),
    setAdBlockRefreshInterval: (...args: unknown[]) => mockSetAdBlockRefreshInterval(...args),
    setAdBlockAutoRefreshEnabled: (...args: unknown[]) =>
      mockSetAdBlockAutoRefreshEnabled(...args),
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
    rules_limit_override: null,
    last_refresh_duration_ms: null,
    last_refresh_failed_at: null,
    ...overrides,
  };
}

function renderWithProviders(ui: React.ReactElement) {
  return render(
    <MemoryRouter>
      <JotaiProvider store={getDefaultStore()}>{ui}</JotaiProvider>
    </MemoryRouter>,
  );
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
      // Issue #211-3: reset so a previous test's loaded limits don't leak
      // into tests that rely on "limits unknown".
      s.set(adBlockLimitsAtom, null);
      s.set(dnsEnabledAtom, false);
    });
    mockGetAdBlockState.mockResolvedValue(makeState());
    mockGetAdBlockLimits.mockResolvedValue({
      rules_per_source_default: 500000,
      rules_per_source_absolute_max: 2000000,
    });
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

  // ---- issue #202: the error banner must not be a permanent false positive ----
  it("hides the error banner when every source is healthy", async () => {
    const src = makeSource({ last_error: null });
    const state = makeState({ sources: [src] });
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    await screen.findByText(/5,?000 rules|100 rules/);
    expect(
      screen.queryByText(/One or more sources have a fetch error/),
    ).not.toBeInTheDocument();
  });

  it("treats an undefined last_error as healthy (pre-#202 wire data)", async () => {
    // Old backends omitted the key entirely (skip_serializing_if), so the
    // runtime value was `undefined` while the type claimed `string | null`.
    const src = makeSource({
      last_error: undefined as unknown as string,
    });
    const state = makeState({ sources: [src] });
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    await screen.findByText("Test List");
    expect(
      screen.queryByText(/One or more sources have a fetch error/),
    ).not.toBeInTheDocument();
  });

  it("shows the error banner when a source has an error", async () => {
    const src = makeSource({ last_error: "timeout" });
    const state = makeState({ sources: [src] });
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    expect(
      await screen.findByText(/One or more sources have a fetch error/),
    ).toBeInTheDocument();
  });

  // ---- issue #207: per-source rules-limit override ----
  it("offers the one-click override when last_error is an over-limit rejection", async () => {
    const src = makeSource({
      last_error: "source produced 612003 rules (limit: 500000)",
    });
    const state = makeState({ sources: [src] });
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);

    const btn = await screen.findByRole("button", {
      name: /Allow 612,003 rules & retry/,
    });
    await act(async () => {
      fireEvent.click(btn);
    });
    // Override written with the actual parsed count…
    expect(mockSetAdBlockSourceRulesLimitOverride).toHaveBeenCalledWith(
      "src-1",
      612003,
    );
    // …then retried through the existing refresh path.
    expect(mockRefreshAdBlockSource).toHaveBeenCalledWith("src-1");
  });

  it("does not offer an override above the absolute cap", async () => {
    // Absolute max = 2,000,000 (issue #211-3: gate comes from
    // backend-delivered limits). 2.5M exceeds it → no entry. The atom is
    // preset explicitly so the assertion doesn't race the mount-effect
    // limits fetch.
    const src = makeSource({
      last_error: "source produced 2500000 rules (limit: 500000)",
    });
    const state = makeState({ sources: [src] });
    setStore((s) => {
      s.set(adBlockStateAtom, state);
      s.set(adBlockLimitsAtom, {
        rules_per_source_default: 500000,
        rules_per_source_absolute_max: 2000000,
      });
    });
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    await screen.findByText("fetch failed");
    expect(
      screen.queryByRole("button", { name: /Allow .* rules & retry/ }),
    ).not.toBeInTheDocument();
  });

  it("still offers the override while limits are unknown", async () => {
    // Issue #211-3: `adBlockLimitsAtom` starts null and limits fetches can
    // fail; the entry must not silently disappear — the backend remains
    // the authority and rejects over-cap overrides itself.
    mockGetAdBlockLimits.mockRejectedValue(new Error("ipc down"));
    const src = makeSource({
      last_error: "source produced 2500000 rules (limit: 500000)",
    });
    const state = makeState({ sources: [src] });
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    expect(
      await screen.findByRole("button", { name: /Allow 2,500,000 rules & retry/ }),
    ).toBeInTheDocument();
    mockGetAdBlockLimits.mockResolvedValue({
      rules_per_source_default: 500000,
      rules_per_source_absolute_max: 2000000,
    });
  });

  it("shows the raised limit and a reset action when an override is set", async () => {
    const src = makeSource({
      rules_limit_override: 612003,
      rule_count: 612003,
    });
    const state = makeState({ sources: [src] });
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);

    expect(
      await screen.findByText(/limit 612,003 \(manually raised\)/),
    ).toBeInTheDocument();
    const reset = screen.getByRole("button", {
      name: /Reset rule limit to default/,
    });
    await act(async () => {
      fireEvent.click(reset);
    });
    // Revoking passes `null`, not `undefined` — the #202 bug class.
    expect(mockSetAdBlockSourceRulesLimitOverride).toHaveBeenCalledWith(
      "src-1",
      null,
    );
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

  it("adds a single whitelist domain via the textarea (one-shot many IPC)", async () => {
    // Issue #196: a single-line value still goes through the batch IPC
    // so the backend's persist-and-reload path is shared with the
    // multi-line case (single IPC, single reload).
    const state = makeState();
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    await screen.findByRole("heading", { name: "Whitelist" });
    const textarea = screen.getByPlaceholderText("trusted.example.com");
    await act(async () => {
      fireEvent.change(textarea, { target: { value: "new.com" } });
    });
    // The whitelist Add button is the second one.
    const addBtns = screen.getAllByText("Add");
    const whitelistAddBtn = addBtns[1];
    await act(async () => {
      fireEvent.click(whitelistAddBtn);
    });
    expect(mockAddAdBlockWhitelistMany).toHaveBeenCalledWith(["new.com"]);
  });

  // ---- issue #196: bulk paste ----
  it("pastes multiple whitelist entries in one many-call", async () => {
    const state = makeState();
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    await screen.findByRole("heading", { name: "Whitelist" });
    const textarea = screen.getByPlaceholderText("trusted.example.com");
    const lines = Array.from({ length: 50 }, (_, i) => `host${i}.com`).join("\n");
    await act(async () => {
      fireEvent.change(textarea, { target: { value: lines } });
    });
    const addBtns = screen.getAllByText("Add");
    await act(async () => {
      fireEvent.click(addBtns[1]);
    });
    expect(mockAddAdBlockWhitelistMany).toHaveBeenCalledTimes(1);
    const calledWith = mockAddAdBlockWhitelistMany.mock.calls[0][0] as string[];
    expect(calledWith).toHaveLength(50);
    expect(calledWith[0]).toBe("host0.com");
    expect(calledWith[49]).toBe("host49.com");
  });

  it("surfaces a toast when many-IPC returns rejected entries", async () => {
    // Mock the many IPC to look like: 2 succeeded, 2 rejected.
    mockAddAdBlockWhitelistMany.mockResolvedValueOnce({
      whitelist: ["good.com"],
      rejected: [
        { input: "bad.com.", reason: "must not end with '.'" },
        { input: "-leading.com", reason: "invalid label '-leading'" },
      ],
    });
    const state = makeState();
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    await screen.findByRole("heading", { name: "Whitelist" });
    const textarea = screen.getByPlaceholderText("trusted.example.com");
    await act(async () => {
      fireEvent.change(textarea, {
        target: { value: "good.com\nbad.com.\n-leading.com" },
      });
    });
    const addBtns = screen.getAllByText("Add");
    await act(async () => {
      fireEvent.click(addBtns[1]);
    });
    expect(
      await screen.findByText(/2 entries skipped/),
    ).toBeInTheDocument();
    // Spot-check that the reasons surface for the user to act on.
    expect(screen.getByText(/bad.com\./)).toBeInTheDocument();
  });

  it("does nothing when the whitelist textarea is empty or whitespace", async () => {
    const state = makeState();
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    await screen.findByRole("heading", { name: "Whitelist" });
    const textarea = screen.getByPlaceholderText("trusted.example.com");
    await act(async () => {
      fireEvent.change(textarea, { target: { value: "   \n   \n  " } });
    });
    const addBtns = screen.getAllByText("Add");
    await act(async () => {
      fireEvent.click(addBtns[1]);
    });
    expect(mockAddAdBlockWhitelistMany).not.toHaveBeenCalled();
  });

  // ---- issue #196: copy-to-clipboard ----
  it("renders a Copy button next to the whitelist title", async () => {
    const state = makeState({ whitelist: ["a.com", "b.com"] });
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    const btn = await screen.findByRole("button", {
      name: /Copy whitelist to clipboard/i,
    });
    expect(btn).toBeEnabled();
  });

  it("disables the Copy button when the whitelist is empty", async () => {
    const state = makeState({ whitelist: [] });
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    const btn = await screen.findByRole("button", {
      name: /Copy whitelist to clipboard/i,
    });
    expect(btn).toBeDisabled();
  });

  // ---- issue #196: bulk add sources ----
  it("bulk-adds sources from a name<TAB>url paste", async () => {
    const state = makeState();
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    await screen.findByRole("heading", { name: "Sources" });
    // The bulk-add textarea is collapsed inside <details>; expand it
    // first so the textarea is rendered into the DOM (jsdom keeps
    // <details> contents in the DOM regardless, but expand() mirrors
    // real-user behavior).
    const bulk = await screen.findByLabelText(/Bulk add sources/i);
    const lines = [
      "StevenBlack\thttps://sb.com/hosts",
      "My List  https://ml.com/hosts", // 2-space separator
      "Third https://t.com/hosts",     // single space
    ].join("\n");
    await act(async () => {
      fireEvent.change(bulk, { target: { value: lines } });
    });
    const addAll = screen.getByRole("button", { name: "Add all" });
    await act(async () => {
      fireEvent.click(addAll);
    });
    expect(mockAddAdBlockSource).toHaveBeenCalledTimes(3);
    expect(mockAddAdBlockSource).toHaveBeenCalledWith(
      "StevenBlack",
      "https://sb.com/hosts",
      "zero_address",
    );
    expect(mockAddAdBlockSource).toHaveBeenCalledWith(
      "My List",
      "https://ml.com/hosts",
      "zero_address",
    );
    expect(mockAddAdBlockSource).toHaveBeenCalledWith(
      "Third",
      "https://t.com/hosts",
      "zero_address",
    );
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

  // ---- issue #192: auto-refresh toggle ----
  it("toggling auto-refresh off calls setAdBlockAutoRefreshEnabled(false)", async () => {
    const state = makeState(); // auto_refresh_enabled: true, no sources
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    await screen.findByText("Auto-refresh");
    const autoRefreshToggle = screen.getByRole("checkbox", {
      name: "Auto-refresh",
    });
    expect(autoRefreshToggle).toBeChecked();
    await act(async () => {
      fireEvent.click(autoRefreshToggle);
    });
    expect(mockSetAdBlockAutoRefreshEnabled).toHaveBeenCalledWith(false);
  });

  it("hides the interval select when auto-refresh is off and shows it when on", async () => {
    const state = makeState({ auto_refresh_enabled: false });
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    await screen.findByText("Auto-refresh");
    expect(
      screen.queryByRole("combobox", { name: "Refresh interval" }),
    ).not.toBeInTheDocument();

    // Re-enable via the toggle: the action re-fetches state; return an
    // enabled one so the select renders.
    mockGetAdBlockState.mockResolvedValue(makeState({ auto_refresh_enabled: true }));
    const autoRefreshToggle = screen.getByRole("checkbox", {
      name: "Auto-refresh",
    });
    await act(async () => {
      fireEvent.click(autoRefreshToggle);
    });
    expect(
      await screen.findByRole("combobox", { name: "Refresh interval" }),
    ).toBeInTheDocument();
  });

  // ---- PR #217 self-review: bulk source failures aggregate into a toast ----
  it("surfaces a summary toast when some bulk-source adds fail", async () => {
    // Make the second call reject so the summary has at least one failure.
    mockAddAdBlockSource
      .mockResolvedValueOnce({ source_id: "s1" } as never)
      .mockRejectedValueOnce(new Error("invalid URL") as never)
      .mockResolvedValueOnce({ source_id: "s3" } as never);
    const state = makeState();
    setStore((s) => s.set(adBlockStateAtom, state));
    mockGetAdBlockState.mockResolvedValue(state);
    renderWithProviders(<AdBlock />);
    await screen.findByRole("heading", { name: "Sources" });
    const bulk = await screen.findByLabelText(/Bulk add sources/i);
    await act(async () => {
      fireEvent.change(bulk, {
        target: { value: "A\thttps://a.com\nB\thttps://b.com\nC\thttps://c.com" },
      });
    });
    const addAll = screen.getByRole("button", { name: "Add all" });
    await act(async () => {
      fireEvent.click(addAll);
    });
    expect(
      await screen.findByText(/1 of 3 sources failed to add: B \(https:\/\/b\.com\)/),
    ).toBeInTheDocument();
  });

  // ---- PR #217 self-review: clipboard failure surfaces a toast ----
  it("surfaces a toast when the clipboard write fails", async () => {
    // jsdom doesn't expose `navigator.clipboard` by default — assign a
    // stub directly and restore the original on teardown.
    const originalClipboard = (navigator as { clipboard?: Clipboard }).clipboard;
    const writeText = vi.fn().mockRejectedValue(new Error("denied"));
    (navigator as { clipboard?: Clipboard }).clipboard = {
      writeText,
    } as unknown as Clipboard;
    try {
      const state = makeState({ whitelist: ["a.com", "b.com"] });
      setStore((s) => s.set(adBlockStateAtom, state));
      mockGetAdBlockState.mockResolvedValue(state);
      renderWithProviders(<AdBlock />);
      const btn = await screen.findByRole("button", {
        name: /Copy whitelist to clipboard/i,
      });
      await act(async () => {
        fireEvent.click(btn);
      });
      expect(await screen.findByText(/Copy failed/)).toBeInTheDocument();
      expect(writeText).toHaveBeenCalledWith("a.com\nb.com");
    } finally {
      (navigator as { clipboard?: Clipboard }).clipboard = originalClipboard;
    }
  });

  // Issue #215: source reorder UI. The buttons are ↑/↓ on each source
  // card; the backend IPC accepts a single-step relative move. The
  // tests below cover: (1) the boundary buttons (first/last) are
  // disabled in the UI so the user can't trigger a server-side
  // no-op; (2) clicking ↑/↓ calls the IPC with the right args.
  //
  // Note: the page's useEffect calls fetchState() which OVERWRITES
  // the pre-set store state with mockGetAdBlockState's resolved
  // value. Each test must therefore set mockGetAdBlockState to the
  // desired state BEFORE rendering — same pattern as the other
  // source-rendering tests in this file (e.g. "renders source cards
  // with name, url, and rule count").
  describe("source reorder (issue #215)", () => {
    beforeEach(() => {
      mockReorderAdBlockSources.mockReset();
      mockReorderAdBlockSources.mockResolvedValue([]);
    });

    it("disables Up on the first source and Down on the last", async () => {
      const a = makeSource({ source_id: "src-a", name: "A" });
      const b = makeSource({ source_id: "src-b", name: "B" });
      const c = makeSource({ source_id: "src-c", name: "C" });
      const state = makeState({ sources: [a, b, c] });
      setStore((s) => s.set(adBlockStateAtom, state));
      mockGetAdBlockState.mockResolvedValue(state);
      renderWithProviders(<AdBlock />);

      // First source's Up is disabled.
      const firstUp = await screen.findByRole("button", { name: /Move source A up/i });
      expect(firstUp).toBeDisabled();
      // First source's Down is enabled.
      const firstDown = screen.getByRole("button", { name: /Move source A down/i });
      expect(firstDown).not.toBeDisabled();

      // Middle source's both buttons enabled.
      expect(screen.getByRole("button", { name: /Move source B up/i })).not.toBeDisabled();
      expect(screen.getByRole("button", { name: /Move source B down/i })).not.toBeDisabled();

      // Last source's Down is disabled; Up enabled.
      expect(screen.getByRole("button", { name: /Move source C up/i })).not.toBeDisabled();
      expect(screen.getByRole("button", { name: /Move source C down/i })).toBeDisabled();
    });

    it("clicking Up invokes the IPC with sourceId + direction 'up'", async () => {
      const a = makeSource({ source_id: "src-a", name: "A" });
      const b = makeSource({ source_id: "src-b", name: "B" });
      const state = makeState({ sources: [a, b] });
      setStore((s) => s.set(adBlockStateAtom, state));
      mockGetAdBlockState.mockResolvedValue(state);
      renderWithProviders(<AdBlock />);

      const upBtn = await screen.findByRole("button", { name: /Move source B up/i });
      await act(async () => {
        fireEvent.click(upBtn);
      });
      // Sub-agent review (PR #221, finding 1): the click triggers
      // `reorderSource(...)` which calls `reorderAdBlockSources`
      // IPC; the assertion runs before that microtask reliably
      // flushes, so wrap in waitFor to dodge the race. Apply
      // symmetrically to the corresponding Down test below.
      await waitFor(() => {
        expect(mockReorderAdBlockSources).toHaveBeenCalledWith("src-b", "up");
      });
    });

    it("clicking Down invokes the IPC with sourceId + direction 'down'", async () => {
      const a = makeSource({ source_id: "src-a", name: "A" });
      const b = makeSource({ source_id: "src-b", name: "B" });
      const state = makeState({ sources: [a, b] });
      setStore((s) => s.set(adBlockStateAtom, state));
      mockGetAdBlockState.mockResolvedValue(state);
      renderWithProviders(<AdBlock />);

      const downBtn = await screen.findByRole("button", { name: /Move source A down/i });
      await act(async () => {
        fireEvent.click(downBtn);
      });
      await waitFor(() => {
        expect(mockReorderAdBlockSources).toHaveBeenCalledWith("src-a", "down");
      });
    });
  });

  // Issue #215 §1: cross-source overlap UI. Two tests:
  //  1. The chip renders only on sources with overlapping_domain_count > 0
  //     and opens the drawer when clicked.
  //  2. The drawer shows the per-domain entries from
  //     `overlapReport.details[source_id]` with the `effective` badge.
  //
  // Like the reorder tests, each test sets `mockGetAdBlockState` so
  // the page's `useEffect` `fetchState()` doesn't overwrite the
  // pre-set `adBlockStateAtom` with an empty state.
  describe("source overlap (issue #215)", () => {
    beforeEach(() => {
      mockGetAdBlockOverlaps.mockReset();
      mockGetAdBlockOverlaps.mockResolvedValue({
        per_source: [],
        details: {},
      });
    });

    it("renders an overlap chip only on sources with overlapping domains", async () => {
      const a = makeSource({ source_id: "src-a", name: "A" });
      const b = makeSource({ source_id: "src-b", name: "B" });
      const c = makeSource({ source_id: "src-c", name: "C" });
      const state = makeState({ sources: [a, b, c] });
      setStore((s) => s.set(adBlockStateAtom, state));
      mockGetAdBlockState.mockResolvedValue(state);
      // Only `src-a` overlaps with `src-b`. `src-c` has zero overlaps
      // so the chip must not render.
      mockGetAdBlockOverlaps.mockResolvedValue({
        per_source: [
          {
            source_id: "src-a",
            source_name: "A",
            overlapping_domain_count: 7,
          },
          {
            source_id: "src-b",
            source_name: "B",
            overlapping_domain_count: 3,
          },
          {
            source_id: "src-c",
            source_name: "C",
            overlapping_domain_count: 0,
          },
        ],
        details: {},
      });
      renderWithProviders(<AdBlock />);

      const chipA = await screen.findByRole("button", {
        name: /Show 7 overlapping domains for A/i,
      });
      expect(chipA).toBeInTheDocument();
      const chipB = screen.getByRole("button", {
        name: /Show 3 overlapping domains for B/i,
      });
      expect(chipB).toBeInTheDocument();
      // No chip for source C because count is 0.
      expect(
        screen.queryByRole("button", {
          name: /overlapping domains for C/i,
        }),
      ).not.toBeInTheDocument();
    });

    it("clicking the chip opens a drawer with per-domain details", async () => {
      const a = makeSource({ source_id: "src-a", name: "A" });
      const b = makeSource({ source_id: "src-b", name: "B" });
      const state = makeState({ sources: [a, b] });
      setStore((s) => s.set(adBlockStateAtom, state));
      mockGetAdBlockState.mockResolvedValue(state);
      mockGetAdBlockOverlaps.mockResolvedValue({
        per_source: [
          {
            source_id: "src-a",
            source_name: "A",
            overlapping_domain_count: 1,
          },
        ],
        details: {
          "src-a": [
            {
              domain: "shared.example.com",
              covered_by: [
                {
                  source_id: "src-b",
                  name: "B",
                  response: "nx_domain",
                },
              ],
              effective: "NxDomain",
            },
          ],
        },
      });
      renderWithProviders(<AdBlock />);

      const chip = await screen.findByRole("button", {
        name: /Show 1 overlapping domains for A/i,
      });
      await act(async () => {
        fireEvent.click(chip);
      });

      // Drawer header + entry show up.
      expect(
        screen.getByRole("heading", { name: /Overlapping domains/i }),
      ).toBeInTheDocument();
      expect(screen.getByText("shared.example.com")).toBeInTheDocument();
      // Use getAllByText then assert the overlap badge is present;
      // the response-type select also contains "NXDOMAIN" as an option
      // value, which would falsely match a plain `getByText`.
      expect(screen.getByText("shared.example.com").nextElementSibling).toHaveTextContent("NxDomain");
      // The covered_by line lists the other source + its response.
      expect(screen.getByText(/B \(nx_domain\)/)).toBeInTheDocument();
    });
  });
});
