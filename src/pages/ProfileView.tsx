import { useEffect, useState, useCallback, useMemo, useRef } from "react";
import { useParams, useNavigate, Navigate } from "react-router-dom";
import { useAtomValue, useSetAtom } from "jotai";
import { save, confirm } from "@tauri-apps/plugin-dialog";
import {
  profilesAtom,
  selectedProfileIdAtom,
  isLoadingAtom,
  errorAtom,
  isApplyingAtom,
  applyErrorAtom,
  updateProfileAtom,
  createProfileAtom,
  deleteProfileAtom,
  fetchProfilesAtom,
  previewApplyAtom,
} from "../stores/profiles";
import { countRealRules } from "../lib/rules";
import { exportProfileToFile, deleteProfile } from "../lib/tauri";
import { extractErrorMessage } from "../lib/error";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import type { HostRule } from "../types";
import RuleEditor from "../components/RuleEditor";
import ImportDialog from "../components/ImportDialog";
import CreateProfileDialog from "../components/CreateProfileDialog";
import styles from "./ProfileView.module.css";

function ProfileView() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const profiles = useAtomValue(profilesAtom);
  const isLoading = useAtomValue(isLoadingAtom);
  const error = useAtomValue(errorAtom);
  const isApplying = useAtomValue(isApplyingAtom);
  const setSelectedId = useSetAtom(selectedProfileIdAtom);
  const setError = useSetAtom(errorAtom);
  const updateProfile = useSetAtom(updateProfileAtom);
  const deleteProfileAction = useSetAtom(deleteProfileAtom);
  const createProfile = useSetAtom(createProfileAtom);
  const fetchProfiles = useSetAtom(fetchProfilesAtom);
  const previewApply = useSetAtom(previewApplyAtom);
  const setApplyError = useSetAtom(applyErrorAtom);
  const { onPointerDown } = useWebKitPointerDown();

  const profile = profiles.find((p) => p.id === id);

  // View state
  const [isEditing, setIsEditing] = useState(false);
  const [isInfoBarExpanded, setIsInfoBarExpanded] = useState(false);
  const [isEditingInfo, setIsEditingInfo] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [isSavingInfo, setIsSavingInfo] = useState(false);
  const [importDialogOpen, setImportDialogOpen] = useState(false);
  const [showCreateDialog, setShowCreateDialog] = useState(false);

  // Draft state for editing rules
  const [draftRules, setDraftRules] = useState<HostRule[]>([]);
  const [hasChanges, setHasChanges] = useState(false);
  const [ruleErrors, setRuleErrors] = useState(false);

  // Draft state for editing profile info (name, description, tags)
  const [draftInfo, setDraftInfo] = useState<{ name: string; description: string; tags: string }>({
    name: "",
    description: "",
    tags: "",
  });
  const [infoHasChanges, setInfoHasChanges] = useState(false);

  useEffect(() => {
    if (id) {
      setSelectedId(id);
    }
  }, [id, setSelectedId]);

  // Track whether we are currently editing to reset draft when profile changes
  const isEditingRef = useRef(false);

  // Reset draft when profile changes
  useEffect(() => {
    if (profile && !isEditingRef.current) {
      setDraftRules([...profile.rules]);
      setHasChanges(false);
      setRuleErrors(false);
    } else if (profile && isEditingRef.current) {
      // Profile changed while editing — reset to new profile's rules
      setDraftRules([...profile.rules]);
      setHasChanges(false);
      setRuleErrors(false);
      setIsEditing(false);
      isEditingRef.current = false;
    }
  }, [profile?.id]);

  // Clear error on unmount
  useEffect(() => {
    return () => { setError(null); };
  }, [setError]);

  const ruleCount = useMemo(() => countRealRules(profile?.rules ?? []), [profile?.rules]);

  const handleEditRules = useCallback(() => {
    if (profile) {
      setDraftRules([...profile.rules]);
      setHasChanges(false);
      setRuleErrors(false);
      setIsEditing(true);
      isEditingRef.current = true;
    }
  }, [profile]);

  const handleCancelEdit = useCallback(() => {
    setDraftRules([]);
    setHasChanges(false);
    setRuleErrors(false);
    setIsEditing(false);
    isEditingRef.current = false;
  }, []);

  const handleRulesChange = useCallback((rules: HostRule[]) => {
    setDraftRules(rules);
    setHasChanges(true);
    setRuleErrors(false);
  }, []);

  const handleRulesErrorChange = useCallback((hasErrors: boolean) => {
    setRuleErrors(hasErrors);
  }, []);

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

  const handleInfoChange = useCallback((field: string, value: string) => {
    setDraftInfo((prev) => ({ ...prev, [field]: value }));
    setInfoHasChanges(true);
  }, []);

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
  }, [profile, draftInfo, infoHasChanges, isSavingInfo, updateProfile, setError]);

  const handleSave = useCallback(async () => {
    if (!profile || ruleErrors || isSaving) return;
    setIsSaving(true);
    try {
      const updated = { ...profile, rules: draftRules };
      await updateProfile(updated);
      setHasChanges(false);
      setIsEditing(false);
      isEditingRef.current = false;
    } catch (err: unknown) {
      setError(extractErrorMessage(err));
    } finally {
      setIsSaving(false);
    }
  }, [profile, draftRules, ruleErrors, isSaving, updateProfile, setError]);

  const handleDeleteProfile = useCallback(async () => {
    if (!profile || !id || profile.protected) return;
    const confirmed = await confirm(`Delete profile "${profile.name}"?`);
    if (!confirmed) return;
    try {
      await deleteProfileAction(id);
      navigate("/profiles");
    } catch (err: unknown) {
      setError(extractErrorMessage(err));
    }
  }, [profile, id, deleteProfileAction, navigate, setError]);

  const handleExport = useCallback(async () => {
    if (!profile || !id) return;
    try {
      const path = await save({
        defaultPath: `${profile.name}.hosts`,
        filters: [{ name: "Hosts", extensions: ["hosts", "txt"] }],
      });
      if (path) {
        await exportProfileToFile(id, "hosts", path);
      }
    } catch (err: unknown) {
      setError(extractErrorMessage(err));
    }
  }, [profile, id, setError]);

  const handleRulesParsed = useCallback(async (rules: HostRule[], tempProfileId?: string) => {
    setImportDialogOpen(false);
    if (!profile) {
      setError("No profile selected. Cannot import rules.");
      return;
    }
    try {
      // Update current profile with imported rules
      const updated = { ...profile, rules };
      await updateProfile(updated);
      // Clean up temporary profile if file import was used
      if (tempProfileId) {
        await deleteProfile(tempProfileId);
      }
      await fetchProfiles();
    } catch (err: unknown) {
      setError(extractErrorMessage(err));
    }
  }, [profile, updateProfile, setError, fetchProfiles]);

  const handleCreateProfile = useCallback(async (name: string) => {
    try {
      const profile = await createProfile(name);
      setShowCreateDialog(false);
      navigate(`/profiles/${profile.id}`);
    } catch (err: unknown) {
      setError(extractErrorMessage(err));
    }
  }, [createProfile, navigate, setError]);

  if (!id) {
    // No profile selected - redirect to first profile or show empty state
    if (profiles.length > 0) {
      return <Navigate to={`/profiles/${profiles[0].id}`} replace />;
    }
    return (
      <div className={styles.viewPage}>
        <div className="empty-state">
          <p>No profiles yet</p>
          <button className="btn btn-primary" onClick={() => setShowCreateDialog(true)}>
            + New Profile
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
          <button className="btn btn-primary" onClick={() => navigate("/profiles")}>
            Back to Profiles
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
          <div className={styles.infoBarCollapsed} onClick={() => setIsInfoBarExpanded(true)}>
            <div className={styles.infoBarSummary}>
              <span className={styles.infoBarName}>{profile.name}</span>
              {profile.description && (
                <span className={styles.infoBarDesc}>{profile.description}</span>
              )}
              {profile.tags.length > 0 && (
                <div className={styles.infoBarTags}>
                  {profile.tags.map((tag) => (
                    <span key={tag} className={styles.infoBarTag}>{tag}</span>
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
                      onChange={(e) => handleInfoChange("description", e.target.value)}
                      placeholder="Optional description"
                    />
                  </div>
                  <div className="form-group">
                    <label className="form-label">Tags (comma-separated)</label>
                    <input
                      className="input"
                      value={draftInfo.tags}
                      onChange={(e) => handleInfoChange("tags", e.target.value)}
                      placeholder="e.g. dev, staging"
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
                    <input
                      className="input"
                      value={profile.name}
                      readOnly
                    />
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
            {profile.enabled ? (
              <span className={`${styles.badge} ${styles.badgeEnabled}`}>Enabled</span>
            ) : (
              <span className={`${styles.badge} ${styles.badgeDisabled}`}>Disabled</span>
            )}
            <button
              className={`btn btn-sm ${profile.enabled ? "btn-ghost" : "btn-primary"}`}
              onClick={() => {
                if (id) {
                  setApplyError(null);
                  previewApply({ id, enabled: !profile.enabled });
                }
              }}
              onPointerDown={onPointerDown(() => {
                if (id) {
                  setApplyError(null);
                  previewApply({ id, enabled: !profile.enabled });
                }
              })}
              disabled={isApplying || isLoading}
            >
              {profile.enabled ? "Disable" : "Enable"}
            </button>
            {profile.protected && (
              <span className={`${styles.badge} ${styles.badgeProtected}`}>Protected</span>
            )}
            <span className={`${styles.badge} ${styles.badgeRules}`}>
              {ruleCount} rule{ruleCount !== 1 ? "s" : ""}
            </span>
          </div>
        </div>
        <div className={styles.viewHeaderActions}>
          {!isEditing ? (
            <>
              <button className="btn btn-ghost btn-sm" onClick={() => setImportDialogOpen(true)}>
                Import
              </button>
              <button className="btn btn-ghost btn-sm" onClick={handleExport} disabled={isLoading}>
                Export
              </button>
              <button className="btn btn-primary btn-sm" onClick={handleEditRules}>
                Edit Rules
              </button>
              <button
                className="btn btn-danger btn-sm"
                onClick={handleDeleteProfile}
                disabled={profile.protected || isLoading}
              >
                Delete
              </button>
            </>
          ) : (
            <>
              <button
                className="btn btn-ghost btn-sm"
                onClick={handleCancelEdit}
                onPointerDown={onPointerDown(handleCancelEdit)}
              >
                Cancel
              </button>
              <button
                className="btn btn-primary btn-sm"
                onClick={handleSave}
                onPointerDown={onPointerDown(handleSave)}
                disabled={!hasChanges || isLoading || ruleErrors || isSaving}
              >
                {isSaving ? "Saving..." : "Save"}
              </button>
            </>
          )}
        </div>
      </div>

      {/* Rules Editor */}
      <div className={styles.rulesSection}>
        <div className={styles.rulesTitleBar}>
          <div className={styles.rulesTitleLeft}>
            <h3 className={styles.rulesTitle}>Rules</h3>
            {!isEditing ? (
              <span className={`${styles.badge} ${styles.badgeReadOnly}`}>Read-only</span>
            ) : (
              <span className={`${styles.badge} ${styles.badgeEditing}`}>Editing</span>
            )}
          </div>
        </div>
        <div className={`${styles.rulesContent} ${isEditing ? styles.rulesContentEditing : styles.rulesContentReadOnly}`}>
          <RuleEditor
            rules={isEditing ? draftRules : profile.rules}
            onChange={handleRulesChange}
            onErrorChange={handleRulesErrorChange}
            readOnly={!isEditing}
          />
        </div>
      </div>

      {/* Import Dialog */}
      <ImportDialog
        open={importDialogOpen}
        onClose={() => setImportDialogOpen(false)}
        mode="replace"
        onRulesParsed={handleRulesParsed}
      />

      {/* Create Profile Dialog */}
      <CreateProfileDialog
        open={showCreateDialog}
        onClose={() => setShowCreateDialog(false)}
        onCreate={handleCreateProfile}
        isLoading={isLoading}
      />
    </div>
  );
}

export default ProfileView;
