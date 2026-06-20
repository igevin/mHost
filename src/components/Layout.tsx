import { NavLink, Outlet } from "react-router-dom";
import { useAtomValue } from "jotai";
import { enabledProfileAtom, isApplyingAtom } from "../stores/profiles";

function Layout() {
  const enabledProfile = useAtomValue(enabledProfileAtom);
  const isApplying = useAtomValue(isApplyingAtom);

  const navItems = [
    { to: "/profiles", label: "Profiles", icon: "Profiles" },
    { to: "/settings", label: "Settings", icon: "Settings" },
  ];

  return (
    <div className="mhost-layout">
      <aside className="sidebar">
        <div className="sidebar-header">
          <div className="logo">
            <span className="logo-icon">m</span>
            <span className="logo-text">mHost</span>
          </div>
        </div>

        <nav className="sidebar-nav">
          {navItems.map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              className={({ isActive }) =>
                `nav-link ${isActive ? "nav-link-active" : ""}`
              }
              end={item.to === "/profiles"}
            >
              <span className="nav-icon">{item.icon[0]}</span>
              <span className="nav-label">{item.label}</span>
            </NavLink>
          ))}
        </nav>

        <div className="sidebar-footer">
          <div className="status-card">
            <div className="status-row">
              <span className="status-label">Active</span>
              <span
                className={`status-dot ${enabledProfile ? "status-dot-on" : "status-dot-off"}`}
              />
            </div>
            <div className="status-profile">
              {enabledProfile ? enabledProfile.name : "None"}
            </div>
            {isApplying && (
              <div className="status-applying">Applying...</div>
            )}
          </div>
        </div>
      </aside>

      <main className="mhost-main">
        <Outlet />
      </main>
    </div>
  );
}

export default Layout;
