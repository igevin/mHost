import type { Profile, ExportFormat } from "../types";
import styles from "../pages/ProfileList.module.css";

interface ProfileCardProps {
  profile: Profile;
  isLoading: boolean;
  onEdit: (id: string) => void;
  onToggle: (id: string) => void;
  onDelete: (id: string) => void;
  onExport?: (id: string, format: ExportFormat) => void;
  onDuplicate?: (id: string) => void;
}

function ProfileCard({
  profile,
  isLoading,
  onEdit,
  onToggle,
  onDelete,
  onExport,
  onDuplicate,
}: ProfileCardProps) {
  return (
    <div
      className={`${styles.profileCard} ${profile.enabled ? styles.profileCardEnabled : ""}`}
    >
      <div className={styles.profileCardMain}>
        <div className={styles.profileCardHeader}>
          <h3
            className={styles.profileName}
            onClick={() => onEdit(profile.id)}
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
            onChange={() => onToggle(profile.id)}
            disabled={isLoading}
          />
          <span className="toggle-slider" />
        </label>
        <button
          className="btn btn-ghost btn-sm"
          onClick={() => onEdit(profile.id)}
        >
          Edit
        </button>
        {onExport && (
          <button
            className="btn btn-ghost btn-sm"
            onClick={() => onExport(profile.id, "hosts")}
            disabled={isLoading}
          >
            Export
          </button>
        )}
        {onDuplicate && (
          <button
            className="btn btn-ghost btn-sm"
            onClick={() => onDuplicate(profile.id)}
            disabled={isLoading}
          >
            Duplicate
          </button>
        )}
        <button
          className="btn btn-danger btn-sm"
          onClick={() => onDelete(profile.id)}
          disabled={profile.protected || isLoading}
        >
          Delete
        </button>
      </div>
    </div>
  );
}

export default ProfileCard;
