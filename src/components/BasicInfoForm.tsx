import type { Profile } from "../types";

interface BasicInfoFormProps {
  draft: Profile;
  onChange: (field: keyof Profile, value: unknown) => void;
}

function BasicInfoForm({ draft, onChange }: BasicInfoFormProps) {
  const handleTagsChange = (value: string) => {
    const tags = value
      .split(",")
      .map((t) => t.trim())
      .filter(Boolean);
    onChange("tags", tags);
  };

  return (
    <div className="card">
      <h3 className="card-title">Basic Info</h3>
      <div className="form-group">
        <label className="form-label">Name</label>
        <input
          className="input"
          value={draft.name}
          onChange={(e) => onChange("name", e.target.value)}
        />
      </div>
      <div className="form-group">
        <label className="form-label">Description</label>
        <textarea
          className="input textarea"
          rows={3}
          value={draft.description ?? ""}
          onChange={(e) =>
            onChange("description", e.target.value || null)
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
  );
}

export default BasicInfoForm;
