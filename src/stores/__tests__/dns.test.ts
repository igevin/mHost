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
}));

import {
  toggleDnsModeAtom,
  cancelActiveDnsToggle,
  dnsEnabledAtom,
  isDnsLoadingAtom,
  dnsErrorAtom,
  dnsStatusAtom,
} from "../profiles";
import {
  setDnsMode,
  cancelDnsMode,
  getDnsMode,
  getDnsStatus,
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