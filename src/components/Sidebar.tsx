import { memo, useCallback, useMemo, useState } from "react";
import { NavLink, useNavigate } from "react-router-dom";
import { useAtomValue, useSetAtom } from "jotai";
import { save } from "@tauri-apps/plugin-dialog";
import {
  profilesAtom,
  selectedProfileIdAtom,
  dnsEnabledAtom,
  dnsStatusAtom,
  isLoadingAtom,
  errorAtom,
  createProfileAtom,
  deleteProfileAtom,
  fetchProfilesAtom,
  previewApplyAtom,
  applyErrorAtom,
  quickApplyOnToggleAtom,
  quickApplyToggleAtom,
} from "../stores/profiles";
import { exportProfileToFile, duplicateProfile } from "../lib/tauri";
import { extractErrorMessage } from "../lib/error";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import type { Profile } from "../types";
import CreateProfileDialog from "./CreateProfileDialog";
import ConfirmDialog from "./ConfirmDialog";
import PopupMenu, { type PopupMenuItem } from "./PopupMenu";
import SidebarProfileItem from "./SidebarProfileItem";
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
}

/**
 * 侧栏组件 —— 自包含 profile 列表 / DNS 状态指示 / 创建对话框。
 *
 * **Phase 2: 侧栏即管理面。** ManagementDrawer（及其 DrawerProfileCard）
 * 已删除；profile 的搜索过滤 / 排序 / ⋯ 菜单（Edit info / Duplicate /
 * Export / Delete / Enable-Disable）全部收在这里。Delete 走 Phase 1 的
 * ConfirmDialog。
 *
 * **fix (P-F1, issue #90)**: 之前 sidebar 是 Layout 内联 JSX，订阅 13 个 atoms
 * （含 apply dialog 状态）。apply 操作启动 → applyConfirmOpen / isApplying 变化
 * → Layout 重渲染 → 整个侧栏（每个 ProfileItem + StatusBar）也跟着重渲染。
 *
 * 现在 Sidebar 自订阅需要的 atoms，apply dialog 状态由 Layout 独占订阅，
 * 侧栏对 apply 状态变化**完全不可见**。
 *
 * React.memo + 稳定的 props（toolNavItems 模块级常量）保证 Layout
 * 重渲染时 Sidebar 完全跳过渲染。
 */
function Sidebar({ toolNavItems }: SidebarProps) {
  const navigate = useNavigate();
  const profiles = useAtomValue(profilesAtom);
  const selectedProfileId = useAtomValue(selectedProfileIdAtom);
  const dnsEnabled = useAtomValue(dnsEnabledAtom);
  const dnsStatus = useAtomValue(dnsStatusAtom);
  const isLoading = useAtomValue(isLoadingAtom);

  const createProfile = useSetAtom(createProfileAtom);
  const deleteProfile = useSetAtom(deleteProfileAtom);
  const fetchProfiles = useSetAtom(fetchProfilesAtom);
  const previewApply = useSetAtom(previewApplyAtom);
  const quickApplyToggle = useSetAtom(quickApplyToggleAtom);
  // issue #123: when ON, Hosts profile toggles skip the preview dialog
  // and call `enable_and_apply` directly. Cmd/Option always forces Preview.
  const quickApplyEnabled = useAtomValue(quickApplyOnToggleAtom);
  const setError = useSetAtom(errorAtom);
  const setApplyError = useSetAtom(applyErrorAtom);
  const { fire, releaseSoon } = useWebKitPointerDown();

  const [showCreateDialog, setShowCreateDialog] = useState(false);
  // 搜索过滤 —— 纯本地 state，不进 atom（Phase 2 brief: no atom）。
  const [profileQuery, setProfileQuery] = useState("");
  // ⋯ 菜单：当前打开的行 id + 锚点 rect（PopupMenu 按 fixed 定位）。
  const [menuState, setMenuState] = useState<{
    id: string;
    anchorRect: DOMRect;
  } | null>(null);
  // Delete 确认目标（ConfirmDialog，Phase 1）+ 删除中状态。
  const [deleteTarget, setDeleteTarget] = useState<Profile | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [duplicatingId, setDuplicatingId] = useState<string | null>(null);

  // 排序 + 过滤都在 memoized selector 里做，绝不 mutate profilesAtom：
  // 启用的在前，其余按名字字母序（localeCompare）。
  const visibleProfiles = useMemo(() => {
    const q = profileQuery.trim().toLowerCase();
    const filtered = q
      ? profiles.filter((p) => p.name.toLowerCase().includes(q))
      : profiles;
    return [...filtered].sort((a, b) => {
      if (a.enabled !== b.enabled) return a.enabled ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
  }, [profiles, profileQuery]);

  const handleProfileClick = useCallback(
    (id: string) => {
      navigate(`/profiles/${id}`);
    },
    [navigate],
  );

  // issue #123: the Quick Apply preference routes Hosts profile toggles
  // straight to `enable_and_apply`. Both `onClick` and `onPointerDown`
  // share `fire()` so the WebKit duplicate event is dropped — without
  // this guard a Quick Apply would run `enable_and_apply` twice, doing
  // two backups and two writes to /etc/hosts per single user click.
  //
   // We deliberately do **not** use the `useWebKitPointerDown.onPointerDown`
   // wrapper here. The wrapper itself calls `fire()` before dispatching
   // the handler; pairing it with handleToggle's own `fire()` check would
   // double-consume `firedRef` and drop every gesture. Instead both
   // event handlers route through `handleToggle`, which is the single
   // owner of the dedupe state via `fire()` + `releaseSoon()`.
  const handleToggle = useCallback(
    (id: string, enabled: boolean, forcePreview: boolean) => {
      if (!fire()) return;
      releaseSoon();
      setApplyError(null);
      if (quickApplyEnabled && !forcePreview) {
        quickApplyToggle({ id, enabled: !enabled, forcePreview });
      } else {
        previewApply({ id, enabled: !enabled });
      }
    },
    [fire, releaseSoon, previewApply, quickApplyToggle, quickApplyEnabled, setApplyError],
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

  const handleOpenMenu = useCallback((id: string, anchorRect: DOMRect) => {
    setMenuState({ id, anchorRect });
  }, []);

  const handleCloseMenu = useCallback(() => {
    setMenuState(null);
  }, []);

  const handleDuplicate = useCallback(
    async (profile: Profile) => {
      if (duplicatingId) return;
      setDuplicatingId(profile.id);
      try {
        await duplicateProfile(profile.id, `${profile.name} (copy)`);
        await fetchProfiles();
      } catch (err: unknown) {
        setError(extractErrorMessage(err));
      } finally {
        setDuplicatingId(null);
      }
    },
    [duplicatingId, setError, fetchProfiles],
  );

  const handleExport = useCallback(
    async (profile: Profile) => {
      try {
        const path = await save({
          defaultPath: `${profile.name}.hosts`,
          filters: [{ name: "Hosts", extensions: ["hosts", "txt"] }],
        });
        if (path) {
          await exportProfileToFile(profile.id, "hosts", path);
        }
      } catch (err: unknown) {
        setError(extractErrorMessage(err));
      }
    },
    [setError],
  );

  const handleDeleteRequest = useCallback((profile: Profile) => {
    setDeleteError(null);
    setDeleteTarget(profile);
  }, []);

  // review F5: RollbackButton 模式 —— 对话框在整个异步删除期间保持打开
  // （isLoading 锁住 Esc/overlay/Cancel），成功才关闭，失败把错误显示在
  // 框内。之前先关框再 await，isLoading/Esc 锁全是死代码。
  const handleDeleteConfirm = useCallback(async () => {
    const target = deleteTarget;
    if (!target || deletingId !== null) return;
    setDeleteError(null);
    setDeletingId(target.id);
    try {
      await deleteProfile(target.id);
      setDeleteTarget(null);
    } catch (err: unknown) {
      setDeleteError(extractErrorMessage(err));
    } finally {
      setDeletingId(null);
    }
  }, [deleteTarget, deletingId, deleteProfile]);

  const handleDeleteCancel = useCallback(() => {
    if (deletingId !== null) return;
    setDeleteTarget(null);
    setDeleteError(null);
  }, [deletingId]);

  const menuProfile = menuState
    ? (profiles.find((p) => p.id === menuState.id) ?? null)
    : null;

  const menuItems: PopupMenuItem[] = menuProfile
    ? [
        {
          label: menuProfile.enabled ? "Disable" : "Enable",
          onSelect: () => handleToggle(menuProfile.id, menuProfile.enabled, false),
        },
        {
          label: "Edit info",
          onSelect: () => navigate(`/profiles/${menuProfile.id}`),
        },
        {
          label: duplicatingId === menuProfile.id ? "Duplicating..." : "Duplicate",
          disabled: duplicatingId !== null,
          onSelect: () => handleDuplicate(menuProfile),
        },
        {
          label: "Export",
          onSelect: () => handleExport(menuProfile),
        },
        {
          label: "Delete",
          danger: true,
          disabled: menuProfile.protected || deletingId !== null,
          onSelect: () => handleDeleteRequest(menuProfile),
        },
      ]
    : [];

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
          (issue #125 review); profileList scrolls internally, Tools 与
          StatusBar 始终钉在底部（滚动隔离）。 */}
      <div className={`${styles.sidebarSection} ${styles.profilesSection}`}>
        <div className={styles.sidebarSectionTitleRow}>
          <span className={styles.sidebarSectionTitle}>Profiles</span>
        </div>
        <input
          className={styles.profileSearchInput}
          type="text"
          placeholder="Filter profiles..."
          aria-label="Filter profiles"
          value={profileQuery}
          onChange={(e) => setProfileQuery(e.target.value)}
          spellCheck={false}
        />
        <div className={styles.profileList}>
          {profiles.length === 0 && (
            <div className={styles.profileEmpty}>No profiles yet</div>
          )}
          {profiles.length > 0 && visibleProfiles.length === 0 && (
            <div className={styles.profileEmpty}>No matching profiles</div>
          )}
          {visibleProfiles.map((profile) => (
            <SidebarProfileItem
              key={profile.id}
              profile={profile}
              isActive={selectedProfileId === profile.id}
              onOpen={handleProfileClick}
              onToggle={handleToggle}
              onOpenMenu={handleOpenMenu}
            />
          ))}
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

      {menuState && menuProfile && (
        <PopupMenu
          anchorRect={menuState.anchorRect}
          items={menuItems}
          onClose={handleCloseMenu}
        />
      )}

      <ConfirmDialog
        open={deleteTarget !== null}
        title="Delete Profile"
        message={
          deleteTarget ? `Delete profile "${deleteTarget.name}"?` : ""
        }
        confirmLabel="Delete"
        danger
        isLoading={deletingId !== null}
        loadingLabel="Deleting..."
        onConfirm={handleDeleteConfirm}
        onCancel={handleDeleteCancel}
      >
        {deleteError && <div className="alert alert-error">{deleteError}</div>}
      </ConfirmDialog>
    </aside>
  );
}

export default memo(Sidebar);

export type { NavItem, SidebarProps };
