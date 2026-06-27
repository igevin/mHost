import { useState, useCallback, useMemo, useEffect } from "react";
import { createPortal } from "react-dom";
import { useNavigate } from "react-router-dom";
import { useAtomValue, useSetAtom } from "jotai";
import {
  profilesAtom,
  deleteProfileAtom,
  toggleProfileEnabledAtom,
  errorAtom,
  createProfileAtom,
  isLoadingAtom,
  fetchProfilesAtom,
} from "../stores/profiles";
import { countRealRules } from "../lib/rules";
import { exportProfileToFile, duplicateProfile } from "../lib/tauri";
import { extractErrorMessage } from "../lib/error";
import { save, confirm } from "@tauri-apps/plugin-dialog";
import type { Profile, ExportFormat } from "../types";
import ImportDialog from "./ImportDialog";
import styles from "./ManagementDrawer.module.css";

interface ManagementDrawerProps {
  open: boolean;
  onClose: () => void;
}

function ManagementDrawer({ open, onClose }: ManagementDrawerProps) {
  const navigate = useNavigate();
  const profiles = useAtomValue(profilesAtom);
  const setError = useSetAtom(errorAtom);
  const deleteProfile = useSetAtom(deleteProfileAtom);
  const toggleEnabled = useSetAtom(toggleProfileEnabledAtom);
  const createProfile = useSetAtom(createProfileAtom);
  const fetchProfiles = useSetAtom(fetchProfilesAtom);
  const isLoading = useAtomValue(isLoadingAtom);

  // Import dialog state -- hooks must be called unconditionally before any early return
  const [showImport, setShowImport] = useState(false);

  // Loading state for toggle operations to prevent duplicate clicks
  const [togglingId, setTogglingId] = useState<string | null>(null);

  // Create profile dialog state
  const [showCreateDialog, setShowCreateDialog] = useState(false);
  const [newProfileName, setNewProfileName] = useState("");

  // Escape key handler
  useEffect(() => {
    if (!open) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [open, onClose]);

  // Stats calculation
  const { totalProfiles, enabledCount, totalRules } = useMemo(
    () => ({
      totalProfiles: profiles.length,
      enabledCount: profiles.filter((p) => p.enabled).length,
      totalRules: profiles.reduce((sum, p) => sum + countRealRules(p.rules), 0),
    }),
    [profiles],
  );

  const handleClose = useCallback(() => {
    onClose();
  }, [onClose]);

  const handleOverlayClick = useCallback(
    (e: React.MouseEvent) => {
      if (e.target === e.currentTarget) {
        onClose();
      }
    },
    [onClose],
  );

  const handleNewProfile = useCallback(() => {
    setNewProfileName("");
    setShowCreateDialog(true);
  }, []);

  const handleCreateProfile = useCallback(async () => {
    const name = newProfileName.trim();
    if (!name) return;
    try {
      const profile = await createProfile(name);
      setShowCreateDialog(false);
      onClose();
      navigate(`/profiles/${profile.id}`);
    } catch (err: unknown) {
      setError(extractErrorMessage(err));
    }
  }, [newProfileName, createProfile, onClose, navigate, setError]);

  const handleImport = useCallback(() => {
    onClose();
    setShowImport(true);
  }, [onClose]);

  const handleEdit = useCallback(
    (id: string) => {
      onClose();
      navigate(`/profiles/${id}`);
    },
    [onClose, navigate],
  );

  const handleDuplicate = useCallback(
    async (profile: Profile) => {
      try {
        await duplicateProfile(profile.id, `${profile.name} (copy)`);
        await fetchProfiles();
      } catch (err: unknown) {
        setError(extractErrorMessage(err));
      }
    },
    [setError, fetchProfiles],
  );

  const handleExport = useCallback(
    async (profile: Profile, format: ExportFormat) => {
      try {
        const path = await save({
          defaultPath: `${profile.name}.${format === "hosts" ? "hosts" : "json"}`,
          filters:
            format === "hosts"
              ? [{ name: "Hosts", extensions: ["hosts", "txt"] }]
              : [{ name: "JSON", extensions: ["json"] }],
        });
        if (path) {
          await exportProfileToFile(profile.id, format, path);
        }
      } catch (err: unknown) {
        setError(extractErrorMessage(err));
      }
    },
    [setError],
  );

  const handleDelete = useCallback(
    async (id: string) => {
      const confirmed = await confirm("Delete this profile?");
      if (!confirmed) return;
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
      if (togglingId) return; // prevent duplicate clicks
      setTogglingId(id);
      try {
        await toggleEnabled(id);
      } catch (err: unknown) {
        setError(extractErrorMessage(err));
      } finally {
        setTogglingId(null);
      }
    },
    [toggleEnabled, setError, togglingId],
  );

  const handleImported = useCallback(async (profile: Profile) => {
    setShowImport(false);
    onClose();
    // Refresh profile list so the imported profile is available
    await fetchProfiles();
    navigate(`/profiles/${profile.id}`);
  }, [onClose, navigate, fetchProfiles]);

  if (!open && !showImport) return null;

  return createPortal(
    <>
      {/* Overlay (only show when drawer is open) */}
      {open && (
        <div className={styles.drawerOverlay} onClick={handleOverlayClick} />
      )}

      {/* Drawer Panel */}
      {open && (
        <div className={styles.drawer}>
          {/* Header */}
          <div className={styles.drawerHeader}>
            <h2 className={styles.drawerTitle}>Profile Management</h2>
            <button
              className={styles.closeBtn}
              onClick={handleClose}
              aria-label="Close"
              title="Close"
            >
              x
            </button>
          </div>

          {/* Body */}
          <div className={styles.drawerBody}>
            {/* Stats Grid */}
            <div className={styles.statsGrid}>
              <div className={styles.statCard}>
                <div className={styles.statLabel}>Total Profiles</div>
                <div className={styles.statValue}>{totalProfiles}</div>
              </div>
              <div className={styles.statCard}>
                <div className={styles.statLabel}>Enabled</div>
                <div className={styles.statValue}>{enabledCount}</div>
              </div>
              <div className={styles.statCard}>
                <div className={styles.statLabel}>Total Rules</div>
                <div className={styles.statValue}>{totalRules}</div>
              </div>
            </div>

            {/* Actions */}
            <div className={styles.actionsBar}>
              <button className="btn btn-primary" onClick={handleNewProfile}>
                + New Profile
              </button>
              <button className="btn btn-ghost" onClick={handleImport}>
                Import
              </button>
            </div>

            {/* Profile Cards */}
            {profiles.map((profile) => (
              <div
                key={profile.id}
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
                  <span>{countRealRules(profile.rules)} rules</span>
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
                    onClick={() => handleEdit(profile.id)}
                  >
                    Edit
                  </button>
                  <button
                    className="btn btn-ghost btn-sm"
                    onClick={() => handleToggle(profile.id)}
                    disabled={togglingId === profile.id}
                  >
                    {togglingId === profile.id
                      ? (profile.enabled ? "Disabling..." : "Enabling...")
                      : (profile.enabled ? "Disable" : "Enable")}
                  </button>
                  <button
                    className="btn btn-ghost btn-sm"
                    onClick={() => handleDuplicate(profile)}
                  >
                    Duplicate
                  </button>
                  <button
                    className="btn btn-ghost btn-sm"
                    onClick={() => handleExport(profile, "hosts")}
                  >
                    Export
                  </button>
                  <button
                    className="btn btn-danger btn-sm"
                    onClick={() => handleDelete(profile.id)}
                    disabled={profile.protected}
                  >
                    Delete
                  </button>
                </div>
              </div>
            ))}

            {profiles.length === 0 && (
              <div className="empty-state">
                <p>No profiles yet.</p>
                <p className="empty-hint">
                  Create a profile to manage your hosts rules.
                </p>
              </div>
            )}
          </div>
        </div>
      )}

      {/* Import Dialog */}
      <ImportDialog
        open={showImport}
        onClose={() => setShowImport(false)}
        onImported={handleImported}
      />

      {/* Create Profile Dialog */}
      {showCreateDialog && (
        <div className={styles.createDialogOverlay} onClick={() => setShowCreateDialog(false)}>
          <div className={styles.createDialog} onClick={(e) => e.stopPropagation()}>
            <h3 className={styles.createDialogTitle}>Create Profile</h3>
            <div className="form-row">
              <input
                className="input"
                placeholder="Profile name"
                value={newProfileName}
                onChange={(e) => setNewProfileName(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") handleCreateProfile(); }}
                autoFocus
              />
              <button
                className="btn btn-primary"
                onClick={handleCreateProfile}
                disabled={!newProfileName.trim() || isLoading}
              >
                Create
              </button>
              <button
                className="btn btn-ghost"
                onClick={() => setShowCreateDialog(false)}
              >
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}
    </>,
    document.body,
  );
}

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

export default ManagementDrawer;
