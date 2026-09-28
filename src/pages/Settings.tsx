import { useCallback, useState, useEffect } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import {
  dnsEnabledAtom,
  dnsStatusAtom,
  isDnsLoadingAtom,
  toggleDnsModeAtom,
  cancelActiveDnsToggle,
  dnsErrorAtom,
  quickApplyOnToggleAtom,
  // issue #153
  systemDnsAtom,
  dnsDiscrepancyAtom,
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
  // issue #153: 系统 DNS 实际状态 vs mHost 内存态的分歧
  const systemDns = useAtomValue(systemDnsAtom);
  const dnsDiscrepancy = useAtomValue(dnsDiscrepancyAtom);
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
  const { fire, releaseSoon } = useWebKitPointerDown();

  // issue #123: persisted Quick Apply preference (localStorage-backed atom).
  const quickApplyOnToggle = useAtomValue(quickApplyOnToggleAtom);
  const setQuickApplyOnToggle = useSetAtom(quickApplyOnToggleAtom);

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
      releaseSoon();
      toggleDnsMode(enabled);
    },
    [fire, releaseSoon, toggleDnsMode],
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

        {/* Apply Card (issue #123) */}
        <div className="card">
          <h3 className="card-title">Apply</h3>
          <div className={styles.settingRow}>
            <div className={styles.settingInfo}>
              <div className={styles.settingLabel}>
                Quick Apply on profile toggle
              </div>
              <div className={styles.settingDesc}>
                Skip the Apply Preview dialog when toggling Hosts profiles.
                Hold Cmd or Option while toggling to force the Preview
                dialog. DNS profile toggles are not affected.
              </div>
            </div>
            <label
              className={styles.toggleSwitch}
              title="Toggle Quick Apply"
              data-testid="quick-apply-toggle"
            >
              <input
                type="checkbox"
                role="switch"
                checked={quickApplyOnToggle}
                onChange={(e) => setQuickApplyOnToggle(e.target.checked)}
              />
              <span className={styles.toggleSlider} />
            </label>
          </div>
        </div>

        {/* DNS Mode Card */}
        <div className="card">
          <h3 className="card-title">DNS Mode</h3>
          {/*
            Issue #153: `dnsEnabledAtom`（Rust 内存态）与系统 DNS 实际状态
            分歧时的横幅。

            两种方向的用户含义完全不同，所以处理方式也不同：

            - `stuck_at_loopback`（显示 Stopped，但系统仍指向 127.0.0.1）
              —— **危险**：用户的 DNS 已经指向一个没人监听的地址，解析会
              直接失败。给一键恢复：调 `set_dns_mode(false)`。该后端路径
              **没有**「已禁用就短路」的分支（`set_dns_mode_disable` 直接
              走 `disable_dns_mode()`），所以即使内存态已经是 false，它仍
              会真的执行系统 DNS 还原 —— 这正是这里需要的能力。

            - `not_pointing`（显示 Running，但系统没指向 mHost）
              —— **不危险**：DNS 本身还能用，只是 mHost 的规则没生效。
              只提示，不给一键修复 —— 要修就得重启 enable 流程
              （disable → enable），那会弹两次 sudo 并重建 DNS server，
              为「省一次手动开关」在特权路径上新增代码不值得
              （见 issue #153 的 Assumptions）。
          */}
          {dnsDiscrepancy && (
            <div
              className={styles.dnsDiscrepancyBanner}
              data-testid="dns-discrepancy-banner"
              data-discrepancy={dnsDiscrepancy}
              role="alert"
            >
              <div className={styles.dnsDiscrepancyText}>
                <div className={styles.dnsDiscrepancyTitle}>
                  {dnsDiscrepancy === "stuck_at_loopback"
                    ? "System DNS still points at mHost"
                    : "System DNS does not point at mHost"}
                </div>
                <div className={styles.dnsDiscrepancyDetail}>
                  {dnsDiscrepancy === "stuck_at_loopback" ? (
                    <>
                      mHost reports DNS mode as{" "}
                      <strong>Stopped</strong>, but your system DNS is{" "}
                      <strong>{systemDns?.servers.join(", ") || "127.0.0.1"}</strong>{" "}
                      on {systemDns?.interface}. Domain resolution may be
                      broken right now.
                    </>
                  ) : (
                    <>
                      mHost reports DNS mode as <strong>Running</strong>, but{" "}
                      {systemDns?.interface} is using{" "}
                      <strong>
                        {systemDns && systemDns.servers.length > 0
                          ? systemDns.servers.join(", ")
                          : "the system default"}
                      </strong>
                      . mHost rules are not being applied. Toggle DNS mode
                      off and on to re-apply.
                    </>
                  )}
                </div>
              </div>
              {dnsDiscrepancy === "stuck_at_loopback" && (
                <button
                  className="btn btn-sm btn-primary"
                  disabled={isDnsLoading}
                  data-testid="dns-restore-button"
                  // 复用主开关的 handler（不要在这里自己调 `fire()`：
                  // `useWebKitPointerDown` 的 firedRef 只在
                  // `releaseSoon()` 里复位，只 fire 不 release 会让这个
                  // 按钮在 Settings 挂载期内**永久失效**一次点击）。
                  // 语义也正好是我们要的：后端 disable 路径会真的跑一次
                  // 系统 DNS 还原，不依赖内存态是否为 false。
                  onClick={() => handleToggleDns(false)}
                >
                  {isDnsLoading ? "Restoring…" : "Restore system DNS"}
                </button>
              )}
            </div>
          )}
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
