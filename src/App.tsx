import { useEffect } from "react";
import { Routes, Route, Navigate } from "react-router-dom";
import { listen } from "@tauri-apps/api/event";
import { useSetAtom } from "jotai";
import Layout from "./components/Layout";
import ProfileView from "./pages/ProfileView";
import Settings from "./pages/Settings";
import { fetchProfilesAtom } from "./stores/profiles";

function App() {
  const fetchProfiles = useSetAtom(fetchProfilesAtom);

  useEffect(() => {
    // Load profiles on app mount
    fetchProfiles().catch(() => {
      // Ignore: error is already stored in errorAtom
    });

    const unlisten = listen("tray:profiles-updated", () => {
      fetchProfiles();
    });
    return () => {
      unlisten.then((fn) => fn()).catch(() => {});
    };
  }, [fetchProfiles]);

  return (
    <Routes>
      <Route element={<Layout />}>
        <Route path="/" element={<Navigate to="/profiles" replace />} />
        <Route path="/profiles" element={<ProfileView />} />
        <Route path="/profiles/:id" element={<ProfileView />} />
        <Route path="/settings" element={<Settings />} />
      </Route>
    </Routes>
  );
}

export default App;
