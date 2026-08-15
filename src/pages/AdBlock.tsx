import { useCallback, useState, useEffect } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import {
  adBlockStateAtom,
  isAdBlockLoadingAtom,
  adBlockErrorAtom,
  adBlockRuleCountAtom,
  adBlockHasErrorsAtom,
  dnsEnabledAtom,
  fetchAdBlockStateAtom,
  toggleAdBlockEnabledAtom,
  setAdBlockIntervalAtom,
  addAdBlockSourceAtom,
  removeAdBlockSourceAtom,
  setAdBlockSourceEnabledAtom,
  setAdBlockSourceResponseAtom,
  refreshAdBlockSourceAtom,
  refreshAllAdBlockSourcesAtom,
  addAdBlockWhitelistAtom,
  removeAdBlockWhitelistAtom,
} from "../stores/profiles";
import { useNavigate } from "react-router-dom";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import type { AdBlockResponse } from "../types";
import styles from "./AdBlock.module.css";

function AdBlock() {
  const state = useAtomValue(adBlockStateAtom);
  const isLoading = useAtomValue(isAdBlockLoadingAtom);
  const error = useAtomValue(adBlockErrorAtom);
  const dnsEnabled = useAtomValue(dnsEnabledAtom);
  const ruleCount = useAtomValue(adBlockRuleCountAtom);
  const hasErrors = useAtomValue(adBlockHasErrorsAtom);

  const fetchState = useSetAtom(fetchAdBlockStateAtom);
  const toggleEnabled = useSetAtom(toggleAdBlockEnabledAtom);
  const setInterval = useSetAtom(setAdBlockIntervalAtom);
  const addSource = useSetAtom(addAdBlockSourceAtom);
  const removeSource = useSetAtom(removeAdBlockSourceAtom);
  const setSourceEnabled = useSetAtom(setAdBlockSourceEnabledAtom);
  const setSourceResponse = useSetAtom(setAdBlockSourceResponseAtom);
  const refreshSource = useSetAtom(refreshAdBlockSourceAtom);
  const refreshAll = useSetAtom(refreshAllAdBlockSourcesAtom);
  const addWhitelist = useSetAtom(addAdBlockWhitelistAtom);
  const removeWhitelist = useSetAtom(removeAdBlockWhitelistAtom);

  const { onPointerDown } = useWebKitPointerDown();
  const navigate = useNavigate();

  // Local form state
  const [newName, setNewName] = useState("");
  const [newUrl, setNewUrl] = useState("");
  const [newResponse, setNewResponse] = useState<AdBlockResponse>("zero_address");
  const [newWhitelistDomain, setNewWhitelistDomain] = useState("");

  // Fetch on mount (idempotent — Tauri handles parallel calls).
  useEffect(() => {
    fetchState().catch(() => {
      /* error already in atom */
    });
  }, [fetchState]);

  const handleAddSource = useCallback(() => {
    if (!newName.trim() || !newUrl.trim()) return;
    addSource({ name: newName.trim(), url: newUrl.trim(), response: newResponse })
      .then(() => {
        setNewName("");
        setNewUrl("");
      })
      .catch(() => {
        /* error in atom */
      });
  }, [addSource, newName, newUrl, newResponse]);

  const handleAddWhitelist = useCallback(() => {
    const d = newWhitelistDomain.trim();
    if (!d) return;
    addWhitelist(d)
      .then(() => setNewWhitelistDomain(""))
      .catch(() => {
        /* error in atom */
      });
  }, [addWhitelist, newWhitelistDomain]);

  const handleIntervalChange = useCallback(
    (hours: number) => {
      setInterval(hours).catch(() => {});
    },
    [setInterval],
  );

  if (!state) {
    return (
      <div className="mhost-page">
        <header className="mhost-page-header">
          <h1 className="mhost-page-title">Ad Block</h1>
        </header>
        <div className={styles.muted}>Loading…</div>
      </div>
    );
  }

  // `dnsEnabled` flip controls whether the DNS engine actually applies
  // ad-block rules. Configuration edits below are ALWAYS persisted to
  // disk (and re-applied when DNS mode comes on), so the form is not
  // disabled when DNS is off — users often configure sources + whitelist
  // before enabling DNS mode for the first time. The banner below
  // explains the effective state.
  const dnsModeOff = !dnsEnabled;

  return (
    <div className="mhost-page">
      <header className="mhost-page-header">
        <h1 className="mhost-page-title">Ad Block</h1>
        <p className="mhost-page-subtitle">
          Block ads at the DNS resolver. macOS DNS mode only.
        </p>
        <div className="mhost-page-actions">
          <button
            className="btn btn-sm btn-ghost"
            onClick={() => refreshAll().catch(() => {})}
            disabled={isLoading || state.sources.length === 0}
            onPointerDown={onPointerDown(() => {})}
          >
            Refresh all
          </button>
        </div>
      </header>

      {error && <div className="alert alert-error">{error}</div>}

      {dnsModeOff && (
        <div className={styles.banner}>
          <span>
            DNS mode is off. Your edits below are saved and will apply the
            next time you enable DNS mode.
          </span>
          <button
            className="btn btn-sm btn-primary"
            onClick={() => navigate("/settings")}
            onPointerDown={onPointerDown(() => {})}
          >
            Open Settings
          </button>
        </div>
      )}

      <div className={styles.pageBody}>
        {/* Master switch + summary */}
        <div className="card">
          <div className={styles.bannerText}>
            <div>
              <div className={styles.bannerTitle}>Enable Ad Block</div>
              <div className={styles.muted}>
                When enabled, the DNS server returns 0.0.0.0 / NXDOMAIN for
                domains in any enabled source.
              </div>
            </div>
            <label className="toggle">
              <input
                type="checkbox"
                checked={state.enabled}
                onChange={(e) =>
                  toggleEnabled(e.target.checked).catch(() => {})
                }
                disabled={isLoading}
              />
              <span className="toggle-slider" />
            </label>
          </div>

          <div className={styles.summaryGrid}>
            <div className={styles.statCard}>
              <div className={styles.statValue}>{state.sources.length}</div>
              <div className={styles.statLabel}>Sources</div>
            </div>
            <div className={styles.statCard}>
              <div className={styles.statValue}>{ruleCount.toLocaleString()}</div>
              <div className={styles.statLabel}>Active Rules</div>
            </div>
            <div className={styles.statCard}>
              <div className={styles.statValue}>{state.whitelist.length}</div>
              <div className={styles.statLabel}>Whitelist</div>
            </div>
          </div>

          {hasErrors && (
            <div className={styles.dangerTextGap}>
              One or more sources have a fetch error — see badges below.
            </div>
          )}
        </div>

        {/* Add source form */}
        <div className="card">
          <h2 className="card-title">Sources</h2>
          <p className={styles.mutedGap}>
            Hosts-format blocklist URLs (one domain per line, IP ignored).
          </p>

          <div className={styles.addSourceForm}>
            <div className="form-group">
              <label className="form-label">Name</label>
              <input
                className="input"
                type="text"
                value={newName}
                placeholder="StevenBlack"
                onChange={(e) => setNewName(e.target.value)}
                disabled={isLoading}
              />
            </div>
            <div className="form-group">
              <label className="form-label">URL</label>
              <input
                className="input"
                type="url"
                value={newUrl}
                placeholder="https://example.com/hosts"
                onChange={(e) => setNewUrl(e.target.value)}
                disabled={isLoading}
              />
            </div>
            <div className="form-group">
              <label className="form-label">Response</label>
              <select
                className="input"
                value={newResponse}
                onChange={(e) =>
                  setNewResponse(e.target.value as AdBlockResponse)
                }
                disabled={isLoading}
              >
                <option value="zero_address">0.0.0.0</option>
                <option value="nx_domain">NXDOMAIN</option>
              </select>
            </div>
            <button
              className="btn btn-primary btn-sm"
              onClick={handleAddSource}
              disabled={isLoading || !newName.trim() || !newUrl.trim()}
              onPointerDown={onPointerDown(() => {})}
            >
              Add
            </button>
          </div>

          {/* Source list */}
          {state.sources.length === 0 ? (
            <div className={styles.empty}>No sources yet.</div>
          ) : (
            <div className={styles.columnGap}>
              {state.sources.map((src) => (
                <div
                  key={src.source_id}
                  className={`${styles.sourceCard} ${!src.enabled ? styles.dimmed : ""}`}
                >
                  <div className={styles.sourceHeader}>
                    <div className={styles.flexGrow}>
                      <div className={styles.sourceTitle}>
                        <span>{src.name}</span>
                        {src.last_error && (
                          <span
                            className={styles.errorBadge}
                            title={src.last_error}
                          >
                            fetch failed
                          </span>
                        )}
                      </div>
                      <div className={styles.sourceMeta}>{src.url}</div>
                      <div className={styles.sourceMeta}>
                        {src.rule_count.toLocaleString()} rules
                        {src.last_fetched_at &&
                          ` · fetched ${new Date(src.last_fetched_at).toLocaleString()}`}
                        {src.last_error && (
                          <>
                            {" · "}
                            <span className={styles.dangerText}>
                              {src.last_error}
                            </span>
                          </>
                        )}
                      </div>
                    </div>

                    <div className={styles.sourceActions}>
                      <label className="toggle">
                        <input
                          type="checkbox"
                          checked={src.enabled}
                          onChange={(e) =>
                            setSourceEnabled({
                              sourceId: src.source_id,
                              enabled: e.target.checked,
                            }).catch(() => {})
                          }
                          disabled={isLoading}
                        />
                        <span className="toggle-slider" />
                      </label>

                      <select
                        className={`input ${styles.badgeSm}`}
                        value={src.response}
                        onChange={(e) =>
                          setSourceResponse({
                            sourceId: src.source_id,
                            response: e.target.value as AdBlockResponse,
                          }).catch(() => {})
                        }
                        disabled={isLoading}
                      >
                        <option value="zero_address">0.0.0.0</option>
                        <option value="nx_domain">NXDOMAIN</option>
                      </select>

                      <button
                        className="btn btn-sm btn-ghost"
                        onClick={() =>
                          refreshSource(src.source_id).catch(() => {})
                        }
                        disabled={isLoading}
                        onPointerDown={onPointerDown(() => {})}
                      >
                        Refresh
                      </button>
                      <button
                        className="btn btn-sm btn-danger"
                        onClick={() => {
                          confirmDialog(
                            `Remove source "${src.name}"?`,
                            { title: "Remove Source", kind: "warning" },
                          ).then((ok) => {
                            if (ok) removeSource(src.source_id).catch(() => {});
                          }).catch(() => {});
                        }}
                        disabled={isLoading}
                        onPointerDown={onPointerDown(() => {})}
                      >
                        Delete
                      </button>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Whitelist */}
        <div className="card">
          <h2 className="card-title">Whitelist</h2>
          <p className={styles.muted}>
            Domains here are exempt from all ad block rules. Suffix-matched:
            adding <code>example.com</code> also exempts{" "}
            <code>api.example.com</code>.
          </p>

          <div className={`${styles.inlineForm} ${styles.sectionGap}`}>
            <input
              className="input"
              type="text"
              value={newWhitelistDomain}
              placeholder="trusted.example.com"
              onChange={(e) => setNewWhitelistDomain(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleAddWhitelist();
              }}
              disabled={isLoading}
            />
            <button
              className="btn btn-primary btn-sm"
              onClick={handleAddWhitelist}
              disabled={!newWhitelistDomain.trim() || isLoading}
              onPointerDown={onPointerDown(() => {})}
            >
              Add
            </button>
          </div>

          {state.whitelist.length === 0 ? (
            <div className={styles.empty}>No whitelist entries.</div>
          ) : (
            <div className={styles.whitelistList}>
              {state.whitelist.map((d) => (
                <span key={d} className={styles.whitelistItem}>
                  {d}
                  <button
                    className={styles.removeBtn}
                    onClick={() => removeWhitelist(d).catch(() => {})}
                    aria-label={`Remove ${d}`}
                    disabled={isLoading}
                  >
                    ×
                  </button>
                </span>
              ))}
            </div>
          )}
        </div>

        {/* Refresh interval */}
        <div className="card">
          <h2 className="card-title">Auto-refresh</h2>
          <p className={styles.muted}>
            Background refresh keeps sources up to date without manual
            intervention. Set to 0 to disable (refresh manually instead).
          </p>
          <div className={`${styles.inlineForm} ${styles.sectionGap}`}>
            <label className={`form-label ${styles.labelReset}`}>
              Every
            </label>
            <select
              className={`input ${styles.width120}`}
              value={state.refresh_interval_hours}
              onChange={(e) =>
                handleIntervalChange(parseInt(e.target.value, 10))
              }
              disabled={isLoading}
            >
              <option value="0">Manual only</option>
              <option value="1">1 hour</option>
              <option value="6">6 hours</option>
              <option value="12">12 hours</option>
              <option value="24">24 hours</option>
              <option value="48">2 days</option>
              <option value="168">1 week</option>
            </select>
          </div>
        </div>
      </div>
    </div>
  );
}

export default AdBlock;
