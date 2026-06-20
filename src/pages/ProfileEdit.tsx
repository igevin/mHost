import { useEffect, useState, useCallback } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { useAtomValue, useSetAtom } from "jotai";
import {
  profilesAtom,
  selectedProfileIdAtom,
  isLoadingAtom,
  errorAtom,
  fetchProfileAtom,
  updateProfileAtom,
} from "../stores/profiles";
import type { Profile } from "../types";
import BasicInfoForm from "../components/BasicInfoForm";
import RuleList from "../components/RuleList";
import styles from "./ProfileEdit.module.css";

function ProfileEdit() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const profiles = useAtomValue(profilesAtom);
  const isLoading = useAtomValue(isLoadingAtom);
  const error = useAtomValue(errorAtom);
  const setSelectedId = useSetAtom(selectedProfileIdAtom);
  const setError = useSetAtom(errorAtom);

  const fetchProfile = useSetAtom(fetchProfileAtom);
  const updateProfile = useSetAtom(updateProfileAtom);

  const [draft, setDraft] = useState<Profile | null>(null);
  const [hasChanges, setHasChanges] = useState(false);

  const profile = profiles.find((p) => p.id === id);

  useEffect(() => {
    if (id) {
      setSelectedId(id);
      if (!profile) {
        fetchProfile(id).catch((err: unknown) => {
          setError(err instanceof Error ? err.message : String(err));
        });
      }
    }
  }, [id, setSelectedId, fetchProfile, profile, setError]);

  useEffect(() => {
    if (profile) {
      setDraft({ ...profile });
      setHasChanges(false);
    }
  }, [profile]);

  const handleChange = useCallback(
    (field: keyof Profile, value: unknown) => {
      setDraft((prev) => {
        if (!prev) return prev;
        const next = { ...prev, [field]: value };
        setHasChanges(true);
        return next;
      });
    },
    [],
  );

  const handleSave = useCallback(async () => {
    if (!draft) return;
    try {
      await updateProfile(draft);
      setHasChanges(false);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [draft, updateProfile, setError]);

  if (!profile || !draft) {
    return (
      <div className="mhost-page">
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
    <div className="mhost-page">
      <header className="mhost-page-header">
        <div>
          <h1 className="mhost-page-title">Edit Profile</h1>
          <p className="mhost-page-subtitle">{profile.name}</p>
        </div>
        <div className="mhost-page-actions">
          <button
            className="btn btn-primary"
            onClick={handleSave}
            disabled={!hasChanges || isLoading}
          >
            Save
          </button>
          <button
            className="btn btn-ghost"
            onClick={() => navigate("/profiles")}
          >
            Back
          </button>
        </div>
      </header>

      {error && <div className="alert alert-error">{error}</div>}

      <div className={styles.editGrid}>
        <BasicInfoForm draft={draft} onChange={handleChange} />

        <div className="card">
          <h3 className="card-title">Rules ({draft.rules.length})</h3>
          <RuleList rules={draft.rules} />
        </div>
      </div>
    </div>
  );
}

export default ProfileEdit;
