import { useState, useEffect } from "react";
import { useSetAtom } from "jotai";
import { readSystemHosts } from "../lib/tauri";
import { rollbackHostsActionAtom } from "../stores/profiles";
import RollbackButton from "../components/RollbackButton";
import styles from "./Settings.module.css";

function Settings() {
  const [hostsContent, setHostsContent] = useState<string | null>(null);
  const [hostsError, setHostsError] = useState<string | null>(null);
  const rollback = useSetAtom(rollbackHostsActionAtom);

  useEffect(() => {
    readSystemHosts()
      .then((content) => setHostsContent(content))
      .catch((err) => setHostsError(err instanceof Error ? err.message : String(err)));
  }, []);

  return (
    <div className="mhost-page">
      <header className="mhost-page-header">
        <h1 className="mhost-page-title">Settings</h1>
      </header>

      <div className={styles.settingsGrid}>
        <div className="card">
          <h3 className="card-title">About</h3>
          <div className={styles.infoRow}>
            <span className={styles.infoLabel}>App Name</span>
            <span className={styles.infoValue}>mHost</span>
          </div>
          <div className={styles.infoRow}>
            <span className={styles.infoLabel}>Version</span>
            <span className={styles.infoValue}>1 (MVP Profile Switching)</span>
          </div>
        </div>

        <div className="card">
          <h3 className="card-title">Storage</h3>
          <div className={styles.infoRow}>
            <span className={styles.infoLabel}>Data Directory</span>
            <span className={styles.infoValue}>~/Library/Application Support/mHost</span>
          </div>
          <div className={styles.infoRow}>
            <span className={styles.infoLabel}>Profiles</span>
            <span className={styles.infoValue}>profiles/</span>
          </div>
          <div className={styles.infoRow}>
            <span className={styles.infoLabel}>Backups</span>
            <span className={styles.infoValue}>backups/</span>
          </div>
        </div>

        <div className="card">
          <h3 className="card-title">Hosts Management</h3>
          <p className={styles.sectionDesc}>
            Rollback the system hosts file to the last backed-up version.
          </p>
          <RollbackButton onRollback={rollback} />
        </div>

        <div className="card card-full">
          <h3 className="card-title">System Hosts Preview</h3>
          {hostsError ? (
            <div className="alert alert-error">{hostsError}</div>
          ) : hostsContent === null ? (
            <div className="loading">Loading...</div>
          ) : (
            <pre className={styles.hostsPreview}>{hostsContent}</pre>
          )}
        </div>
      </div>
    </div>
  );
}

export default Settings;
