import { memo } from "react";
import type { Profile } from "../types";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import styles from "./Layout.module.css";

interface SidebarProfileItemProps {
  profile: Profile;
  isActive: boolean;
  onOpen: (id: string) => void;
  /** issue #123: `forcePreview=true` skips Quick Apply for this click. */
  onToggle: (id: string, enabled: boolean, forcePreview: boolean) => void;
  onOpenMenu: (id: string, anchorRect: DOMRect) => void;
}

/**
 * 侧栏 profile 行（Phase 2：侧栏即管理面，取代已删除的 ManagementDrawer
 * 卡片行）。`React.memo` 包裹 —— profile / handler 引用未变时跳过重渲染
 * （P-F3, issue #90 的同款纪律；所有 handler 由 Sidebar 以 useCallback
 * 传入）。
 *
 * Toggle 的 pointerdown+click 去重由 `onToggle`（Sidebar 的
 * handleToggle）里的 `useWebKitPointerDown` guard 负责（pattern B，
 * 见 hook 文档）：两个事件都直接喂 onToggle，pointerdown 侧只做内联
 * `e.button !== 0` 过滤。
 */
function SidebarProfileItem({
  profile,
  isActive,
  onOpen,
  onToggle,
  onOpenMenu,
}: SidebarProfileItemProps) {
  // ⋯ 按钮是「输入框（搜索框）→ 按钮」的焦点转移场景，正是
  // useWebKitPointerDown 要兜底的 WebKit 吞点击 case（pattern A：
  // 开菜单是幂等 setState，handler 自己不持锁）。
  const { onPointerDown } = useWebKitPointerDown();

  return (
    <div
      className={`${styles.profileItem} ${isActive ? styles.profileItemActive : ""}`}
      onClick={() => onOpen(profile.id)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter") onOpen(profile.id);
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
            <span className={styles.profileItemDesc}>{profile.description}</span>
          )}
        </div>
      </div>
      <button
        className={styles.profileMenuBtn}
        aria-label={`Profile actions for ${profile.name}`}
        title="Profile actions"
        onClick={(e) => {
          e.stopPropagation();
          onOpenMenu(profile.id, e.currentTarget.getBoundingClientRect());
        }}
        onPointerDown={onPointerDown((e) => {
          e.stopPropagation();
          onOpenMenu(profile.id, e.currentTarget.getBoundingClientRect());
        })}
      >
        ⋯
      </button>
      <label
        className={styles.toggle}
        onClick={(e) => {
          e.stopPropagation();
          e.preventDefault();
          onToggle(profile.id, profile.enabled, e.metaKey || e.altKey);
        }}
        onPointerDown={(e) => {
          if (e.button !== 0) return;
          onToggle(profile.id, profile.enabled, e.metaKey || e.altKey);
        }}
      >
        <input type="checkbox" role="switch" checked={profile.enabled} readOnly />
        <span className={styles.toggleSlider} />
      </label>
    </div>
  );
}

export default memo(SidebarProfileItem);
