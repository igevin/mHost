import { memo, useMemo } from "react";
import type { Profile } from "../types";
import { countRealRules } from "../lib/rules";
import styles from "./ManagementDrawer.module.css";

interface DrawerProfileCardProps {
  profile: Profile;
  isApplying: boolean;
  duplicatingId: string | null;
  deletingId: string | null;
  onEdit: (id: string) => void;
  onToggle: (id: string, enabled: boolean) => void;
  onDuplicate: (profile: Profile) => void;
  onExport: (profile: Profile) => void;
  onDelete: (id: string) => void;
  onPointerDownToggle: (handler: () => void) => (e: React.PointerEvent) => void;
  formatDate: (isoDate: string) => string;
}

/**
 * ManagementDrawer 内的 profile 卡片。
 *
 * **fix (P-F3, issue #90)**:
 *   - `React.memo` 包裹：profile / handler 引用未变时跳过重渲染。
 *     之前 ManagementDrawer 渲染 6 个 inline handler（onClick / onPointerDown），
 *     加上未 memoized 的 `countRealRules(profile.rules)`，每次 drawer 重渲染
 *     （例如 apply dialog 状态变化）所有卡片都跑完整 O(N) + 重新创建 handlers。
 *   - `useMemo` 缓存 ruleCount。
 *   - 所有 handler 通过 props 传入（useCallback'd in parent）。
 */
function DrawerProfileCard({
  profile,
  isApplying,
  duplicatingId,
  deletingId,
  onEdit,
  onToggle,
  onDuplicate,
  onExport,
  onDelete,
  onPointerDownToggle,
  formatDate,
}: DrawerProfileCardProps) {
  const ruleCount = useMemo(() => countRealRules(profile.rules), [profile.rules]);

  return (
    <div
      className={`${styles.profileCard} ${
        profile.enabled
          ? styles.profileCardEnabled
          : styles.profileCardDisabled
      } ${profile.protected ? styles.profileCardProtected : ""}`}
    >
      <div className={styles.profileCardHeader}>
        <h3 className={styles.profileName}>{profile.name}</h3>
        <div className={styles.profileBadges}>
          {profile.enabled ? (
            <span className={`${styles.badge} ${styles.badgeEnabled}`}>
              Enabled
            </span>
          ) : (
            <span className={`${styles.badge} ${styles.badgeDisabled}`}>
              Disabled
            </span>
          )}
          {profile.protected && (
            <span className={`${styles.badge} ${styles.badgeProtected}`}>
              Protected
            </span>
          )}
        </div>
      </div>

      {profile.description && (
        <p className={styles.profileDesc}>{profile.description}</p>
      )}

      <div className={styles.profileMeta}>
        <span>{ruleCount} rules</span>
        <span className={styles.metaSep}>|</span>
        <span>{formatDate(profile.updated_at || profile.created_at)}</span>
      </div>

      {profile.tags.length > 0 && (
        <div className={styles.profileTags}>
          {profile.tags.map((tag) => (
            <span key={tag} className="tag">
              {tag}
            </span>
          ))}
        </div>
      )}

      <div className={styles.profileCardActions}>
        <button
          className="btn btn-ghost btn-sm"
          onClick={() => onEdit(profile.id)}
        >
          Edit
        </button>
        <button
          className="btn btn-ghost btn-sm"
          onClick={() => onToggle(profile.id, profile.enabled)}
          onPointerDown={onPointerDownToggle(() =>
            onToggle(profile.id, profile.enabled),
          )}
          disabled={isApplying}
        >
          {profile.enabled ? "Disable" : "Enable"}
        </button>
        <button
          className="btn btn-ghost btn-sm"
          onClick={() => onDuplicate(profile)}
          disabled={duplicatingId === profile.id}
        >
          {duplicatingId === profile.id ? "Duplicating..." : "Duplicate"}
        </button>
        <button
          className="btn btn-ghost btn-sm"
          onClick={() => onExport(profile)}
        >
          Export
        </button>
        <button
          className="btn btn-danger btn-sm"
          onClick={() => onDelete(profile.id)}
          disabled={profile.protected || deletingId === profile.id}
        >
          {deletingId === profile.id ? "Deleting..." : "Delete"}
        </button>
      </div>
    </div>
  );
}

export default memo(DrawerProfileCard);