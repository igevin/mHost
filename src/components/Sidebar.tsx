import { memo, useCallback, useState } from "react";
import { NavLink, useNavigate } from "react-router-dom";
import { useAtomValue, useSetAtom } from "jotai";
import {
  profilesAtom,
  selectedProfileIdAtom,
  dnsEnabledAtom,
  dnsStatusAtom,
  isLoadingAtom,
  errorAtom,
  createProfileAtom,
  previewApplyAtom,
  applyErrorAtom,
} from "../stores/profiles";
import { extractErrorMessage } from "../lib/error";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import CreateProfileDialog from "./CreateProfileDialog";
import StatusBar from "./StatusBar";
import styles from "./Layout.module.css";

interface NavItem {
  to: string;
  label: string;
  icon: React.ReactNode;
  badge?: number;
  disabled?: boolean;
}

interface SidebarProps {
  /** 工具导航项（模块级常量，引用稳定） */
  toolNavItems: ReadonlyArray<NavItem>;
  /** Layout 提供的回调：打开 ManagementDrawer */
  onOpenManagement: () => void;
}

/**
 * 侧栏组件 —— 自包含 profile 列表 / DNS 状态指示 / 创建对话框。
 *
 * **fix (P-F1, issue #90)**: 之前 sidebar 是 Layout 内联 JSX，订阅 13 个 atoms
 * （含 apply dialog 状态）。apply 操作启动 → applyConfirmOpen / isApplying 变化
 * → Layout 重渲染 → 整个侧栏（每个 ProfileItem + StatusBar）也跟着重渲染。
 *
 * 现在 Sidebar 自订阅需要的 atoms，apply dialog 状态由 Layout 独占订阅，
 * 侧栏对 apply 状态变化**完全不可见**。
 *
 * React.memo + 稳定的 props（toolNavItems 模块级常量 + useCallback'd
 * onOpenManagement）保证 Layout 重渲染时 Sidebar 完全跳过渲染。
 */
function Sidebar({ toolNavItems, onOpenManagement }: SidebarProps) {
  const navigate = useNavigate();
  const profiles = useAtomValue(profilesAtom);
  const selectedProfileId = useAtomValue(selectedProfileIdAtom);
  const dnsEnabled = useAtomValue(dnsEnabledAtom);
  const dnsStatus = useAtomValue(dnsStatusAtom);
  const isLoading = useAtomValue(isLoadingAtom);

  const createProfile = useSetAtom(createProfileAtom);
  const previewApply = useSetAtom(previewApplyAtom);
  const setError = useSetAtom(errorAtom);
  const setApplyError = useSetAtom(applyErrorAtom);
  const { onPointerDown } = useWebKitPointerDown();

  const [showCreateDialog, setShowCreateDialog] = useState(false);

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

  const handleCreateProfile = useCallback(
    async (name: string) => {
      try {
        const profile = await createProfile(name);
        setShowCreateDialog(false);
        navigate(`/profiles/${profile.id}`);
      } catch (err) {
        setError(extractErrorMessage(err));
      }
    },
    [createProfile, navigate, setError],
  );

  const handleDnsIndicatorClick = useCallback(() => {
    navigate("/settings");
  }, [navigate]);

  return (
    <aside className={styles.sidebar}>
      <div className={styles.sidebarHeader}>
        <div className={styles.logo}>
          <span className={styles.logoIcon}>m</span>
          <span className={styles.logoText}>mHost</span>
        </div>
        {dnsEnabled && (
          <button
            className={styles.dnsStatusIndicator}
            onClick={handleDnsIndicatorClick}
            title={`DNS running on port ${dnsStatus?.port ?? "?"}`}
          >
            <span className={styles.dnsStatusDot} />
            <span className={styles.dnsStatusLabel}>DNS</span>
          </button>
        )}
      </div>

      {/* Profiles Section — flex-grows to fill available sidebar height
          (issue #125 review); profileList scrolls internally. */}
      <div className={`${styles.sidebarSection} ${styles.profilesSection}`}>
        <div className={styles.sidebarSectionTitleRow}>
          <span className={styles.sidebarSectionTitle}>Profiles</span>
          <button className={styles.manageLink} onClick={onOpenManagement}>
            Manage -&gt;
          </button>
        </div>
        <div className={styles.profileList}>
          {profiles.length === 0 && (
            <div className={styles.profileEmpty}>No profiles yet</div>
          )}
          {profiles.map((profile) => {
            const isActive = selectedProfileId === profile.id;
            return (
              <div
                key={profile.id}
                className={`${styles.profileItem} ${
                  isActive ? styles.profileItemActive : ""
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
                      profile.enabled
                        ? styles.profileStatusDotOn
                        : styles.profileStatusDotOff
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
            );
          })}
        </div>
        <button className={styles.newProfileBtn} onClick={handleNewProfile}>
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

      {/*
       * sidebarSpacer 推 StatusBar 到 sidebar 底部。
       * sidebar 是 column-direction flex；spacer 拿剩余高度让 StatusBar 贴在最下方。
       */}
      <div className={styles.sidebarSpacer} />
      <StatusBar />
    </aside>
  );
}

export default memo(Sidebar);

export type { NavItem, SidebarProps };