import { NavLink, Outlet } from "react-router-dom";
import { useAtomValue } from "jotai";
import { enabledProfileAtom, isApplyingAtom } from "../stores/profiles";
import styles from "./Layout.module.css";

function Layout() {
  const enabledProfile = useAtomValue(enabledProfileAtom);
  const isApplying = useAtomValue(isApplyingAtom);

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

        <div className={styles.sidebarFooter}>
          <div className={styles.statusCard}>
            <div className={styles.statusRow}>
              <span className={styles.statusLabel}>Active</span>
              <span
                className={`${styles.statusDot} ${enabledProfile ? styles.statusDotOn : styles.statusDotOff}`}
              />
            </div>
            <div className={styles.statusProfile}>
              {enabledProfile ? enabledProfile.name : "None"}
            </div>
            {isApplying && (
              <div className={styles.statusApplying}>Applying...</div>
            )}
          </div>
        </div>
      </aside>

      <main className={styles.mhostMain}>
        <Outlet />
      </main>
    </div>
  );
}

export default Layout;
