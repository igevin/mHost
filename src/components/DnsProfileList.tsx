import { memo, useCallback, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useAtomValue, useSetAtom } from "jotai";
import { save, confirm } from "@tauri-apps/plugin-dialog";
import {
  dnsProfilesAtom,
  isDnsLoadingAtom,
  dnsErrorAtom,
  dnsEnabledAtom,
  enabledDnsProfilesAtom,
  dnsRuleCountAtom,
  toggleDnsProfileEnabledAtom,
  createDnsProfileAtom,
  deleteDnsProfileAtom,
  fetchDnsProfilesAtom,
} from "../stores/profiles";
import { countRealRules } from "../lib/rules";
import { exportProfileToFile, duplicateProfile } from "../lib/tauri";
import { extractErrorMessage } from "../lib/error";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import type { Profile } from "../types";
import CreateProfileDialog from "./CreateProfileDialog";
import styles from "./DnsProfileList.module.css";
import viewStyles from "../pages/ProfileView.module.css";

/**
 * DNS 模式的列表落地页 —— 取代 v0.3 中 /dns-profiles 无 id 时的
 * auto-redirect-to-first-profile 行为。
 *
 * **issue #67**：DNS 模式允许多个 profile 同时启用（后端
 * `RuleEngine::rebuild` 取并集，first-wins 冲突），所以这里用 inline
 * checkbox 而非单选开关。
 *
 * **subscription isolation (P-F4, issue #90)**: 只订阅 DNS 相关 atoms，
 * 不订阅 hosts atom；与 `DnsProfileView` 同源，DNS profile 切换/编辑不会
 * 触发 hosts 视图重渲染。
 */
function DnsProfileList() {
  const navigate = useNavigate();

  const profiles = useAtomValue(dnsProfilesAtom);
  const enabledProfiles = useAtomValue(enabledDnsProfilesAtom);
  const totalRules = useAtomValue(dnsRuleCountAtom);
  const dnsEnabled = useAtomValue(dnsEnabledAtom);
  const isLoading = useAtomValue(isDnsLoadingAtom);
  const error = useAtomValue(dnsErrorAtom);

  const createProfile = useSetAtom(createDnsProfileAtom);
  const deleteProfile = useSetAtom(deleteDnsProfileAtom);
  const fetchProfiles = useSetAtom(fetchDnsProfilesAtom);
  const toggleEnabled = useSetAtom(toggleDnsProfileEnabledAtom);
  const setError = useSetAtom(dnsErrorAtom);
  const { onPointerDown } = useWebKitPointerDown();

  const [showCreateDialog, setShowCreateDialog] = useState(false);
  const [duplicatingId, setDuplicatingId] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);

  // issue #67：单击 label 会触发 pointerdown + click 两个事件，两个事件
  // 都直接调 handleToggle 会让 setProfileEnabled 跑两次（true → false），
  // 结果 profile 留在 disabled、规则永不生效。用 ref 锁 100ms 防双触发。
  const toggleLockedRef = useRef(false);

  const stats = useMemo(
    () => ({
      totalProfiles: profiles.length,
      enabledCount: enabledProfiles.length,
      totalRules,
    }),
    [profiles.length, enabledProfiles.length, totalRules],
  );

  const handleCreate = useCallback(
    async (name: string) => {
      try {
        const profile = await createProfile(name);
        setShowCreateDialog(false);
        navigate(`/dns-profiles/${profile.id}`);
      } catch (err: unknown) {
        setError(extractErrorMessage(err));
      }
    },
    [createProfile, navigate, setError],
  );

  const handleToggle = useCallback(
    (id: string, enabled: boolean) => {
      if (toggleLockedRef.current) return;
      toggleLockedRef.current = true;
      setTimeout(() => {
        toggleLockedRef.current = false;
      }, 100);
      toggleEnabled({ id, enabled: !enabled });
    },
    [toggleEnabled],
  );

  const handleEdit = useCallback(
    (id: string) => navigate(`/dns-profiles/${id}`),
    [navigate],
  );

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
    [duplicatingId, fetchProfiles, setError],
  );

  const handleExport = useCallback(async (profile: Profile) => {
    try {
      const path = await save({
        defaultPath: `${profile.name}.dns`,
        filters: [{ name: "Hosts", extensions: ["hosts", "txt"] }],
      });
      if (path) {
        await exportProfileToFile(profile.id, "hosts", path);
      }
    } catch (err: unknown) {
      setError(extractErrorMessage(err));
    }
  }, [setError]);

  const handleDelete = useCallback(
    async (id: string) => {
      if (deletingId) return;
      const ok = await confirm("Delete this DNS profile?");
      if (!ok) return;
      setDeletingId(id);
      try {
        await deleteProfile(id);
      } catch (err: unknown) {
        setError(extractErrorMessage(err));
      } finally {
        setDeletingId(null);
      }
    },
    [deletingId, deleteProfile, setError],
  );

  return (
    <div className={viewStyles.viewPage}>
      {error && <div className="alert alert-error">{error}</div>}

      {/* Header */}
      <div className={viewStyles.viewHeader}>
        <div className={viewStyles.viewHeaderLeft}>
          <h1 className={viewStyles.viewTitle}>DNS Profiles</h1>
          <div className={viewStyles.viewBadges}>
            <span className={`${viewStyles.badge} ${viewStyles.badgeModeDns}`}>
              DNS
            </span>
            <span
              className={`${viewStyles.badge} ${
                dnsEnabled
                  ? viewStyles.badgeEnabled
                  : viewStyles.badgeDisabled
              }`}
            >
              {dnsEnabled ? "Mode On" : "Mode Off"}
            </span>
          </div>
        </div>
        <div className={viewStyles.viewHeaderActions}>
          {profiles.length > 0 && (
            <button
              className="btn btn-primary btn-sm"
              onClick={() => setShowCreateDialog(true)}
              onPointerDown={onPointerDown(() => setShowCreateDialog(true))}
            >
              + New DNS Profile
            </button>
          )}
        </div>
      </div>

      {/* Stats Grid */}
      <div className={styles.statsGrid}>
        <div className={styles.statCard}>
          <div className={styles.statLabel}>Total Profiles</div>
          <div className={styles.statValue}>{stats.totalProfiles}</div>
        </div>
        <div className={styles.statCard}>
          <div className={styles.statLabel}>Enabled</div>
          <div className={styles.statValue}>
            {stats.enabledCount}
            <span className={styles.statTotal}>
              /{stats.totalProfiles}
            </span>
          </div>
        </div>
        <div className={styles.statCard}>
          <div className={styles.statLabel}>Active Rules</div>
          <div className={styles.statValue}>{stats.totalRules}</div>
        </div>
      </div>

      {/* Mode-off hint banner */}
      {!dnsEnabled && profiles.length > 0 && (
        <div className={styles.modeBanner}>
          <span className={styles.modeBannerIcon}>i</span>
          <span>
            DNS mode is off. Enabled profiles are saved but not applied to
            system DNS.{" "}
            <a
              className={styles.modeBannerLink}
              onClick={() => navigate("/settings")}
              onPointerDown={onPointerDown(() => navigate("/settings"))}
            >
              Enable in Settings →
            </a>
          </span>
        </div>
      )}

      {/* Profile Cards */}
      <div className={styles.profileList}>
        {profiles.length === 0 ? (
          <div className={styles.emptyState}>
            <p>No DNS profiles yet</p>
            <p className={styles.emptyHint}>
              Create a profile to manage your DNS rules.
            </p>
            <button
              className="btn btn-primary"
              onClick={() => setShowCreateDialog(true)}
              onPointerDown={onPointerDown(() => setShowCreateDialog(true))}
            >
              + New DNS Profile
            </button>
          </div>
        ) : (
          profiles.map((profile) => (
            <DnsProfileListCard
              key={profile.id}
              profile={profile}
              duplicatingId={duplicatingId}
              deletingId={deletingId}
              onEdit={handleEdit}
              onToggle={handleToggle}
              onDuplicate={handleDuplicate}
              onExport={handleExport}
              onDelete={handleDelete}
              onPointerDownToggle={onPointerDown}
              formatDate={formatDate}
            />
          ))
        )}
      </div>

      <CreateProfileDialog
        open={showCreateDialog}
        onClose={() => setShowCreateDialog(false)}
        onCreate={handleCreate}
        isLoading={isLoading}
      />
    </div>
  );
}

/* ---- Card ---- */

interface DnsProfileListCardProps {
  profile: Profile;
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
 * 列表卡片 —— 镜像 `DrawerProfileCard` 的密度，但带 inline enable toggle
 * （checkbox switch）。`React.memo` 包裹：handler 引用稳定时跳过重渲染。
 */
function DnsProfileListCard({
  profile,
  duplicatingId,
  deletingId,
  onEdit,
  onToggle,
  onDuplicate,
  onExport,
  onDelete,
  onPointerDownToggle,
  formatDate,
}: DnsProfileListCardProps) {
  const ruleCount = useMemo(
    () => countRealRules(profile.rules),
    [profile.rules],
  );

  return (
    <div
      className={`${styles.profileCard} ${
        profile.enabled ? styles.profileCardEnabled : styles.profileCardDisabled
      } ${profile.protected ? styles.profileCardProtected : ""}`}
      data-testid="dns-profile-card"
    >
      <div className={styles.profileCardMain}>
        <label
          className={styles.toggle}
          onClick={(e) => {
            e.stopPropagation();
            e.preventDefault();
            onToggle(profile.id, profile.enabled);
          }}
          onPointerDown={onPointerDownToggle(() =>
            onToggle(profile.id, profile.enabled),
          )}
          title={profile.enabled ? "Disable profile" : "Enable profile"}
        >
          <input
            type="checkbox"
            role="switch"
            checked={profile.enabled}
            readOnly
          />
          <span className={styles.toggleSlider} />
        </label>

        <div
          className={styles.profileCardBody}
          onClick={() => onEdit(profile.id)}
          role="button"
          tabIndex={0}
          onKeyDown={(e) => {
            if (e.key === "Enter") onEdit(profile.id);
          }}
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
            <span>
              {ruleCount} rule{ruleCount !== 1 ? "s" : ""}
            </span>
            <span className={styles.metaSep}>|</span>
            <span>Updated {formatDate(profile.updated_at)}</span>
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
        </div>
      </div>

      <div className={styles.profileCardActions}>
        <button
          className="btn btn-ghost btn-sm"
          onClick={() => onEdit(profile.id)}
        >
          Edit
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

const MemoizedDnsProfileListCard = memo(DnsProfileListCard);

/* ---- Helpers ---- */

function formatDate(isoDate: string): string {
  try {
    const date = new Date(isoDate);
    if (isNaN(date.getTime())) return isoDate;
    return date.toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
    });
  } catch {
    return isoDate;
  }
}

export default DnsProfileList;

// Re-export the card so it can be unit-tested in isolation.
export { MemoizedDnsProfileListCard as DnsProfileListCard };
