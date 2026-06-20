import { NavLink, Outlet } from "react-router-dom";
import StatusBar from "./StatusBar";
import styles from "./Layout.module.css";

function Layout() {
  const navItems = [
    { to: "/profiles", label: "Profiles", icon: "Profiles" },
    { to: "/settings", label: "Settings", icon: "Settings" },
  ];

  return (
    <div className={styles.mhostLayout}>
      <aside className={styles.sidebar}>
        <div className={styles.sidebarHeader}>
          <div className={styles.logo}>
            <span className={styles.logoIcon}>m</span>
            <span className={styles.logoText}>mHost</span>
          </div>
        </div>

        <nav className={styles.sidebarNav}>
          {navItems.map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              className={({ isActive }) =>
                `${styles.navLink} ${isActive ? styles.navLinkActive : ""}`
              }
              end={item.to === "/profiles"}
            >
              <span className={styles.navIcon}>{item.icon[0]}</span>
              <span>{item.label}</span>
            </NavLink>
          ))}
        </nav>

        <StatusBar />
      </aside>

      <main className={styles.mhostMain}>
        <Outlet />
      </main>
    </div>
  );
}

export default Layout;
