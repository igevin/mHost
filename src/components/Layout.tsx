import { NavLink, Outlet } from "react-router-dom";
import StatusBar from "./StatusBar";
import styles from "./Layout.module.css";

interface NavItem {
  to: string;
  label: string;
  icon: React.ReactNode;
  badge?: number;
  disabled?: boolean;
}

function Layout() {
  const profileNavItems: NavItem[] = [
    { to: "/profiles", label: "Profiles", icon: <ProfilesIcon /> },
  ];

  const toolNavItems: NavItem[] = [
    {
      to: "#adblock",
      label: "Ad Block",
      icon: <ShieldIcon />,
      badge: 10,
      disabled: true,
    },
    {
      to: "#remote",
      label: "Remote Rules",
      icon: <GlobeIcon />,
      badge: 3,
      disabled: true,
    },
    {
      to: "#backup",
      label: "Backup",
      icon: <BackupIcon />,
      disabled: true,
    },
    { to: "/settings", label: "Settings", icon: <SettingsIcon /> },
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

        {/* Profiles Section */}
        <div className={styles.sidebarSection}>
          <div className={styles.sidebarSectionTitle}>Profiles</div>
          <nav className={styles.sidebarNav}>
            {profileNavItems.map((item) => (
              <NavLink
                key={item.to}
                to={item.to}
                className={({ isActive }) =>
                  `${styles.navLink} ${isActive ? styles.navLinkActive : ""}`
                }
                end={item.to === "/profiles"}
              >
                <span className={styles.navIcon}>{item.icon}</span>
                <span>{item.label}</span>
              </NavLink>
            ))}
          </nav>
        </div>

        {/* Tools Section */}
        <div className={styles.sidebarSection}>
          <div className={styles.sidebarSectionTitle}>Tools</div>
          <nav className={styles.sidebarNav}>
            {toolNavItems.map((item) =>
              item.disabled ? (
                <div
                  key={item.to}
                  className={`${styles.navLink} ${styles.navLinkDisabled}`}
                  title="Coming soon"
                >
                  <span className={styles.navIcon}>{item.icon}</span>
                  <span>{item.label}</span>
                  {item.badge !== undefined && item.badge > 0 && (
                    <span className={styles.navBadge}>{item.badge}</span>
                  )}
                  <span className={styles.comingSoonBadge}>Soon</span>
                </div>
              ) : (
                <NavLink
                  key={item.to}
                  to={item.to}
                  className={({ isActive }) =>
                    `${styles.navLink} ${isActive ? styles.navLinkActive : ""}`
                  }
                >
                  <span className={styles.navIcon}>{item.icon}</span>
                  <span>{item.label}</span>
                </NavLink>
              ),
            )}
          </nav>
        </div>

        <div className={styles.sidebarSpacer} />

        <StatusBar />
      </aside>

      <main className={styles.mhostMain}>
        <Outlet />
      </main>
    </div>
  );
}

/* ---- SVG Icons ---- */

function ProfilesIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      width="18"
      height="18"
    >
      <rect x="3" y="3" width="7" height="7" rx="1" />
      <rect x="14" y="3" width="7" height="7" rx="1" />
      <rect x="3" y="14" width="7" height="7" rx="1" />
      <rect x="14" y="14" width="7" height="7" rx="1" />
    </svg>
  );
}

function ShieldIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      width="18"
      height="18"
    >
      <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
    </svg>
  );
}

function GlobeIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      width="18"
      height="18"
    >
      <circle cx="12" cy="12" r="10" />
      <line x1="2" y1="12" x2="22" y2="12" />
      <path d="M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z" />
    </svg>
  );
}

function BackupIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      width="18"
      height="18"
    >
      <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4" />
      <polyline points="7 10 12 15 17 10" />
      <line x1="12" y1="15" x2="12" y2="3" />
    </svg>
  );
}

function SettingsIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      width="18"
      height="18"
    >
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83 0 2 2 0 010-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z" />
    </svg>
  );
}

export default Layout;
