import { useCallback } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import {
  dnsEnabledAtom,
  dnsStatusAtom,
  isDnsLoadingAtom,
  toggleDnsModeAtom,
  dnsErrorAtom,
} from "../stores/profiles";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import styles from "./Settings.module.css";

function Settings() {
  const dnsEnabled = useAtomValue(dnsEnabledAtom);
  const dnsStatus = useAtomValue(dnsStatusAtom);
  const isDnsLoading = useAtomValue(isDnsLoadingAtom);
  const dnsError = useAtomValue(dnsErrorAtom);
  const toggleDnsMode = useSetAtom(toggleDnsModeAtom);
  const { onPointerDown } = useWebKitPointerDown();

  const handleToggleDns = useCallback(
    (enabled: boolean) => {
      toggleDnsMode(enabled);
    },
    [toggleDnsMode],
  );

  return (
    <div className="mhost-page">
      {dnsError && <div className="alert alert-error">{dnsError}</div>}
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

        {/* DNS Mode Card */}
        <div className="card">
          <h3 className="card-title">DNS Mode</h3>
          <div className={styles.dnsStatusRow}>
            <span className={styles.dnsStatusLabel}>Status:</span>
            <span className={dnsEnabled ? styles.dnsStatusOn : styles.dnsStatusOff}>
              {dnsEnabled ? "Running" : "Stopped"}
            </span>
            {dnsEnabled && dnsStatus && (
              <span className={styles.dnsStatusDetail}>
                {dnsStatus.rule_count} rules &middot; Port {dnsStatus.port}
              </span>
            )}
          </div>
          <div className={styles.dnsActions}>
            {dnsEnabled ? (
              <button
                className="btn btn-danger"
                onClick={() => handleToggleDns(false)}
                onPointerDown={onPointerDown(() => handleToggleDns(false))}
                disabled={isDnsLoading}
              >
                {isDnsLoading ? "Disabling..." : "Disable DNS Mode"}
              </button>
            ) : (
              <button
                className="btn btn-primary"
                onClick={() => handleToggleDns(true)}
                onPointerDown={onPointerDown(() => handleToggleDns(true))}
                disabled={isDnsLoading}
              >
                {isDnsLoading ? "Enabling..." : "Enable DNS Mode"}
              </button>
            )}
          </div>
          {dnsEnabled && dnsStatus && (
            <div className={styles.dnsDetails}>
              <div>
                Upstream (resolver for unmatched queries):{" "}
                {dnsStatus.upstream.length > 0
                  ? dnsStatus.upstream.join(", ")
                  : "System default"}
              </div>
              <div>
                Original DNS (will be restored on disable):{" "}
                {(dnsStatus.original_dns ?? []).length > 0
                  ? (dnsStatus.original_dns ?? []).join(", ")
                  : "(empty — DHCP default)"}
              </div>
              <div>Cache capacity: {dnsStatus.cache_capacity}</div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export default Settings;
