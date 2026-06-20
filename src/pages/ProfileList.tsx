import { useEffect, useState, useCallback } from "react";
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

  const [newName, setNewName] = useState("");
  const [showCreate, setShowCreate] = useState(false);

  useEffect(() => {
    // Load profiles on mount; gracefully handle missing backend
    fetchProfiles().catch((err: unknown) => {
      setError(err instanceof Error ? err.message : String(err));
    });
  }, [fetchProfiles, setError]);

  const handleCreate = useCallback(async () => {
    const name = newName.trim();
    if (!name) return;
    try {
      const profile = await createProfile(name);
      setNewName("");
      setShowCreate(false);
      setSelectedId(profile.id);
      navigate(`/profiles/${profile.id}`);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [newName, createProfile, setSelectedId, navigate, setError]);

  const handleDelete = useCallback(
    async (id: string) => {
      if (!confirm("Delete this profile?")) return;
      try {
        await deleteProfile(id);
      } catch (err: unknown) {
        setError(err instanceof Error ? err.message : String(err));
      }
    },
    [deleteProfile, setError],
  );

  const handleToggle = useCallback(
    async (id: string) => {
      try {
        await toggleEnabled(id);
      } catch (err: unknown) {
        setError(err instanceof Error ? err.message : String(err));
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

  return (
    <div className="mhost-page">
      <header className="mhost-page-header">
        <h1 className="mhost-page-title">Profiles</h1>
        <button
          className="btn btn-primary"
          onClick={() => setShowCreate(true)}
          disabled={isLoading}
        >
          + New Profile
        </button>
      </header>

      {error && <div className="alert alert-error">{error}</div>}

      {showCreate && (
        <div className={`card ${styles.createCard}`}>
          <h3>Create Profile</h3>
          <div className="form-row">
            <input
              className="input"
              placeholder="Profile name"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleCreate();
              }}
              autoFocus
            />
            <button
              className="btn btn-primary"
              onClick={handleCreate}
              disabled={!newName.trim() || isLoading}
            >
              Create
            </button>
            <button
              className="btn btn-ghost"
              onClick={() => {
                setShowCreate(false);
                setNewName("");
              }}
            >
              Cancel
            </button>
          </div>
        </div>
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
          <div
            key={profile.id}
            className={`${styles.profileCard} ${profile.enabled ? styles.profileCardEnabled : ""}`}
          >
            <div className={styles.profileCardMain}>
              <div className={styles.profileCardHeader}>
                <h3
                  className={styles.profileName}
                  onClick={() => handleEdit(profile.id)}
                >
                  {profile.name}
                </h3>
                <div className={styles.profileTags}>
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
              <div className={styles.profileMeta}>
                <span>{profile.rules.length} rules</span>
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
                  onChange={() => handleToggle(profile.id)}
                  disabled={isLoading}
                />
                <span className="toggle-slider" />
              </label>
              <button
                className="btn btn-ghost btn-sm"
                onClick={() => handleEdit(profile.id)}
              >
                Edit
              </button>
              <button
                className="btn btn-danger btn-sm"
                onClick={() => handleDelete(profile.id)}
                disabled={profile.protected || isLoading}
              >
                Delete
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

export default ProfileList;
