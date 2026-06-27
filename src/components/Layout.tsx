import { useCallback, useState } from "react";
import { NavLink, useNavigate, Outlet } from "react-router-dom";
import { useAtomValue, useSetAtom } from "jotai";
import {
  profilesAtom,
  selectedProfileIdAtom,
  toggleProfileEnabledAtom,
  createProfileAtom,
  isLoadingAtom,
} from "../stores/profiles";
import { extractErrorMessage } from "../lib/error";
import StatusBar from "./StatusBar";
import ManagementDrawer from "./ManagementDrawer";
import styles from "./Layout.module.css";

interface NavItem {
  to: string;
  label: string;
  icon: React.ReactNode;
  badge?: number;
  disabled?: boolean;
}

/* ---- SVG Icons ---- */

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

/* ---- Static tool nav items (module-level constant, created once) ---- */

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

function Layout() {
  const profiles = useAtomValue(profilesAtom);
  const selectedProfileId = useAtomValue(selectedProfileIdAtom);
  const toggleEnabled = useSetAtom(toggleProfileEnabledAtom);
  const createProfile = useSetAtom(createProfileAtom);
  const isLoading = useAtomValue(isLoadingAtom);

  const navigate = useNavigate();
  const [showManagement, setShowManagement] = useState(false);
  const [showCreateDialog, setShowCreateDialog] = useState(false);
  const [newProfileName, setNewProfileName] = useState("");

  const handleProfileClick = useCallback(
    (id: string) => {
      navigate(`/profiles/${id}`);
    },
    [navigate],
  );

  const handleToggle = useCallback(
    (e: React.MouseEvent, id: string) => {
      e.stopPropagation();
      e.preventDefault();
      toggleEnabled(id).catch((err) => {
        console.warn("Failed to toggle profile:", err);
      });
    },
    [toggleEnabled],
  );

  const handleNewProfile = useCallback(() => {
    setNewProfileName("");
    setShowCreateDialog(true);
  }, []);

  const handleCreateProfile = useCallback(async () => {
    const name = newProfileName.trim();
    if (!name) return;
    try {
      const profile = await createProfile(name);
      setShowCreateDialog(false);
      navigate(`/profiles/${profile.id}`);
    } catch (err) {
      console.warn("Failed to create profile:", err);
    }
  }, [newProfileName, createProfile, navigate]);

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
          <div className={styles.sidebarSectionTitleRow}>
            <span className={styles.sidebarSectionTitle}>Profiles</span>
            <button
              className={styles.manageLink}
              onClick={() => setShowManagement(true)}
            >
              Manage -&gt;
            </button>
          </div>
          <div className={styles.profileList}>
            {profiles.length === 0 && (
              <div className={styles.profileEmpty}>No profiles yet</div>
            )}
            {profiles.map((profile) => (
              <div
                key={profile.id}
                className={`${styles.profileItem} ${
                  selectedProfileId === profile.id ? styles.profileItemActive : ""
                }`}
                onClick={() => handleProfileClick(profile.id)}
                role="button"
                tabIndex={0}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleProfileClick(profile.id);
                }}
              >
                <div className={styles.profileItemContent}>
                  <span
                    className={`${styles.profileStatusDot} ${
                      profile.enabled ? styles.profileStatusDotOn : styles.profileStatusDotOff
                    }`}
                  />
                  <div className={styles.profileItemInfo}>
                    <span className={styles.profileItemName}>{profile.name}</span>
                    {profile.description && (
                      <span className={styles.profileItemDesc}>
                        {profile.description}
                      </span>
                    )}
                  </div>
                </div>
                <label
                  className={styles.toggle}
                  onClick={(e) => handleToggle(e, profile.id)}
                >
                  <input
                    type="checkbox"
                    role="switch"
                    checked={profile.enabled}
                    readOnly
                  />
                  <span className={styles.toggleSlider} />
                </label>
              </div>
            ))}
          </div>
          <button
            className={styles.newProfileBtn}
            onClick={handleNewProfile}
          >
            + New Profile
          </button>
          {showCreateDialog && (
            <div className={styles.createDialogOverlay} onClick={() => setShowCreateDialog(false)}>
              <div className={styles.createDialog} onClick={(e) => e.stopPropagation()}>
                <h3 className={styles.createDialogTitle}>Create Profile</h3>
                <div className="form-row">
                  <input
                    className="input"
                    placeholder="Profile name"
                    value={newProfileName}
                    onChange={(e) => setNewProfileName(e.target.value)}
                    onKeyDown={(e) => { if (e.key === "Enter") handleCreateProfile(); }}
                    autoFocus
                  />
                  <button
                    className="btn btn-primary"
                    onClick={handleCreateProfile}
                    disabled={!newProfileName.trim() || isLoading}
                  >
                    Create
                  </button>
                  <button
                    className="btn btn-ghost"
                    onClick={() => setShowCreateDialog(false)}
                  >
                    Cancel
                  </button>
                </div>
              </div>
            </div>
          )}
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

      <ManagementDrawer
        open={showManagement}
        onClose={() => setShowManagement(false)}
      />
    </div>
  );
}

export default Layout;
