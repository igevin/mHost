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
        <div className="card create-card">
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

      <div className="profile-list">
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
            className={`profile-card ${profile.enabled ? "profile-card-enabled" : ""}`}
          >
            <div className="profile-card-main">
              <div className="profile-card-header">
                <h3
                  className="profile-name"
                  onClick={() => handleEdit(profile.id)}
                >
                  {profile.name}
                </h3>
                <div className="profile-tags">
                  {profile.tags.map((tag) => (
                    <span key={tag} className="tag">
                      {tag}
                    </span>
                  ))}
                </div>
              </div>
              {profile.description && (
                <p className="profile-desc">{profile.description}</p>
              )}
              <div className="profile-meta">
                <span>{profile.rules.length} rules</span>
                <span className="meta-sep">·</span>
                <span>{profile.enabled ? "Enabled" : "Disabled"}</span>
                {profile.protected && (
                  <>
                    <span className="meta-sep">·</span>
                    <span className="protected-badge">Protected</span>
                  </>
                )}
              </div>
            </div>

            <div className="profile-card-actions">
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
