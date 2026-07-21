import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { BrowserRouter } from "react-router-dom";
import { getDefaultStore, Provider as JotaiProvider } from "jotai";
import {
  dnsEnabledAtom,
  dnsStatusAtom,
  isDnsLoadingAtom,
  quickApplyOnToggleAtom,
} from "../../stores/profiles";

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

vi.mock("../../lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/tauri")>();
  return {
    ...actual,
    getDnsMode: vi.fn().mockResolvedValue(false),
    getDnsStatus: (...args: unknown[]) => mockGetDnsStatus(...args),
    setDnsMode: (...args: unknown[]) => mockSetDnsMode(...args),
    reloadDnsRules: vi.fn().mockResolvedValue(undefined),
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

    expect(mockSetDnsMode).toHaveBeenCalledWith(true);
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

    expect(mockSetDnsMode).toHaveBeenCalledWith(false);
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
