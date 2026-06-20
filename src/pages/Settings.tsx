import { useState, useEffect } from "react";
import { readSystemHosts } from "../lib/tauri";

function Settings() {
  const [hostsContent, setHostsContent] = useState<string | null>(null);
  const [hostsError, setHostsError] = useState<string | null>(null);

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

      <div className="settings-grid">
        <div className="card">
          <h3 className="card-title">About</h3>
          <div className="info-row">
            <span className="info-label">App Name</span>
            <span className="info-value">mHost</span>
          </div>
          <div className="info-row">
            <span className="info-label">Version</span>
            <span className="info-value">0.1.0</span>
          </div>
          <div className="info-row">
            <span className="info-label">Phase</span>
            <span className="info-value">0 (Skeleton)</span>
          </div>
        </div>

        <div className="card">
          <h3 className="card-title">Storage</h3>
          <div className="info-row">
            <span className="info-label">Data Directory</span>
            <span className="info-value">~/Library/Application Support/mHost</span>
          </div>
          <div className="info-row">
            <span className="info-label">Profiles</span>
            <span className="info-value">profiles/</span>
          </div>
          <div className="info-row">
            <span className="info-label">Backups</span>
            <span className="info-value">backups/</span>
          </div>
        </div>

        <div className="card card-full">
          <h3 className="card-title">System Hosts Preview</h3>
          {hostsError ? (
            <div className="alert alert-error">{hostsError}</div>
          ) : hostsContent === null ? (
            <div className="loading">Loading...</div>
          ) : (
            <pre className="hosts-preview">{hostsContent}</pre>
          )}
        </div>
      </div>
    </div>
  );
}

export default Settings;
