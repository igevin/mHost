import { useCallback, useState } from "react";
import { NavLink, useNavigate, Outlet } from "react-router-dom";
import { useAtomValue, useSetAtom } from "jotai";
import {
  profilesAtom,
  selectedProfileIdAtom,
  createProfileAtom,
  errorAtom,
  isLoadingAtom,
  applyConfirmOpenAtom,
  applyPlanAtom,
  applyResultAtom,
  applyErrorAtom,
  isApplyingAtom,
  previewApplyAtom,
  executeApplyAtom,
  closeApplyConfirmAtom,
  rollbackHostsActionAtom,
} from "../stores/profiles";
import { extractErrorMessage } from "../lib/error";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import StatusBar from "./StatusBar";
import ManagementDrawer from "./ManagementDrawer";
import CreateProfileDialog from "./CreateProfileDialog";
import ApplyConfirmDialog from "./ApplyConfirmDialog";
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

function FileTextIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      width="18"
      height="18"
    >
      <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
      <polyline points="14 2 14 8 20 8" />
      <line x1="16" y1="13" x2="8" y2="13" />
      <line x1="16" y1="17" x2="8" y2="17" />
      <polyline points="10 9 9 9 8 9" />
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
    to: "/snapshot",
    label: "Snapshots",
    icon: <BackupIcon />,
    disabled: false,
  },
  {
    to: "/hosts",
    label: "System Hosts",
    icon: <FileTextIcon />,
    disabled: false,
  },
  { to: "/settings", label: "Settings", icon: <SettingsIcon /> },
];

function Layout() {
  const profiles = useAtomValue(profilesAtom);
  const selectedProfileId = useAtomValue(selectedProfileIdAtom);
  const createProfile = useSetAtom(createProfileAtom);
  const isLoading = useAtomValue(isLoadingAtom);

  const navigate = useNavigate();
  const setError = useSetAtom(errorAtom);
  const [showManagement, setShowManagement] = useState(false);
  const [showCreateDialog, setShowCreateDialog] = useState(false);

  const applyConfirmOpen = useAtomValue(applyConfirmOpenAtom);
  const applyPlan = useAtomValue(applyPlanAtom);
  const applyResult = useAtomValue(applyResultAtom);
  const applyError = useAtomValue(applyErrorAtom);
  const isApplying = useAtomValue(isApplyingAtom);
  const previewApply = useSetAtom(previewApplyAtom);
  const executeApply = useSetAtom(executeApplyAtom);
  const closeApplyConfirm = useSetAtom(closeApplyConfirmAtom);
  const rollbackHostsAction = useSetAtom(rollbackHostsActionAtom);
  const setApplyError = useSetAtom(applyErrorAtom);
  const { onPointerDown } = useWebKitPointerDown();

  const handleProfileClick = useCallback(
    (id: string) => {
      navigate(`/profiles/${id}`);
    },
    [navigate],
  );

  const handleToggle = useCallback(
    (id: string, enabled: boolean) => {
      setApplyError(null);
      previewApply({ id, enabled: !enabled });
    },
    [previewApply, setApplyError],
  );

  const handleNewProfile = useCallback(() => {
    setShowCreateDialog(true);
  }, []);

  const handleCreateProfile = useCallback(async (name: string) => {
    try {
      const profile = await createProfile(name);
      setShowCreateDialog(false);
      navigate(`/profiles/${profile.id}`);
    } catch (err) {
      setError(extractErrorMessage(err));
    }
  }, [createProfile, navigate, setError]);

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
                  onClick={(e) => {
                    e.stopPropagation();
                    e.preventDefault();
                    handleToggle(profile.id, profile.enabled);
                  }}
                  onPointerDown={onPointerDown(() => {
                    handleToggle(profile.id, profile.enabled);
                  })}
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
          <CreateProfileDialog
            open={showCreateDialog}
            onClose={() => setShowCreateDialog(false)}
            onCreate={handleCreateProfile}
            isLoading={isLoading}
          />
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
        {applyError && !applyConfirmOpen && (
          <div className="alert alert-error">
            {applyError}
            <button
              className="btn btn-ghost btn-sm"
              onClick={() => setApplyError(null)}
              style={{ marginLeft: 8 }}
            >
              Dismiss
            </button>
          </div>
        )}
        <Outlet />
      </main>

      <ManagementDrawer
        open={showManagement}
        onClose={() => setShowManagement(false)}
      />

      <ApplyConfirmDialog
        open={applyConfirmOpen}
        plan={applyPlan}
        onConfirm={() => executeApply()}
        onCancel={() => closeApplyConfirm()}
        isApplying={isApplying}
        applyResult={applyResult}
        applyError={applyError}
        onRollback={() => rollbackHostsAction()}
      />
    </div>
  );
}

export default Layout;
