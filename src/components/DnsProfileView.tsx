import { useState, useCallback, useMemo } from "react";
import { useParams, useNavigate, Navigate } from "react-router-dom";
import { useAtomValue, useSetAtom } from "jotai";
import { save } from "@tauri-apps/plugin-dialog";
import {
  dnsProfilesAtom,
  isDnsLoadingAtom,
  dnsErrorAtom,
  updateDnsProfileAtom,
  createDnsProfileAtom,
  deleteDnsProfileAtom,
  toggleDnsProfileEnabledAtom,
} from "../stores/profiles";
import { countRealRules } from "../lib/rules";
import { exportProfileToFile } from "../lib/tauri";
import { extractErrorMessage } from "../lib/error";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import CreateProfileDialog from "./CreateProfileDialog";
import styles from "../pages/ProfileView.module.css";

/**
 * DNS 模式的 ProfileView —— 只订阅 DNS 相关 atoms（dnsProfilesAtom、
 * isDnsLoadingAtom、dnsErrorAtom 等），不订阅任何 hosts atom。
 *
 * **fix (P-F4, issue #90)**: 与 HostsProfileView 互补。DNS 模式下：
 *   - DNS profile 切换/编辑/toggle 不再触发 hosts atom 订阅者的重渲染
 *   - hosts 模式（ProfileList 等）的 atom 变化对 DNS ProfileView 不可见
 *
 * DNS 模式与 Hosts 模式的关键差异：
 *   - **toggle**：DNS 直接生效（toggleDnsProfileEnabledAtom），不走 apply preview
 *     （hosts 模式因涉及 /etc/hosts 写入需要用户确认）
 *   - **export** 后缀固定为 `.dns`
 *   - **create** 后导航到 `/dns-profiles/{id}`
 *   - 不导入 rules / 不编辑 rules（DNS profile 在 v0.2 主要是规则集合，
 *     编辑走独立 dialog —— 这里保持原 ProfileView 行为）
 */
export default function DnsProfileView() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();

  const profiles = useAtomValue(dnsProfilesAtom);
  const isLoading = useAtomValue(isDnsLoadingAtom);
  const error = useAtomValue(dnsErrorAtom);

  const updateProfile = useSetAtom(updateDnsProfileAtom);
  const createProfileAction = useSetAtom(createDnsProfileAtom);
  const deleteProfileAction = useSetAtom(deleteDnsProfileAtom);
  const toggleEnabled = useSetAtom(toggleDnsProfileEnabledAtom);
  const { onPointerDown } = useWebKitPointerDown();

  const profile = profiles.find((p) => p.id === id);

  const [isInfoBarExpanded, setIsInfoBarExpanded] = useState(false);
  const [isEditingInfo, setIsEditingInfo] = useState(false);
  const [isSavingInfo, setIsSavingInfo] = useState(false);
  const [showCreateDialog, setShowCreateDialog] = useState(false);

  const [draftInfo, setDraftInfo] = useState<{
    name: string;
    description: string;
    tags: string;
  }>({
    name: "",
    description: "",
    tags: "",
  });
  const [infoHasChanges, setInfoHasChanges] = useState(false);

  const ruleCount = useMemo(
    () => countRealRules(profile?.rules ?? []),
    [profile?.rules],
  );

  const handleEditInfo = useCallback(() => {
    if (profile) {
      setDraftInfo({
        name: profile.name,
        description: profile.description ?? "",
        tags: profile.tags.join(", "),
      });
      setInfoHasChanges(false);
      setIsEditingInfo(true);
      setIsInfoBarExpanded(true);
    }
  }, [profile]);

  const handleCancelInfoEdit = useCallback(() => {
    setIsEditingInfo(false);
    setInfoHasChanges(false);
  }, []);

  const handleInfoChange = useCallback(
    (field: string, value: string) => {
      setDraftInfo((prev) => ({ ...prev, [field]: value }));
      setInfoHasChanges(true);
    },
    [],
  );

  const handleSaveInfo = useCallback(async () => {
    if (!profile || !infoHasChanges || isSavingInfo) return;
    setIsSavingInfo(true);
    try {
      const tags = draftInfo.tags
        .split(",")
        .map((t) => t.trim())
        .filter((t) => t.length > 0);
      const updated = {
        ...profile,
        name: draftInfo.name.trim() || profile.name,
        description: draftInfo.description.trim() || null,
        tags,
      };
      await updateProfile(updated);
      setInfoHasChanges(false);
      setIsEditingInfo(false);
      setIsInfoBarExpanded(false);
    } catch (err: unknown) {
      setError(extractErrorMessage(err));
    } finally {
      setIsSavingInfo(false);
    }
  }, [profile, draftInfo, infoHasChanges, isSavingInfo, updateProfile]);

  const handleExport = useCallback(async () => {
    if (!profile || !id) return;
    try {
      const path = await save({
        defaultPath: `${profile.name}.dns`,
        filters: [{ name: "Hosts", extensions: ["hosts", "txt"] }],
      });
      if (path) {
        await exportProfileToFile(id, "hosts", path);
      }
    } catch (err: unknown) {
      setError(extractErrorMessage(err));
    }
  }, [profile, id]);

  const handleDeleteProfile = useCallback(async () => {
    if (!profile || !id || profile.protected) return;
    const confirmed = window.confirm(`Delete profile "${profile.name}"?`);
    if (!confirmed) return;
    try {
      await deleteProfileAction(id);
      navigate("/dns-profiles");
    } catch (err: unknown) {
      setError(extractErrorMessage(err));
    }
  }, [profile, id, deleteProfileAction, navigate]);

  const handleCreateProfile = useCallback(
    async (name: string) => {
      try {
        const profile = await createProfileAction(name);
        setShowCreateDialog(false);
        navigate(`/dns-profiles/${profile.id}`);
      } catch (err: unknown) {
        setError(extractErrorMessage(err));
      }
    },
    [createProfileAction, navigate],
  );

  // **fix (P-F4)**: DNS 模式 toggle 直接生效（不走 apply preview），因为
  // DNS profile 不涉及 /etc/hosts 写入。
  const handleToggleEnabled = useCallback(() => {
    if (!id || !profile) return;
    toggleEnabled({ id, enabled: !profile.enabled });
  }, [id, profile, toggleEnabled]);

  // 帮助函数：error atom setter 不直接暴露，从 useSetAtom 取
  const setError = useSetAtom(dnsErrorAtom);

  if (!id) {
    if (profiles.length > 0) {
      return (
        <Navigate to={`/dns-profiles/${profiles[0].id}`} replace />
      );
    }
    return (
      <div className={styles.viewPage}>
        <div className="empty-state">
          <p>No DNS profiles yet</p>
          <button
            className="btn btn-primary"
            onClick={() => setShowCreateDialog(true)}
          >
            + New DNS Profile
          </button>
        </div>
        <CreateProfileDialog
          open={showCreateDialog}
          onClose={() => setShowCreateDialog(false)}
          onCreate={handleCreateProfile}
          isLoading={isLoading}
        />
      </div>
    );
  }

  if (!profile) {
    return (
      <div className={styles.viewPage}>
        <div className="empty-state">
          <p>Profile not found.</p>
          <button
            className="btn btn-primary"
            onClick={() => navigate("/dns-profiles")}
          >
            Back to DNS Profiles
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className={styles.viewPage}>
      {error && <div className="alert alert-error">{error}</div>}

      {/* Info Bar */}
      <div className={styles.infoBar}>
        {!isInfoBarExpanded ? (
          <div
            className={styles.infoBarCollapsed}
            onClick={() => setIsInfoBarExpanded(true)}
          >
            <div className={styles.infoBarSummary}>
              <span className={styles.infoBarName}>{profile.name}</span>
              {profile.description && (
                <span className={styles.infoBarDesc}>
                  {profile.description}
                </span>
              )}
              {profile.tags.length > 0 && (
                <div className={styles.infoBarTags}>
                  {profile.tags.map((tag) => (
                    <span key={tag} className={styles.infoBarTag}>
                      {tag}
                    </span>
                  ))}
                </div>
              )}
            </div>
            <button
              className={styles.infoBarEditLink}
              onClick={(e) => {
                e.stopPropagation();
                handleEditInfo();
              }}
            >
              Edit info -&gt;
            </button>
          </div>
        ) : (
          <div className={styles.infoBarExpanded}>
            {isEditingInfo ? (
              <>
                <div className={styles.infoBarFields}>
                  <div className="form-group">
                    <label className="form-label">Name</label>
                    <input
                      className="input"
                      value={draftInfo.name}
                      onChange={(e) => handleInfoChange("name", e.target.value)}
                    />
                  </div>
                  <div className="form-group">
                    <label className="form-label">Description</label>
                    <input
                      className="input"
                      value={draftInfo.description}
                      onChange={(e) =>
                        handleInfoChange("description", e.target.value)
                      }
                      placeholder="Optional description"
                    />
                  </div>
                  <div className="form-group">
                    <label className="form-label">
                      Tags (comma-separated)
                    </label>
                    <input
                      className="input"
                      value={draftInfo.tags}
                      onChange={(e) => handleInfoChange("tags", e.target.value)}
                      placeholder="e.g. ads, trackers"
                    />
                  </div>
                </div>
                <div className={styles.infoBarActions}>
                  <button
                    className="btn btn-primary btn-sm"
                    onClick={handleSaveInfo}
                    onPointerDown={onPointerDown(handleSaveInfo)}
                    disabled={!infoHasChanges || isLoading || isSavingInfo}
                  >
                    {isSavingInfo ? "Saving..." : "Save"}
                  </button>
                  <button
                    className="btn btn-ghost btn-sm"
                    onClick={handleCancelInfoEdit}
                    onPointerDown={onPointerDown(handleCancelInfoEdit)}
                  >
                    Cancel
                  </button>
                </div>
              </>
            ) : (
              <>
                <div className={styles.infoBarFields}>
                  <div className="form-group">
                    <label className="form-label">Name</label>
                    <input className="input" value={profile.name} readOnly />
                  </div>
                  <div className="form-group">
                    <label className="form-label">Description</label>
                    <input
                      className="input"
                      value={profile.description ?? ""}
                      readOnly
                    />
                  </div>
                  <div className="form-group">
                    <label className="form-label">Tags</label>
                    <input
                      className="input"
                      value={profile.tags.join(", ")}
                      readOnly
                    />
                  </div>
                </div>
                <div className={styles.infoBarActions}>
                  <button
                    className="btn btn-ghost btn-sm"
                    onClick={() => {
                      setIsEditingInfo(false);
                      setIsInfoBarExpanded(false);
                    }}
                  >
                    Collapse
                  </button>
                </div>
              </>
            )}
          </div>
        )}
      </div>

      {/* Header */}
      <div className={styles.viewHeader}>
        <div className={styles.viewHeaderLeft}>
          <h1 className={styles.viewTitle}>{profile.name}</h1>
          <div className={styles.viewBadges}>
            <span
              className={`${styles.badge} ${styles.badgeModeDns}`}
            >
              DNS
            </span>
            {profile.enabled ? (
              <span className={`${styles.badge} ${styles.badgeEnabled}`}>
                Enabled
              </span>
            ) : (
              <span className={`${styles.badge} ${styles.badgeDisabled}`}>
                Disabled
              </span>
            )}
            <button
              className={`btn btn-sm ${profile.enabled ? "btn-ghost" : "btn-primary"}`}
              onClick={handleToggleEnabled}
              onPointerDown={onPointerDown(handleToggleEnabled)}
              disabled={isLoading}
            >
              {profile.enabled ? "Disable" : "Enable"}
            </button>
            {profile.protected && (
              <span className={`${styles.badge} ${styles.badgeProtected}`}>
                Protected
              </span>
            )}
            <span className={`${styles.badge} ${styles.badgeRules}`}>
              {ruleCount} rule{ruleCount !== 1 ? "s" : ""}
            </span>
          </div>
        </div>
        <div className={styles.viewHeaderActions}>
          <button
            className="btn btn-ghost btn-sm"
            onClick={handleExport}
            onPointerDown={onPointerDown(handleExport)}
            disabled={isLoading}
          >
            Export
          </button>
          <button
            className="btn btn-primary btn-sm"
            onClick={handleEditInfo}
            onPointerDown={onPointerDown(handleEditInfo)}
          >
            Edit Info
          </button>
          <button
            className="btn btn-danger btn-sm"
            onClick={handleDeleteProfile}
            onPointerDown={onPointerDown(handleDeleteProfile)}
            disabled={profile.protected || isLoading}
          >
            Delete
          </button>
        </div>
      </div>

      <CreateProfileDialog
        open={showCreateDialog}
        onClose={() => setShowCreateDialog(false)}
        onCreate={handleCreateProfile}
        isLoading={isLoading}
      />
    </div>
  );
}