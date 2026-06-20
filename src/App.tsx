import { useEffect } from "react";
import { Routes, Route, Navigate } from "react-router-dom";
import { listen } from "@tauri-apps/api/event";
import { useSetAtom } from "jotai";
import Layout from "./components/Layout";
import ProfileList from "./pages/ProfileList";
import ProfileEdit from "./pages/ProfileEdit";
import Settings from "./pages/Settings";
import { fetchProfilesAtom } from "./stores/profiles";

function App() {
  const fetchProfiles = useSetAtom(fetchProfilesAtom);

  useEffect(() => {
    const unlisten = listen("tray:profiles-updated", () => {
      fetchProfiles();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [fetchProfiles]);

  return (
    <Routes>
      <Route element={<Layout />}>
        <Route path="/" element={<Navigate to="/profiles" replace />} />
        <Route path="/profiles" element={<ProfileList />} />
        <Route path="/profiles/:id" element={<ProfileEdit />} />
        <Route path="/settings" element={<Settings />} />
      </Route>
    </Routes>
  );
}

export default App;
