import { Routes, Route, Navigate } from "react-router-dom";
import Layout from "./components/Layout";
import ProfileList from "./pages/ProfileList";
import ProfileEdit from "./pages/ProfileEdit";
import Settings from "./pages/Settings";

function App() {
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
