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
} from "./stores/profiles";

function App() {
  const fetchProfiles = useSetAtom(fetchProfilesAtom);
  const fetchDnsProfiles = useSetAtom(fetchDnsProfilesAtom);
  const fetchDnsMode = useSetAtom(fetchDnsModeAtom);
  const fetchAdBlock = useSetAtom(fetchAdBlockStateAtom);
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
