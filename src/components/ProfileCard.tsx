import { memo, useMemo } from "react";
import type { Profile, ExportFormat } from "../types";
import { countRealRules } from "../lib/rules";
import styles from "../pages/ProfileList.module.css";

interface ProfileCardProps {
  profile: Profile;
  isLoading: boolean;
  onEdit: (id: string) => void;
  onToggle: (id: string) => void;
  onDelete: (id: string) => void;
  onExport?: (id: string, format: ExportFormat) => void;
  onDuplicate?: (id: string) => void;
}

/**
 * Profile 卡片 —— 展示单个 profile 的基本信息 + 操作按钮。
 *
 * **fix (P-F2, issue #90)**:
 *   - `React.memo` 包裹：profile / handler 引用未变时跳过重渲染。
 *     父组件 ProfileList 在 apply 状态变化时不应让所有卡片重渲染。
 *   - `useMemo` 包 `countRealRules(profile.rules)`：每个卡片的 rule
 *     计数只在该 profile 的 rules 变化时重算（不再每次渲染都跑 O(N)）。
 *   - 内部 `onClick` 用箭头函数不可避免（依赖 `profile.id`），但因 React.memo
 *     + 父组件 useCallback'd handlers 后整体开销可控。
 */
function ProfileCard({
  profile,
  isLoading,
  onEdit,
  onToggle,
  onDelete,
  onExport,
  onDuplicate,
}: ProfileCardProps) {
  const ruleCount = useMemo(() => countRealRules(profile.rules), [profile.rules]);

  return (
    <div
      className={`${styles.profileCard} ${profile.enabled ? styles.profileCardEnabled : ""}`}
    >
      <div className={styles.profileCardMain}>
        <div className={styles.profileCardHeader}>
          <h3
            className={styles.profileName}
            onClick={() => onEdit(profile.id)}
          >
            {profile.name}
          </h3>
          <div className={styles.profileCardTags}>
            {profile.tags.map((tag) => (
              <span key={tag} className="tag">
                {tag}
              </span>
            ))}
          </div>
        </div>
        {profile.description && (
          <p className={styles.profileDesc}>{profile.description}</p>
        )}
        <div className={styles.profileCardMeta}>
          <span>{ruleCount} rules</span>
          <span className={styles.metaSep}>·</span>
          <span>{profile.enabled ? "Enabled" : "Disabled"}</span>
          {profile.protected && (
            <>
              <span className={styles.metaSep}>·</span>
              <span className={styles.protectedBadge}>Protected</span>
            </>
          )}
        </div>
      </div>

      <div className={styles.profileCardActions}>
        <label className="toggle">
          <input
            type="checkbox"
            checked={profile.enabled}
            onChange={() => onToggle(profile.id)}
            disabled={isLoading}
          />
          <span className="toggle-slider" />
        </label>
        <button
          className="btn btn-ghost btn-sm"
          onClick={() => onEdit(profile.id)}
        >
          Edit
        </button>
        {onExport && (
          <button
            className="btn btn-ghost btn-sm"
            onClick={() => onExport(profile.id, "hosts")}
            disabled={isLoading}
          >
            Export
          </button>
        )}
        {onDuplicate && (
          <button
            className="btn btn-ghost btn-sm"
            onClick={() => onDuplicate(profile.id)}
            disabled={isLoading}
          >
            Duplicate
          </button>
        )}
        <button
          className="btn btn-danger btn-sm"
          onClick={() => onDelete(profile.id)}
          disabled={profile.protected || isLoading}
        >
          Delete
        </button>
      </div>
    </div>
  );
}

export default memo(ProfileCard);