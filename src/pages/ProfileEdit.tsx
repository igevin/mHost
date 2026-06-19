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

  const handleTagsChange = useCallback((value: string) => {
    const tags = value
      .split(",")
      .map((t) => t.trim())
      .filter(Boolean);
    handleChange("tags", tags);
  }, [handleChange]);

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

      <div className="edit-grid">
        <div className="card">
          <h3 className="card-title">Basic Info</h3>
          <div className="form-group">
            <label className="form-label">Name</label>
            <input
              className="input"
              value={draft.name}
              onChange={(e) => handleChange("name", e.target.value)}
            />
          </div>
          <div className="form-group">
            <label className="form-label">Description</label>
            <textarea
              className="input textarea"
              rows={3}
              value={draft.description ?? ""}
              onChange={(e) =>
                handleChange(
                  "description",
                  e.target.value || null,
                )
              }
            />
          </div>
          <div className="form-group">
            <label className="form-label">Tags (comma separated)</label>
            <input
              className="input"
              value={draft.tags.join(", ")}
              onChange={(e) => handleTagsChange(e.target.value)}
            />
          </div>
          <div className="form-group">
            <label className="form-label">Status</label>
            <div className="form-static">
              {draft.enabled ? "Enabled" : "Disabled"}
              {draft.protected && " · Protected"}
            </div>
          </div>
        </div>

        <div className="card">
          <h3 className="card-title">Rules ({draft.rules.length})</h3>
          {draft.rules.length === 0 ? (
            <div className="empty-state">
              <p>No rules in this profile.</p>
              <p className="empty-hint">
                Rule editing will be available in a later phase.
              </p>
            </div>
          ) : (
            <div className="rule-list">
              {draft.rules.map((rule) => (
                <div
                  key={rule.id}
                  className={`rule-item ${rule.enabled ? "" : "rule-item-disabled"}`}
                >
                  <div className="rule-header">
                    <span className="rule-ip">{rule.ip}</span>
                    <span className="rule-status">
                      {rule.enabled ? "On" : "Off"}
                    </span>
                  </div>
                  <div className="rule-domains">
                    {rule.domains.join(", ")}
                  </div>
                  {rule.comment && (
                    <div className="rule-comment">{rule.comment}</div>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export default ProfileEdit;
