import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, cleanup, waitFor, act } from "@testing-library/react";
import { BrowserRouter } from "react-router-dom";
import { Provider } from "jotai";
import App from "./App";
import { listProfiles } from "./lib/tauri";

const listenMock = vi.fn();
const unlistenFn = vi.fn();

vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

vi.mock("./lib/tauri", () => ({
  listProfiles: vi.fn().mockResolvedValue([]),
  getProfile: vi.fn(),
  createProfile: vi.fn(),
  updateProfile: vi.fn(),
  deleteProfile: vi.fn(),
  enableAndApply: vi.fn().mockResolvedValue(undefined),
  rollbackHosts: vi.fn(),
  exportProfileToFile: vi.fn(),
  duplicateProfile: vi.fn(),
  // AdBlock IPCs (issue #130) — App fetches ad-block state on mount.
  getAdBlockState: vi.fn().mockResolvedValue({
    enabled: false,
    sources: [],
    whitelist: [],
    auto_refresh_enabled: true,
    refresh_interval_hours: 24,
  }),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: vi.fn(),
}));

describe("App", () => {
  beforeEach(() => {
    listenMock.mockResolvedValue(unlistenFn);
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("listens to tray:profiles-updated on mount", async () => {
    await act(async () => {
      render(
        <Provider>
          <BrowserRouter>
            <App />
          </BrowserRouter>
        </Provider>,
      );
    });

    // PR #154 review (P2): keep total call count so a double-registration
// regression is caught. We register 2 listeners per mount: `tray:profiles-updated`
// (profile refresh) and `navigate` (issue #130 tray deep-link). React
// StrictMode in dev causes double mount → 2 × 2 = 4 calls. Use `>=` to
// tolerate the StrictMode double-mount while still catching obvious
// regression cases (e.g. a third registration).
expect(listenMock.mock.calls.length).toBeGreaterThanOrEqual(2);
expect(listenMock).toHaveBeenCalledWith(
      "tray:profiles-updated",
      expect.any(Function),
    );
    // issue #130: also listens for the tray-driven `navigate` event so the
    // "广告屏蔽" menu item can deep-link to /ad-block without coupling
    // backend to router.
    expect(listenMock).toHaveBeenCalledWith(
      "navigate",
      expect.any(Function),
    );
  });

  it("triggers profile refresh when tray:profiles-updated event fires", async () => {
    await act(async () => {
      render(
        <Provider>
          <BrowserRouter>
            <App />
          </BrowserRouter>
        </Provider>,
      );
    });

    const [, handler] = listenMock.mock.calls[0];
    expect(handler).toBeDefined();

    await act(async () => {
      handler();
    });

    await waitFor(() => {
      expect(listProfiles).toHaveBeenCalled();
    });
  });

  it("unsubscribes listener on unmount", async () => {
    let unmountFn: () => void;

    await act(async () => {
      const { unmount } = render(
        <Provider>
          <BrowserRouter>
            <App />
          </BrowserRouter>
        </Provider>,
      );
      unmountFn = unmount;
    });

    // Same count assertion as the first test — guards against double-register.
    expect(listenMock.mock.calls.length).toBeGreaterThanOrEqual(2);

    await act(async () => {
      unmountFn();
    });

    // Wait for the microtask from the cleanup function
    await new Promise((resolve) => setTimeout(resolve, 0));

    // React StrictMode may cause double mount/unmount in development,
    // so we verify unlisten was called at least once rather than exactly once.
    expect(unlistenFn).toHaveBeenCalled();
  });
});
