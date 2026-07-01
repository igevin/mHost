import { useState, useEffect } from "react";
import { readSystemHosts } from "../lib/tauri";
import { extractErrorMessage } from "../lib/error";
import styles from "./Settings.module.css";

function Settings() {
  const [hostsContent, setHostsContent] = useState<string | null>(null);
  const [hostsError, setHostsError] = useState<string | null>(null);

  useEffect(() => {
    readSystemHosts()
      .then((content) => setHostsContent(content))
      .catch((err) => setHostsError(extractErrorMessage(err)));
  }, []);

  return (
    <div className="mhost-page">
      <header className="mhost-page-header">
        <h1 className="mhost-page-title">Settings</h1>
      </header>

      <div className={styles.settingsGrid}>
        {/* About Card */}
        <div className={`card ${styles.aboutCard}`}>
          <div className={styles.aboutLogo}>m</div>
          <div className={styles.aboutName}>mHost</div>
          <div className={styles.aboutVersion}>Version {__APP_VERSION__}</div>
          <div className={styles.aboutInfo}>
            <div className={styles.aboutInfoItem}>
              <div className={styles.label}>Phase</div>
              <div className={styles.value}>MVP Profile Switching</div>
            </div>
            <div className={styles.aboutInfoItem}>
              <div className={styles.label}>Platform</div>
              <div className={styles.value}>macOS</div>
            </div>
          </div>
        </div>

        {/* Storage Card */}
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

        {/* System Hosts Preview */}
        <div className={`card ${styles.previewCard}`}>
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
