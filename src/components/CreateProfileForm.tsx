import { useState } from "react";
import styles from "../pages/ProfileList.module.css";

interface CreateProfileFormProps {
  isLoading: boolean;
  onCreate: (name: string) => void;
  onCancel: () => void;
}

function CreateProfileForm({ isLoading, onCreate, onCancel }: CreateProfileFormProps) {
  const [newName, setNewName] = useState("");

  const handleCreate = () => {
    const name = newName.trim();
    if (!name) return;
    onCreate(name);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") handleCreate();
  };

  return (
    <div className={`card ${styles.createCard}`}>
      <h3>Create Profile</h3>
      <div className="form-row">
        <input
          className="input"
          placeholder="Profile name"
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
          onKeyDown={handleKeyDown}
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
          onClick={onCancel}
        >
          Cancel
        </button>
      </div>
    </div>
  );
}

export default CreateProfileForm;
