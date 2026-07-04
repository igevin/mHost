import { useEffect } from "react";
import { Routes, Route, Navigate } from "react-router-dom";
import { listen } from "@tauri-apps/api/event";
import { useSetAtom } from "jotai";
import Layout from "./components/Layout";
import ProfileView from "./pages/ProfileView";
import Settings from "./pages/Settings";
import SnapshotPage from "./pages/Snapshot";
import SystemHosts from "./pages/SystemHosts";
import { fetchProfilesAtom, fetchDnsProfilesAtom, fetchDnsModeAtom } from "./stores/profiles";

function App() {
  const fetchProfiles = useSetAtom(fetchProfilesAtom);
  const fetchDnsProfiles = useSetAtom(fetchDnsProfilesAtom);
  const fetchDnsMode = useSetAtom(fetchDnsModeAtom);

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

    const unlisten = listen("tray:profiles-updated", () => {
      fetchProfiles();
    });
    return () => {
      unlisten.then((fn) => fn()).catch(() => {});
    };
  }, [fetchProfiles, fetchDnsProfiles, fetchDnsMode]);

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
      </Route>
    </Routes>
  );
}

export default App;
