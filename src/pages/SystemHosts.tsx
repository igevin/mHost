import { useState, useEffect } from "react";
import { readSystemHosts } from "../lib/tauri";
import { extractErrorMessage } from "../lib/error";
import styles from "./SystemHosts.module.css";

function SystemHosts() {
  const [hostsContent, setHostsContent] = useState<string | null>(null);
  const [hostsError, setHostsError] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;
    readSystemHosts()
      .then((content) => {
        if (mounted) setHostsContent(content);
      })
      .catch((err) => {
        if (mounted) setHostsError(extractErrorMessage(err));
      });
    return () => {
      mounted = false;
    };
  }, []);

  return (
    <div className="mhost-page">
      <header className="mhost-page-header">
        <h1 className="mhost-page-title">System Hosts</h1>
      </header>

      <div className={`card ${styles.hostsCard}`}>
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
  );
}

export default SystemHosts;
