import { useEffect } from "react";
import { Routes, Route, Navigate, useNavigate } from "react-router-dom";
import { listen } from "@tauri-apps/api/event";
import { useSetAtom } from "jotai";
import Layout from "./components/Layout";
import ProfileView from "./pages/ProfileView";
import Settings from "./pages/Settings";
import SnapshotPage from "./pages/Snapshot";
import SystemHosts from "./pages/SystemHosts";
import AdBlock from "./pages/AdBlock";
import {
  fetchProfilesAtom,
  fetchDnsProfilesAtom,
  fetchDnsModeAtom,
  fetchAdBlockStateAtom,
  probeSystemDnsAtom,
} from "./stores/profiles";

function App() {
  const fetchProfiles = useSetAtom(fetchProfilesAtom);
  const fetchDnsProfiles = useSetAtom(fetchDnsProfilesAtom);
  const fetchDnsMode = useSetAtom(fetchDnsModeAtom);
  const fetchAdBlock = useSetAtom(fetchAdBlockStateAtom);
  const probeSystemDns = useSetAtom(probeSystemDnsAtom);
  const navigate = useNavigate();

  useEffect(() => {
    // Load profiles on app mount
    fetchProfiles().catch(() => {
      // Ignore: error is already stored in errorAtom
    });
    fetchDnsProfiles().catch(() => {
      // Ignore: error is already stored in dnsErrorAtom
    });
    fetchDnsMode().catch(() => {
      // Ignore: error is already stored in dnsErrorAtom
    });
    fetchAdBlock().catch(() => {
      // Ignore: error is already stored in adBlockErrorAtom
    });

    const unlistenProfiles = listen("tray:profiles-updated", () => {
      fetchProfiles();
    });
    // issue #130: tray "广告屏蔽" menu item emits this event with the
    // target route. Lets the tray drive deep-linking to /ad-block without
    // coupling backend to router.
    //
    // **security (PR #154 review P2)**: whitelist allowed routes. The
    // backend emitter (tray.rs `TrayMenuAction::AdBlock`) is trusted but
    // any future payload source — including a malformed emitter or a
    // future debug/test hook — must not be able to push the router into
    // arbitrary paths (which would silently no-op render or, worse, leak
    // some future route meant for in-app use only).
    const ALLOWED_TRAY_ROUTES: ReadonlySet<string> = new Set(["/ad-block"]);
    const unlistenNavigate = listen<string>("navigate", (event) => {
      const target = event.payload;
      if (typeof target === "string" && ALLOWED_TRAY_ROUTES.has(target)) {
        navigate(target);
      } else if (typeof target === "string" && target.startsWith("/")) {
        // Unknown path — log and ignore.
        console.warn(`[mHost] tray navigate: refused unknown route "${target}"`);
      }
    });
    return () => {
      unlistenProfiles.then((fn) => fn()).catch(() => {});
      unlistenNavigate.then((fn) => fn()).catch(() => {});
    };
  }, [fetchProfiles, fetchDnsProfiles, fetchDnsMode, fetchAdBlock, navigate]);

  /**
   * Issue #153: 窗口重新获得焦点时重新探测系统 DNS。
   *
   * 覆盖「用户在 mHost 开着的时候，在 System Settings 或别的工具里改了
   * 系统 DNS」这个场景 —— 启动时的探测看不到它，但用户切回 app 的瞬间
   * 就是天然的复检时机（而且比任何定时器都便宜：只有真的切回来才跑）。
   *
   * 两道闸门防止来回切窗口时刷 IPC：
   * - in-flight 标志：上一次探测还没回来就不再发一次
   * - 1s 冷却：macOS 上 focus 事件会成簇触发（点标题栏、Cmd-Tab
   *   回弹、点通知中心再点回来……）
   *
   * 探测失败由 `probeSystemDnsAtom` 自己吞掉，这里不处理。
   */
  useEffect(() => {
    let inFlight = false;
    let lastProbeAt = 0;
    const COOLDOWN_MS = 1000;

    const onFocus = () => {
      const now = Date.now();
      if (inFlight || now - lastProbeAt < COOLDOWN_MS) return;
      inFlight = true;
      lastProbeAt = now;
      probeSystemDns().finally(() => {
        inFlight = false;
      });
    };

    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [probeSystemDns]);

  return (
    <Routes>
      <Route element={<Layout />}>
        <Route path="/" element={<Navigate to="/profiles" replace />} />
        <Route path="/profiles" element={<ProfileView mode="hosts" />} />
        <Route path="/profiles/:id" element={<ProfileView mode="hosts" />} />
        <Route path="/dns-profiles" element={<ProfileView mode="dns" />} />
        <Route path="/dns-profiles/:id" element={<ProfileView mode="dns" />} />
        <Route path="/settings" element={<Settings />} />
        <Route path="/snapshot" element={<SnapshotPage />} />
        <Route path="/hosts" element={<SystemHosts />} />
        <Route path="/ad-block" element={<AdBlock />} />
      </Route>
    </Routes>
  );
}

export default App;
