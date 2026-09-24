import { useCallback, useState, useEffect, useRef } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import {
  adBlockStateAtom,
  isAdBlockLoadingAtom,
  adBlockErrorAtom,
  adBlockRuleCountAtom,
  adBlockHasErrorsAtom,
  adBlockLimitsAtom,
  dnsEnabledAtom,
  fetchAdBlockStateAtom,
  fetchAdBlockLimitsAtom,
  adBlockStatsAtom,
  adBlockOverlapReportAtom,
  fetchAdBlockStatsAtom,
  fetchAdBlockOverlapsAtom,
  toggleAdBlockEnabledAtom,
  setAdBlockIntervalAtom,
  setAdBlockAutoRefreshEnabledAtom,
  addAdBlockSourceAtom,
  removeAdBlockSourceAtom,
  setAdBlockSourceEnabledAtom,
  setAdBlockSourceResponseAtom,
  setAdBlockSourceRulesLimitOverrideAtom,
  overrideAdBlockSourceRulesLimitAtom,
  refreshAdBlockSourceAtom,
  reorderAdBlockSourceAtom,
  refreshAllAdBlockSourcesAtom,
  addAdBlockWhitelistManyAtom,
  removeAdBlockWhitelistAtom,
} from "../stores/profiles";
import { useNavigate } from "react-router-dom";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import type { AdBlockResponse, BlocklistFormat } from "../types";
import styles from "./AdBlock.module.css";

// Issue #207: parse "source produced N rules (limit: M)" out of
// `last_error` so the card can offer a one-click override with the actual
// numbers. Must mirror the backend error format in
// `src-tauri/src/commands/adblock.rs::fetch_and_cache_source`.
const OVER_LIMIT_RE = /source produced (\d+) rules \(limit: (\d+)\)/;

function parseOverLimitError(lastError: string): { actual: number } | null {
  const match = OVER_LIMIT_RE.exec(lastError);
  if (!match) return null;
  const actual = parseInt(match[1], 10);
  if (Number.isNaN(actual) || actual <= 0) return null;
  return { actual };
}

function AdBlock() {
  const state = useAtomValue(adBlockStateAtom);
  const isLoading = useAtomValue(isAdBlockLoadingAtom);
  const error = useAtomValue(adBlockErrorAtom);
  const setError = useSetAtom(adBlockErrorAtom);
  const dnsEnabled = useAtomValue(dnsEnabledAtom);
  const ruleCount = useAtomValue(adBlockRuleCountAtom);
  const hasErrors = useAtomValue(adBlockHasErrorsAtom);
  const limits = useAtomValue(adBlockLimitsAtom);

  const fetchState = useSetAtom(fetchAdBlockStateAtom);
  const fetchLimits = useSetAtom(fetchAdBlockLimitsAtom);
  const stats = useAtomValue(adBlockStatsAtom);
  const overlapReport = useAtomValue(adBlockOverlapReportAtom);
  const fetchOverlaps = useSetAtom(fetchAdBlockOverlapsAtom);
  const fetchStats = useSetAtom(fetchAdBlockStatsAtom);
  const toggleEnabled = useSetAtom(toggleAdBlockEnabledAtom);
  const setInterval = useSetAtom(setAdBlockIntervalAtom);
  const setAutoRefresh = useSetAtom(setAdBlockAutoRefreshEnabledAtom);
  const addSource = useSetAtom(addAdBlockSourceAtom);
  const removeSource = useSetAtom(removeAdBlockSourceAtom);
  const setSourceEnabled = useSetAtom(setAdBlockSourceEnabledAtom);
  const setSourceResponse = useSetAtom(setAdBlockSourceResponseAtom);
  const overrideSourceLimit = useSetAtom(overrideAdBlockSourceRulesLimitAtom);
  const resetSourceLimit = useSetAtom(setAdBlockSourceRulesLimitOverrideAtom);
  const refreshSource = useSetAtom(refreshAdBlockSourceAtom);
  const reorderSource = useSetAtom(reorderAdBlockSourceAtom);
  const refreshAll = useSetAtom(refreshAllAdBlockSourcesAtom);
  const addWhitelistMany = useSetAtom(addAdBlockWhitelistManyAtom);
  const removeWhitelist = useSetAtom(removeAdBlockWhitelistAtom);

  const { onPointerDown } = useWebKitPointerDown();
  const navigate = useNavigate();

  // Local form state
  const [newName, setNewName] = useState("");
  const [newUrl, setNewUrl] = useState("");
  const [newResponse, setNewResponse] = useState<AdBlockResponse>("zero_address");
  // Issue #213: upstream blocklist format, chosen explicitly at add
  // time (the backend never sniffs). Defaults to hosts — the format
  // every source predating this feature uses.
  const [newFormat, setNewFormat] = useState<BlocklistFormat>("hosts");
  // Issue #215 §1: id of the source whose overlap drawer is
  // open, or null. The drawer reads from `overlapReport`.
  const [overlapDrawerSrcId, setOverlapDrawerSrcId] = useState<string | null>(null);
  const [newWhitelistDomain, setNewWhitelistDomain] = useState("");
  // Issue #196: ref so the bulk-add source button can read its value
  // without the previousElementSibling hack.
  const bulkSourceRef = useRef<HTMLTextAreaElement>(null);

  // Fetch on mount (idempotent — Tauri handles parallel calls). Limits
  // (issue #211-3) are static backend constants: fetched once, failure is
  // non-fatal (the override entry stays available and the backend still
  // rejects over-cap values itself).
  useEffect(() => {
    fetchState().catch(() => {
      /* error already in atom */
    });
    fetchLimits().catch(() => {});
    // Issue #199 sub-task B: pull the cumulative engine counters
    // for the stats panel. Non-fatal on failure — the panel
    // shows an "unknown" placeholder and the next interaction
    // retries.
    fetchStats().catch(() => {});
    // Issue #215: pull the cross-source overlap report so each
    // source card can render its chip without an extra round
    // trip when the user opens the drawer.
    fetchOverlaps().catch(() => {});
  }, [fetchState, fetchLimits, fetchStats, fetchOverlaps]);

  const handleAddSource = useCallback(() => {
    if (!newName.trim() || !newUrl.trim()) return;
    addSource({ name: newName.trim(), url: newUrl.trim(), response: newResponse, format: newFormat })
      .then(() => {
        setNewName("");
        setNewUrl("");
      })
      .catch(() => {
        /* error in atom */
      });
  }, [addSource, newName, newUrl, newResponse, newFormat]);

  /**
   * Issue #196: parse a multi-line paste into individual entries, drop
   * blanks/comments, and dispatch as a single batch IPC. The textarea
   * stays a single source of truth — a paste of 200 domains becomes one
   * `add_ad_block_whitelist_many` call instead of 200 single-entry
   * round-trips (each of which clears the DNS LRU response cache).
   */
  const handleAddWhitelist = useCallback(() => {
    const lines = newWhitelistDomain
      .split(/\r?\n/)
      .map((l) => l.trim())
      // Strip inline `# ...` comments (the parser does this for hosts
      // files; we mirror it so users can paste commented lists).
      .map((l) => l.replace(/#.*$/, "").trim())
      .filter(Boolean);
    if (lines.length === 0) return;
    addWhitelistMany(lines)
      .then(() => setNewWhitelistDomain(""))
      .catch(() => {
        /* error in atom */
      });
  }, [addWhitelistMany, newWhitelistDomain]);

  /**
   * Issue #196: copy the full whitelist to the clipboard for easy
   * backup / share. Reads from state so the user gets exactly what's
   * persisted (post-normalization), not whatever was last typed.
   */
  const handleCopyWhitelist = useCallback(() => {
    if (!state || state.whitelist.length === 0) return;
    void navigator.clipboard
      .writeText(state.whitelist.join("\n"))
      .catch(() => {
        // Self-review finding (PR #217): silently swallowing the
        // clipboard failure left the user staring at a button that did
        // nothing. Surface through the existing `adBlockErrorAtom`
        // toast channel — same UX as the bulk-add rejection toast.
        setError("Copy failed (clipboard unavailable)");
      });
  }, [state, setError]);

  /**
   * Issue #196: bulk-add sources from a `name<TAB>url` paste (one
   * source per line). Each line fires the existing single-entry IPC so
   * the backend can stream the per-source fetch + persist pipeline;
   * we don't bypass the IPC layer for this — it stays in user space
   * because source fetches are inherently slow (HTTP) and the user
   * already expects N sequential network calls when adding N sources.
   */
  const handleAddSourcesBulk = useCallback(
    (raw: string) => {
      const entries = raw
        .split(/\r?\n/)
        .map((l) => l.trim())
        .filter(Boolean)
        // Match-based parser: each regex is tried left-to-right and the
        // first one that matches wins, so the 2+space branch (which is
        // what `My List  https://…` uses) is correctly preferred over a
        // single-space split. Naively using String.split with an
        // alternation regex always splits at the first single-space
        // match, even when a longer 2+space match exists later in the
        // line — that bug ate multi-word source names.
        .map((line) => {
          // The branches are tried in priority order so the longest
          // separator (2+ spaces) wins over a single-space fallback —
          // a single space inside a multi-word name must NOT cut the
          // name. Anchoring with `\S` at the start ensures the name
          // begins at the first non-space char.
          const m2 =
            // 2+ spaces — the canonical "name  url" paste form.
            line.match(/^(\S(?:.*?\S)?)[ \t]{2,}(.+)$/) ??
            // Tab-separated.
            line.match(/^(\S(?:.*?\S)?)\t(.+)$/) ??
            // Single-space fallback — split at the first whitespace.
            line.match(/^(\S+)[ \t](.+)$/);
          if (!m2) return { name: "", url: "" };
          return { name: m2[1].trim(), url: m2[2].trim() };
        })
        .filter((e) => e.name && e.url);
      if (entries.length === 0) return;
      // Self-review finding (PR #217): the previous Promise.all with
      // `.catch(() => null)` swallowed every per-source failure and the
      // last `addSource` atom overwrote any earlier toast. Use
      // allSettled and surface failures as one summary toast so the
      // user sees the count and a sample of inputs that failed.
      void Promise.allSettled(
        entries.map((e) =>
          // Bulk-added lines share the form's current format selection
          // (issue #213) — a paste of domains-format URLs needs one
          // dropdown flip, not N re-adds.
          addSource({ name: e.name, url: e.url, response: newResponse, format: newFormat }),
        ),
      ).then((results) => {
        const failures = results
          .map((r, i) => (r.status === "rejected" ? entries[i] : null))
          .filter((e): e is { name: string; url: string } => e !== null);
        if (failures.length > 0) {
          const sample = failures
            .slice(0, 5)
            .map((f) => `${f.name} (${f.url})`)
            .join("; ");
          const suffix = failures.length > 5 ? "…" : "";
          setError(
            `${failures.length} of ${entries.length} sources failed to add: ${sample}${suffix}`,
          );
        }
      });
    },
    [addSource, newResponse, newFormat, setError],
  );

  const handleIntervalChange = useCallback(
    (hours: number) => {
      setInterval(hours).catch(() => {});
    },
    [setInterval],
  );

  const handleAutoRefreshToggle = useCallback(
    (enabled: boolean) => {
      setAutoRefresh(enabled).catch(() => {});
    },
    [setAutoRefresh],
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
            Blocklist subscription URLs — hosts format (0.0.0.0 domain) or
            plain domains (one per line), picked via the Format dropdown.
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
            {/* Issue #213: explicit upstream format. `hosts` is the
                default every pre-#213 source uses; `domains` covers
                anti-AD domains.txt / oisd-style one-domain-per-line
                lists. Not editable post-add — re-add the source to
                change it (mirrors the no-edit-source IPC contract). */}
            <div className="form-group">
              <label className="form-label">Format</label>
              <select
                className="input"
                value={newFormat}
                onChange={(e) =>
                  setNewFormat(e.target.value as BlocklistFormat)
                }
                disabled={isLoading}
              >
                <option value="hosts">hosts (0.0.0.0 …)</option>
                <option value="domains">domains (one per line)</option>
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

          {/* Issue #196: bulk paste — one source per line as
              `name<TAB>url` (tab, 2+ spaces, or single space all work).
              Each line still goes through the single-entry IPC so the
              backend's per-source fetch + persist pipeline is reused
              unchanged; this UI just saves the user N clicks. */}
          <details className={styles.bulkAdd}>
            <summary className={styles.bulkAddSummary}>
              Bulk add (one per line: <code>name&lt;TAB&gt;url</code>)
            </summary>
            <textarea
              ref={bulkSourceRef}
              className={`input ${styles.bulkTextarea ?? ""}`}
              rows={3}
              placeholder={"StevenBlack\thttps://example.com/hosts\nMy List  https://ml.com/hosts"}
              aria-label="Bulk add sources (name<TAB>url per line)"
              disabled={isLoading}
            />
            <button
              className="btn btn-sm"
              type="button"
              disabled={isLoading}
              onClick={() => {
                if (bulkSourceRef.current) handleAddSourcesBulk(bulkSourceRef.current.value);
              }}
              onPointerDown={onPointerDown(() => {})}
            >
              Add all
            </button>
          </details>

          {/* Source list */}
          {state.sources.length === 0 ? (
            <div className={styles.empty}>No sources yet.</div>
          ) : (
            <div className={styles.columnGap}>
              {state.sources.map((src, index) => {
                const overlapSummary = overlapReport?.per_source.find(
                  (s) => s.source_id === src.source_id,
                );
                return (
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
                        {` · ${src.format} format`}
                        {src.rules_limit_override != null &&
                          ` · limit ${src.rules_limit_override.toLocaleString()} (manually raised)`}
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


                      {/* Issue #215 §1: overlap chip — visible only when
                          this source shares at least one domain with
                          another enabled source. Clicking opens the
                          drill-down drawer at the bottom of the page.
                          The chip is intentionally outside the existing
                          `sourceMeta` line so it doesn't clutter the
                          status text for sources with zero overlaps. */}
                      {overlapSummary &&
                        overlapSummary.overlapping_domain_count > 0 && (
                          <button
                            type="button"
                            className={styles.overlapChip}
                            onClick={() =>
                              setOverlapDrawerSrcId(src.source_id)
                            }
                            aria-label={`Show ${overlapSummary.overlapping_domain_count} overlapping domains for ${src.name}`}
                            onPointerDown={onPointerDown(() => {})}
                          >
                            {overlapSummary.overlapping_domain_count.toLocaleString()}{" "}
                            domains also covered by other sources
                          </button>
                        )}
                      {/* Issue #207: one-click way out for legitimately huge
                          lists. The backend stays fail-closed (no truncation);
                          this raises the per-source cap to the actual parsed
                          count and retries through the normal refresh path.
                          Issue #211-3: the absolute-cap gate uses the
                          backend-delivered limits; while limits are unknown
                          the entry stays available — the backend remains the
                          authority and rejects over-cap overrides itself. */}
                      {(() => {
                        const over =
                          src.last_error != null
                            ? parseOverLimitError(src.last_error)
                            : null;
                        if (!over) return null;
                        if (
                          limits != null &&
                          over.actual > limits.rules_per_source_absolute_max
                        ) {
                          return null;
                        }
                        return (
                          <div className={styles.limitOverrideRow}>
                            <span className={styles.muted}>
                              This list has {over.actual.toLocaleString()}{" "}
                              rules — above the default cap.
                            </span>
                            <button
                              className="btn btn-sm btn-primary"
                              onClick={() =>
                                overrideSourceLimit({
                                  sourceId: src.source_id,
                                  limit: over.actual,
                                }).catch(() => {})
                              }
                              disabled={isLoading}
                              onPointerDown={onPointerDown(() => {})}
                            >
                              Allow {over.actual.toLocaleString()} rules &amp;
                              retry
                            </button>
                          </div>
                        );
                      })()}

                      {src.rules_limit_override != null && (
                        <div className={styles.limitOverrideRow}>
                          <button
                            className={`btn btn-sm btn-ghost ${styles.limitResetBtn}`}
                            onClick={() =>
                              resetSourceLimit({
                                sourceId: src.source_id,
                                limit: null,
                              }).catch(() => {})
                            }
                            disabled={isLoading}
                            onPointerDown={onPointerDown(() => {})}
                          >
                            Reset rule limit to default
                          </button>
                        </div>
                      )}
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


                      {/* Issue #215: source reorder. Two ↑/↓ buttons
                          (no dnd — see AGENTS.md / spec; the project
                          has no draggable primitive). The Up button
                          is disabled at the head and the Down
                          button at the tail, so the boundary no-op
                          case on the server can't be reached from
                          the UI. Order is purely a presentation
                          concern — never affects interception
                          (covered by the backend regression tests
                          in `commands::adblock::tests`). */}
                      <button
                        type="button"
                        className="btn btn-sm btn-ghost"
                        onClick={() =>
                          reorderSource({
                            sourceId: src.source_id,
                            direction: "up",
                          }).catch(() => {})
                        }
                        disabled={isLoading || index === 0}
                        aria-label={`Move source ${src.name} up`}
                        title="Move up"
                        onPointerDown={onPointerDown(() => {})}
                      >
                        ↑
                      </button>
                      <button
                        type="button"
                        className="btn btn-sm btn-ghost"
                        onClick={() =>
                          reorderSource({
                            sourceId: src.source_id,
                            direction: "down",
                          }).catch(() => {})
                        }
                        disabled={
                          isLoading || index === state.sources.length - 1
                        }
                        aria-label={`Move source ${src.name} down`}
                        title="Move down"
                        onPointerDown={onPointerDown(() => {})}
                      >
                        ↓
                      </button>
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
              );})}
            </div>
          )}
        </div>

        {/* Whitelist */}
        <div className="card">
          <div className={styles.whitelistHeader}>
            <h2 className="card-title">Whitelist</h2>
            <button
              className="btn btn-sm"
              onClick={handleCopyWhitelist}
              disabled={state.whitelist.length === 0}
              aria-label="Copy whitelist to clipboard"
              title="Copy whitelist to clipboard"
              onPointerDown={onPointerDown(() => {})}
            >
              Copy
            </button>
          </div>
          <p className={styles.muted}>
            Domains here are exempt from all ad block rules. Suffix-matched:
            adding <code>example.com</code> also exempts{" "}
            <code>api.example.com</code>. Paste multiple — one per line —
            and submit with the Add button or Cmd/Ctrl+Enter.
          </p>

          <div className={`${styles.inlineForm} ${styles.sectionGap}`}>
            <textarea
              className={`input ${styles.whitelistTextarea ?? ""}`}
              rows={4}
              value={newWhitelistDomain}
              placeholder="trusted.example.com"
              onChange={(e) => setNewWhitelistDomain(e.target.value)}
              onKeyDown={(e) => {
                // Cmd/Ctrl+Enter submits the whole batch. Plain Enter
                // stays as a line break so users can compose multi-line
                // pastes by hand. Issue #196.
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  handleAddWhitelist();
                }
              }}
              disabled={isLoading}
              aria-label="Whitelist domains (one per line, Cmd/Ctrl+Enter to submit)"
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

        {/* Auto-refresh toggle + interval */}
        <div className="card">
          <div className={styles.bannerText}>
            <div>
              <div className={styles.bannerTitle}>Auto-refresh</div>
              <div className={styles.muted}>
                Background refresh keeps sources up to date without manual
                intervention. Turn it off to refresh manually only.
              </div>
            </div>
            {/* #192: before this toggle, `auto_refresh_enabled` was a dead
                field — nothing could set it, and "Manual only" (interval=0)
                was the only way to disable auto-refresh. */}
            <label className="toggle">
              <input
                type="checkbox"
                aria-label="Auto-refresh"
                checked={state.auto_refresh_enabled}
                onChange={(e) => handleAutoRefreshToggle(e.target.checked)}
                disabled={isLoading}
              />
              <span className="toggle-slider" />
            </label>
          </div>
          {state.auto_refresh_enabled && (
            <div className={`${styles.inlineForm} ${styles.sectionGap}`}>
              <label className={`form-label ${styles.labelReset}`}>
                Every
              </label>
              <select
                className={`input ${styles.width120}`}
                aria-label="Refresh interval"
                value={state.refresh_interval_hours}
                onChange={(e) =>
                  handleIntervalChange(parseInt(e.target.value, 10))
                }
                disabled={isLoading}
              >
                {/* Legacy state from the pre-toggle UI ("Manual only" set
                    interval=0 while auto stayed true): without this
                    placeholder the select would misdisplay as "1 hour"
                    while nothing ever refreshes. */}
                {state.refresh_interval_hours === 0 && (
                  <option value="0" disabled>
                    Choose interval…
                  </option>
                )}
                <option value="1">1 hour</option>
                <option value="6">6 hours</option>
                <option value="12">12 hours</option>
                <option value="24">24 hours</option>
                <option value="48">2 days</option>
                <option value="168">1 week</option>
              </select>
            </div>
          )}
        </div>

        {/* Stats panel (issue #199 sub-task B): cumulative ad-block
            engine hits + misses + per-source refresh timing. Collapsed by
            default to keep the page quiet; users can open it when
            investigating blocked-traffic levels or refresh cadence. */}
        <details className="card">
          <summary className={styles.bannerTitle}>
            Ad-block stats
          </summary>
          <div className={styles.sectionGap}>
            {stats === null ? (
              <div className={styles.muted}>
                Stats not loaded yet — waiting for the first
                `getAdBlockStats` IPC. Counters will appear once the
                backend responds.
              </div>
            ) : !stats.enabled ? (
              <div className={styles.muted}>
                Master switch is off. The engine is parked, so no
                queries are being classified. Cumulative counters
                below reflect activity from when the switch was last
                on (they are not reset on toggle).
              </div>
            ) : (
              <div className={styles.muted}>
                Cumulative since process start. Toggle the master
                switch off and back on to keep the engine parked
                without losing history.
              </div>
            )}
            {stats !== null && (
              <div className={styles.statGrid}>
                <StatCounter
                  label="0.0.0.0 hits"
                  value={stats.hits_zero_addr}
                />
                <StatCounter
                  label="NXDOMAIN hits"
                  value={stats.hits_nxdomain}
                />
                <StatCounter
                  label="Whitelist hits"
                  value={stats.hits_whitelist}
                />
                <StatCounter label="Misses" value={stats.misses} />
              </div>
            )}
            {/* Per-source refresh timing. The list mirrors the
                source order on disk; disabled sources are still shown
                so the user can see when they last refreshed before
                being parked. */}
            {state.sources.length > 0 && stats !== null && (
              <table className={styles.statTable}>
                <thead>
                  <tr>
                    <th>Source</th>
                    {/* Issue #199 sub-task B: header renamed from
                        "Last refresh" to "Duration" — the
                        column shows milliseconds (issue #199
                        `last_refresh_duration_ms`), not a
                        timestamp. "Last failed" keeps the
                        timestamp shape. */}
                    <th>Duration</th>
                    <th>Last failed</th>
                  </tr>
                </thead>
                <tbody>
                  {state.sources.map((src) => (
                    <tr key={src.source_id}>
                      <td>{src.name}</td>
                      <td>
                        {src.last_refresh_duration_ms != null
                          ? `${src.last_refresh_duration_ms} ms`
                          : "—"}
                      </td>
                      <td>
                        {src.last_refresh_failed_at
                          ? new Date(src.last_refresh_failed_at).toLocaleString()
                          : "—"}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
        </details>

      {/* Issue #215 §1: overlap drill-down drawer. The drawer reads
          `overlapDrawerSrcId` and the cached `overlapReport.details`
          for that source. Closing clears the id; opening re-fetches
          the report so reorders / source mutations are reflected
          without a manual reload. The "No overlapping domains" empty
          state handles the case where the chip was clicked after a
          mutation cleared the overlap (rare race — `fetchOverlaps`
          is fired on every mutation but is async). */}
      {overlapDrawerSrcId !== null && (
        <>
          <div
            className={styles.overlapOverlay}
            onClick={() => setOverlapDrawerSrcId(null)}
          />
          <div
            role="dialog"
            aria-modal="true"
            aria-label="Cross-source overlap drill-down"
            className={styles.overlapModal}
          >
            <div className={styles.overlapModalHeader}>
              <h2 className={styles.overlapModalTitle}>
                Overlapping domains —{" "}
                {state.sources.find(
                  (s) => s.source_id === overlapDrawerSrcId,
                )?.name ?? "—"}
              </h2>
              <button
                className="btn btn-sm"
                onClick={() => setOverlapDrawerSrcId(null)}
                aria-label="Close overlap drawer"
                onPointerDown={onPointerDown(() => {})}
              >
                ×
              </button>
            </div>
            {(() => {
              const entries = overlapReport?.details[overlapDrawerSrcId] ?? [];
              if (entries.length === 0) {
                return (
                  <div className={styles.muted}>
                    No overlapping domains with other sources. (The chip
                    may have been clicked while a mutation was in
                    flight.)
                  </div>
                );
              }
              return (
                <div>
                  <div className={`${styles.muted} ${styles.mutedGap}`}>
                    {entries.length.toLocaleString()} domains are also
                    covered by at least one other enabled source. The
                    "effective" badge shows what the engine will return
                    for each — derived from the priority chain whitelist
                    &gt; NXDOMAIN &gt; 0.0.0.0.
                  </div>
                  {entries.map((entry) => (
                    <div key={entry.domain} className={styles.overlapEntry}>
                      <div>
                        <span className={styles.overlapEntryDomain}>
                          {entry.domain}
                        </span>
                        <span className={styles.overlapEffectiveBadge}>
                          {entry.effective}
                        </span>
                      </div>
                      <div className={styles.overlapEntryMeta}>
                        Also in:{" "}
                        {entry.covered_by
                          .map((s) => `${s.name} (${s.response})`)
                          .join(", ")}
                      </div>
                    </div>
                  ))}
                </div>
              );
            })()}
          </div>
        </>
      )}
      </div>
    </div>
  );
}

// Issue #199 sub-task B: small counter tile for the stats panel.
// Kept inline (not exported) because the layout is bespoke to this
// page; promoting it later is fine but premature now.
function StatCounter({
  label,
  value,
}: {
  label: string;
  value: number;
}) {
  return (
    <div className={styles.statCounter}>
      <div className={styles.statValue}>{value.toLocaleString()}</div>
      <div className={styles.statLabel}>{label}</div>
    </div>
  );
}

export default AdBlock;
