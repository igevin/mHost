import { useEffect, useState, useCallback, useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { useAtomValue, useSetAtom } from "jotai";
import {
  profilesAtom,
  selectedProfileIdAtom,
  isLoadingAtom,
  errorAtom,
  fetchProfilesAtom,
  createProfileAtom,
  deleteProfileAtom,
  toggleProfileEnabledAtom,
} from "../stores/profiles";
import { extractErrorMessage } from "../lib/error";
import type { Profile, ExportFormat } from "../types";
import { exportProfileToFile, duplicateProfile } from "../lib/tauri";
import { save } from "@tauri-apps/plugin-dialog";
import ProfileCard from "../components/ProfileCard";
import CreateProfileForm from "../components/CreateProfileForm";
import ImportDialog from "../components/ImportDialog";
import styles from "./ProfileList.module.css";

function ProfileList() {
  const navigate = useNavigate();
  const profiles = useAtomValue(profilesAtom);
  const isLoading = useAtomValue(isLoadingAtom);
  const error = useAtomValue(errorAtom);
  const setSelectedId = useSetAtom(selectedProfileIdAtom);
  const setError = useSetAtom(errorAtom);

  const fetchProfiles = useSetAtom(fetchProfilesAtom);
  const createProfile = useSetAtom(createProfileAtom);
  const deleteProfile = useSetAtom(deleteProfileAtom);
  const toggleEnabled = useSetAtom(toggleProfileEnabledAtom);

  const [showCreate, setShowCreate] = useState(false);
  const [showImport, setShowImport] = useState(false);
  const [duplicateTarget, setDuplicateTarget] = useState<Profile | null>(null);
  const [duplicateName, setDuplicateName] = useState("");
  const [exportTarget, setExportTarget] = useState<Profile | null>(null);

  useEffect(() => {
    // Load profiles on mount; gracefully handle missing backend
    fetchProfiles().catch((err: unknown) => {
      setError(extractErrorMessage(err));
    });
  }, [fetchProfiles, setError]);

  const handleCreate = useCallback(async (name: string) => {
    try {
      const profile = await createProfile(name);
      setShowCreate(false);
      setSelectedId(profile.id);
      navigate(`/profiles/${profile.id}`);
    } catch (err: unknown) {
      setError(extractErrorMessage(err));
    }
  }, [createProfile, setSelectedId, navigate, setError]);

  const handleDelete = useCallback(
    async (id: string) => {
      if (!confirm("Delete this profile?")) return;
      try {
        await deleteProfile(id);
      } catch (err: unknown) {
        setError(extractErrorMessage(err));
      }
    },
    [deleteProfile, setError],
  );

  const handleToggle = useCallback(
    async (id: string) => {
      try {
        await toggleEnabled(id);
      } catch (err: unknown) {
        setError(extractErrorMessage(err));
      }
    },
    [toggleEnabled, setError],
  );

  const handleEdit = useCallback(
    (id: string) => {
      setSelectedId(id);
      navigate(`/profiles/${id}`);
    },
    [setSelectedId, navigate],
  );

  const handleImported = useCallback(
    (profile: Profile) => {
      setShowImport(false);
      setSelectedId(profile.id);
    },
    [setSelectedId],
  );

  const handleExport = useCallback(async (profile: Profile, format: ExportFormat) => {
    try {
      const path = await save({
        defaultPath: `${profile.name}.${format === "hosts" ? "hosts" : "json"}`,
        filters: format === "hosts"
          ? [{ name: "Hosts", extensions: ["hosts", "txt"] }]
          : [{ name: "JSON", extensions: ["json"] }],
      });
      if (path) {
        await exportProfileToFile(profile.id, format, path);
      }
    } catch (err: unknown) {
      setError(extractErrorMessage(err));
    }
    setExportTarget(null);
  }, [setError]);

  const handleDuplicate = useCallback(async () => {
    if (!duplicateTarget || !duplicateName.trim()) return;
    try {
      const profile = await duplicateProfile(duplicateTarget.id, duplicateName.trim());
      setDuplicateTarget(null);
      setDuplicateName("");
      setSelectedId(profile.id);
      navigate(`/profiles/${profile.id}`);
    } catch (err: unknown) {
      setError(extractErrorMessage(err));
    }
  }, [duplicateTarget, duplicateName, setSelectedId, navigate, setError]);

  // Stats calculation — Perf fix (#35): useMemo to avoid recalculating on every render
  const { totalProfiles, enabledProfiles, totalRules } = useMemo(() => ({
    totalProfiles: profiles.length,
    enabledProfiles: profiles.filter((p) => p.enabled).length,
    totalRules: profiles.reduce((sum, p) => sum + p.rules.length, 0),
  }), [profiles]);

  return (
    <div className="mhost-page">
      <header className="mhost-page-header">
        <h1 className="mhost-page-title">Profiles</h1>
        <div className="mhost-page-actions">
          <button
            className="btn btn-ghost"
            onClick={() => setShowImport(true)}
            disabled={isLoading}
          >
            Import
          </button>
          <button
            className="btn btn-primary"
            onClick={() => setShowCreate(true)}
            disabled={isLoading}
          >
            + New Profile
          </button>
        </div>
      </header>

      {error && <div className="alert alert-error">{error}</div>}

      {/* Stats Dashboard */}
      <div className={styles.statsGrid}>
        <div className={styles.statCard}>
          <div className={styles.statLabel}>Total Profiles</div>
          <div className={styles.statValue}>{totalProfiles}</div>
        </div>
        <div className={styles.statCard}>
          <div className={styles.statLabel}>Enabled</div>
          <div className={styles.statValue}>{enabledProfiles}</div>
        </div>
        <div className={styles.statCard}>
          <div className={styles.statLabel}>Total Rules</div>
          <div className={styles.statValue}>{totalRules}</div>
        </div>
      </div>

      {showCreate && (
        <CreateProfileForm
          isLoading={isLoading}
          onCreate={handleCreate}
          onCancel={() => setShowCreate(false)}
        />
      )}

      <div className={styles.profileList}>
        {profiles.length === 0 && !isLoading && (
          <div className="empty-state">
            <p>No profiles yet.</p>
            <p className="empty-hint">
              Create a profile to manage your hosts rules.
            </p>
          </div>
        )}

        {profiles.map((profile) => (
          <ProfileCard
            key={profile.id}
            profile={profile}
            isLoading={isLoading}
            onEdit={handleEdit}
            onToggle={handleToggle}
            onDelete={handleDelete}
            onExport={(id) => {
              const target = profiles.find((p) => p.id === id);
              if (target) {
                setExportTarget(target);
              }
            }}
            onDuplicate={(id) => {
              const target = profiles.find((p) => p.id === id);
              if (target) {
                setDuplicateTarget(target);
                setDuplicateName(`${target.name} (copy)`);
              }
            }}
          />
        ))}
      </div>

      {/* Import Dialog */}
      <ImportDialog
        open={showImport}
        onClose={() => setShowImport(false)}
        onImported={handleImported}
      />

      {/* Export Format Dialog */}
      {exportTarget && (
        <div className={styles.overlay} onClick={() => setExportTarget(null)}>
          <div className={styles.dialog} onClick={(e) => e.stopPropagation()}>
            <h3 className={styles.dialogTitle}>Export Profile</h3>
            <p className={styles.dialogDesc}>
              Export "{exportTarget.name}" as:
            </p>
            <div className={styles.dialogActions}>
              <button
                className="btn btn-primary"
                onClick={() => handleExport(exportTarget, "hosts")}
              >
                hosts format
              </button>
              <button
                className="btn btn-ghost"
                onClick={() => handleExport(exportTarget, "json")}
              >
                JSON format
              </button>
              <button
                className="btn btn-ghost"
                onClick={() => setExportTarget(null)}
              >
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Duplicate Dialog */}
      {duplicateTarget && (
        <div className={styles.overlay} onClick={() => setDuplicateTarget(null)}>
          <div className={styles.dialog} onClick={(e) => e.stopPropagation()}>
            <h3 className={styles.dialogTitle}>Duplicate Profile</h3>
            <div className="form-group">
              <label className="form-label">New Name</label>
              <input
                className="input"
                value={duplicateName}
                onChange={(e) => setDuplicateName(e.target.value)}
                autoFocus
              />
            </div>
            <div className={styles.dialogActions}>
              <button
                className="btn btn-primary"
                onClick={handleDuplicate}
                disabled={!duplicateName.trim() || isLoading}
              >
                Duplicate
              </button>
              <button
                className="btn btn-ghost"
                onClick={() => setDuplicateTarget(null)}
              >
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default ProfileList;
