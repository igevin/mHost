import { useCallback, useState, useEffect } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import {
  dnsEnabledAtom,
  dnsStatusAtom,
  isDnsLoadingAtom,
  toggleDnsModeAtom,
  cancelActiveDnsToggle,
  dnsErrorAtom,
} from "../stores/profiles";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import { checkUpdate } from "../lib/tauri";
import type { LatestRelease } from "../lib/tauri";
import styles from "./Settings.module.css";

function Settings() {
  const dnsEnabled = useAtomValue(dnsEnabledAtom);
  const dnsStatus = useAtomValue(dnsStatusAtom);
  const isDnsLoading = useAtomValue(isDnsLoadingAtom);
  const dnsError = useAtomValue(dnsErrorAtom);
  const toggleDnsMode = useSetAtom(toggleDnsModeAtom);
  // issue #149 / #123 follow-up: DNS toggle must dedupe pointerdown + click.
  // The `useWebKitPointerDown.onPointerDown` wrapper is NOT used here —
  // pairing it with a raw onClick double-fires the toggle (the wrapper
  // already calls `fire()`, and re-clicking calls the handler again before
  // `firedRef` resets). Sidebar/DrawerProfileCard hit the same bug in
  // commit 88641c6 and were fixed by routing both events through one
  // handler that owns `fire()` + `release()`. We do the same here —
  // without it, the second invocation overwrites the active
  // AbortController slot, and the synchronous re-render (Enable →
  // Cancel) causes the trailing click event to fire `handleCancelDns`,
  // which cancels the in-flight Rust `set_dns_mode` future before the
  // TCC prompt can appear. Symptom: Enable click hangs forever with
  // "Enable DNS mode" UI state and no macOS authorization dialog.
  const { fire, release } = useWebKitPointerDown();

  // Update check state
  const [updateStatus, setUpdateStatus] = useState<"idle" | "checking" | "available" | "up-to-date" | "error">("idle");
  const [latestRelease, setLatestRelease] = useState<LatestRelease | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);

  const doCheckUpdate = useCallback(async () => {
    setUpdateStatus("checking");
    setUpdateError(null);
    try {
      const release = await checkUpdate(__APP_VERSION__);
      if (release) {
        setLatestRelease(release);
        setUpdateStatus("available");
      } else {
        setLatestRelease(null);
        setUpdateStatus("up-to-date");
      }
    } catch (err) {
      setUpdateError(err instanceof Error ? err.message : String(err));
      setUpdateStatus("error");
    }
  }, []);

  // Check for updates on mount (best-effort, non-blocking)
  useEffect(() => {
    doCheckUpdate();
  }, [doCheckUpdate]);

  const handleToggleDns = useCallback(
    (enabled: boolean) => {
      // WebKit pointerdown/click dedupe — see comment on the
      // `useWebKitPointerDown` destructuring above. Without this guard
      // a single click fires `toggleDnsMode(enabled)` twice; the second
      // call overwrites the active AbortController slot, and the
      // pointerdown-triggered `set(isDnsLoadingAtom, true)` re-renders
      // Enable → Cancel before the click event fires. The trailing
      // click then lands on the newly-mounted Cancel button and cancels
      // the in-flight Rust enable — no TCC prompt ever appears.
      if (!fire()) return;
      release();
      toggleDnsMode(enabled);
    },
    [fire, release, toggleDnsMode],
  );

  // issue #149: Settings cancel button. Aborts the in-flight `set_dns_mode`
  // IPC, fires `cancel_dns_mode` to drive the backend rollback, and lets
  // `toggleDnsModeAtom`'s catch path revert the UI without surfacing an
  // error. No-op when no toggle is in flight.
  const handleCancelDns = useCallback(() => {
    cancelActiveDnsToggle();
  }, []);

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

          {/* Update check */}
          <div className={styles.updateSection}>
            {updateStatus === "checking" && (
              <span className={styles.updateChecking}>Checking for updates...</span>
            )}
            {updateStatus === "up-to-date" && (
              <span className={styles.updateUpToDate}>You&#39;re up to date!</span>
            )}
            {updateStatus === "available" && latestRelease && (
              <span className={styles.updateAvailable}>
                {latestRelease.title || latestRelease.tag} is available.{" "}
                <a
                  href={latestRelease.url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className={styles.updateLink}
                >
                  Download
                </a>
              </span>
            )}
            {updateStatus === "error" && (
              <span className={styles.updateError}>
                Update check failed: {updateError}
              </span>
            )}
            {(updateStatus === "idle" || updateStatus === "up-to-date" || updateStatus === "error" || updateStatus === "available") && (
              <button
                className={`btn btn-secondary ${styles.updateBtn}`}
                onClick={doCheckUpdate}
              >
                Check for Updates
              </button>
            )}
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
            {/* issue #149: while toggling, the primary action button is
                replaced with a Cancel button. Clicking it aborts the
                in-flight `set_dns_mode` IPC and fires the backend
                rollback — the user sees the UI revert without an
                error toast. */}
            {dnsEnabled ? (
              <button
                className="btn btn-danger"
                disabled={isDnsLoading}
                onClick={() => handleToggleDns(false)}
                onPointerDown={(e) => {
                  if (e.button !== 0) return;
                  handleToggleDns(false);
                }}
              >
                {isDnsLoading ? "Disabling…" : "Disable DNS Mode"}
              </button>
            ) : (
              <button
                className="btn btn-primary"
                disabled={isDnsLoading}
                onClick={() => handleToggleDns(true)}
                onPointerDown={(e) => {
                  if (e.button !== 0) return;
                  handleToggleDns(true);
                }}
              >
                {isDnsLoading ? "Enabling…" : "Enable DNS Mode"}
              </button>
            )}
            {/*
              Cancel button is rendered as a SIBLING — never as a swap-in
              replacement for the Enable/Disable button. The previous
              layout `{isDnsLoading ? <Cancel/> : <Enable/>}` mounted
              Cancel at the same DOM coordinate as Enable the instant
              pointerdown fired; the trailing synthetic `click` event
              then landed on the newly-mounted Cancel button and called
              `cancelActiveDnsToggle()` against the in-flight
              AbortController — the TCC dialog never appeared because
              the Rust CancellationToken was cancelled before
              spawn_blocking ran. Disabling the toggle button (instead
              of unmounting it) swallows the phantom click (disabled
              buttons fire no events) without losing the user's
              explicit Cancel intent.
            */}
            {isDnsLoading && (
              <button
                className="btn btn-danger"
                onClick={handleCancelDns}
                data-testid="dns-cancel-button"
              >
                Cancel
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
                {dnsStatus.original_dns.kind === "manual"
                  ? dnsStatus.original_dns.servers.length > 0
                    ? dnsStatus.original_dns.servers.join(", ")
                    : "(empty — DHCP default)"
                  : "(DHCP default — captured empty)"}
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
