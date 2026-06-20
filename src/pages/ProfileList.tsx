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
import ProfileCard from "../components/ProfileCard";
import CreateProfileForm from "../components/CreateProfileForm";
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

  useEffect(() => {
    // Load profiles on mount; gracefully handle missing backend
    fetchProfiles().catch((err: unknown) => {
      setError(err instanceof Error ? err.message : String(err));
    });
  }, [fetchProfiles, setError]);

  const handleCreate = useCallback(async (name: string) => {
    try {
      const profile = await createProfile(name);
      setShowCreate(false);
      setSelectedId(profile.id);
      navigate(`/profiles/${profile.id}`);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [createProfile, setSelectedId, navigate, setError]);

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
          />
        ))}
      </div>
    </div>
  );
}

export default ProfileList;
