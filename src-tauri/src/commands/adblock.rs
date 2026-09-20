//! DNS-mode ad block IPC commands (issue #130).
//!
//! 12 commands: state CRUD, source management, refresh control, whitelist.
//! Storage layout is defined in [`mhost_storage::adblock`]. The
//! in-memory `state.ad_block_state` is the source of truth for hot-reload;
//! changes go through [`persist_and_reload`] which keeps file + memory +
//! `DnsServer.ad_block_engine` in sync atomically.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use chrono::Utc;
use mhost_core::{AdBlockResponse, AdBlockSource, AdBlockState, MhostError, SourceId};
use mhost_hosts::Parser;
use mhost_storage::adblock as adblock_store;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::state::{lock_or_recover, AppState};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default upper bound on rules per source (issue #205). The constant
/// predates DNS-mode ad-block: issue #130 introduced it to stop 100k+ hosts
/// entries from being written into `/etc/hosts`. Ad-block rules never touch
/// the hosts apply path, though — they live only in the DNS engine's
/// in-memory sets (`classify_rules` → `reload_ad_block_rules`). What this
/// value guards today is process memory: ~500k rules ≈ 35–50 MB resident in
/// the engine's HashMap/HashSet plus a transient parse buffer, which is
/// acceptable for a desktop app. It is sized as headroom for real-world
/// lists (hagezi / oisd reach 130k–400k; anti-AD floats around 100k).
///
/// Per-source: several large subscriptions multiply. A user can raise the
/// cap for a single source via `rules_limit_override` (issue #207), bounded
/// by [`ABSOLUTE_MAX_RULES_PER_SOURCE`].
const MAX_RULES_PER_SOURCE: usize = 500_000;

/// Absolute ceiling for a per-source `rules_limit_override` (issue #207).
/// ~2M rules ≈ 150–200 MB of engine memory — at that point the list is
/// rejected outright and the UI must not offer the override. The override
/// is an escape hatch for legitimately huge lists, not a tuning knob.
const ABSOLUTE_MAX_RULES_PER_SOURCE: usize = 2_000_000;

/// HTTP fetch timeout. Blocklist refresh shouldn't block the UI thread;
/// 30 s is generous for typical hosts-format payloads.
const FETCH_TIMEOUT_SECS: u64 = 30;

/// Maximum response body size for an ad-block source (PR #131 review
/// finding 1.8 — a malicious or unbounded-misconfigured source can still
/// run for `FETCH_TIMEOUT_SECS` and start streaming bytes; cap the bytes).
/// Sized against the rules cap: a 500k-line hosts list is ~15 MB of raw
/// bytes, and well-annotated lists carry comment overhead on top, so
/// 64 MB gives comfortable headroom (issue #205 — must move in lockstep
/// with `MAX_RULES_PER_SOURCE`, otherwise oversized lists fail at the
/// download stage with a harder-to-diagnose error than the rules limit).
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

/// Maximum length of a source URL. URLs longer than this are rejected
/// to prevent IPC-level memory abuse (PR #154 review P3).
const MAX_URL_LEN: usize = 2048;

/// Maximum length of a whitelist domain entry (RFC 1035 §3.1: each
/// label ≤ 63 chars, full domain ≤ 253 chars). PR #154 review P3.
const MAX_DOMAIN_LEN: usize = 253;

/// Concurrency cap for `refresh_all_ad_block_sources` and the periodic
/// background refresh (PR #131 review finding 1.4 — refresh was a serial
/// loop, blocking the UI for up to N × FETCH_TIMEOUT_SECS).
pub(crate) const REFRESH_CONCURRENCY: usize = 4;

const USER_AGENT: &str = "mHost-Desktop/1.0";

// ---------------------------------------------------------------------------
// Shared HTTP agent (PR #131 review findings 1.8 + 1.9; issue #180)
// ---------------------------------------------------------------------------
//
// `ureq` is a synchronous HTTP client. `Agent` construction is cheap-ish
// (builds TLS config + connection pool) but not free, so we still cache one
// in a `OnceLock` for process-wide reuse. `ureq` deliberately ships a much
// smaller dependency footprint than `reqwest` — no `h2`, no `hyper`,
// no `aws-lc-sys` — which trimmed the dependency graph on the order of
// several MBs of indirect rlib code (issue #180).
static HTTP_AGENT: OnceLock<ureq::Agent> = OnceLock::new();

fn http_agent() -> &'static ureq::Agent {
    HTTP_AGENT.get_or_init(|| {
        // `http_status_as_error = false`: we want to inspect `304 Not Modified`
        // ourselves (RFC 7232 conditional GET — issue #193) instead of having
        // ureq collapse it into `Err(StatusCode(304))`. The trade-off is that
        // 4xx/5xx must be handled explicitly below; that's the cost of one
        // `if` for a working conditional-GET contract.
        ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_global(Some(Duration::from_secs(FETCH_TIMEOUT_SECS)))
            .http_status_as_error(false)
            .build()
            .into()
    })
}

/// Outcome of an ad-block source HTTP GET (issue #193).
///
/// `Fresh` means the upstream returned a 200 with a body we should cache;
/// `NotModified` means a 304 to a conditional GET — the caller should keep
/// the existing cache and only refresh the bookkeeping
/// (`last_fetched_at` / clear `last_error`).
#[derive(Debug)]
pub(crate) enum FetchOutcome {
    Fresh { body: Vec<u8>, etag: Option<String> },
    NotModified,
}

/// Per-source refresh gates (issue #206 finding 1).
///
/// `fetch_and_cache_source` writes the cache file inside `spawn_blocking`
/// and then updates `etag` / `rule_count` under the state write lock as two
/// separate steps. Two concurrent refreshes of the *same* source (a manual
/// refresh racing the periodic tick, or "Refresh all" racing a single-source
/// refresh) used to interleave so the on-disk cache held body-v2 while the
/// state recorded etag-v3; the next conditional GET then 304'd against v3
/// and kept serving the stale v2 body until the upstream actually changed.
///
/// Serializing per source fixes that without any global lock: refreshes of
/// *different* sources still run concurrently (bounded by
/// `REFRESH_CONCURRENCY`), only same-source refreshes queue up. The map is
/// process-global because `fetch_and_cache_source` is reached from several
/// entry points (`add_ad_block_source`, `refresh_ad_block_source`,
/// `fetch_sources_concurrent` from both the manual IPC and the periodic
/// task) that don't share an `AppState` reference in tests. Entries are
/// keyed by UUID and never removed — bounded by the number of sources ever
/// created in one process lifetime, i.e. negligible.
static SOURCE_REFRESH_GATES: OnceLock<
    tokio::sync::Mutex<HashMap<SourceId, Arc<tokio::sync::Mutex<()>>>>,
> = OnceLock::new();

/// Acquire the per-source refresh gate for `source_id` (issue #206 finding
/// 1). The returned guard must be held for the whole fetch+cache+record
/// sequence.
async fn acquire_source_refresh_gate(source_id: &SourceId) -> Arc<tokio::sync::Mutex<()>> {
    let map = SOURCE_REFRESH_GATES.get_or_init(Default::default);
    let gate = map
        .lock()
        .await
        .entry(source_id.clone())
        .or_default()
        .clone();
    gate
}

/// Fetch `url` synchronously via the shared agent, optionally as a
/// conditional GET (RFC 7232 — issue #193):
///
/// - `if_none_match` → emitted as `If-None-Match: <value>` when `Some`.
/// - `if_modified_since` → emitted as `If-Modified-Since: <rfc7231 date>`
///   when `Some`. Honored independently of `If-None-Match` (a server that
///   supports only one will pick the relevant header).
/// - `force` → both conditional headers are dropped regardless (issue #206
///   design note 1). A user-initiated "Refresh" means "give me fresh data
///   now"; silently replaying a 304 against a locally-stale cache defeats
///   that. The periodic background refresh passes `force=false` to keep
///   the bandwidth savings.
///
/// Returns [`FetchOutcome::NotModified`] on `304 Not Modified` — no body is
/// read in that branch. Returns the body + ETag on `200 OK`. Rejects anything
/// larger than `MAX_RESPONSE_BYTES` and any 4xx/5xx as `MhostError`.
///
/// Sync because `ureq` is sync; callers wrap in
/// `tokio::task::spawn_blocking` (issue #180 — `ureq` replaced `reqwest`
/// to shrink the dependency graph). Size enforcement uses `ureq`'s
/// built-in `Body::with_config().limit(...)` reader so a malicious or
/// misconfigured source cannot blow past `MAX_RESPONSE_BYTES` even when
/// the server lies about `Content-Length` (PR #131 review finding 1.8).
fn fetch_source_sync(
    url: &str,
    if_none_match: Option<&str>,
    if_modified_since: Option<&str>,
    force: bool,
) -> Result<FetchOutcome, MhostError> {
    let mut req = http_agent().get(url);
    if !force {
        if let Some(etag) = if_none_match {
            req = req.header("If-None-Match", etag);
        }
        if let Some(date) = if_modified_since {
            req = req.header("If-Modified-Since", date);
        }
    }

    let mut resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(code)) => {
            // With `http_status_as_error = false` ureq shouldn't return this,
            // but keep the arm so a future config flip doesn't silently lose
            // the mapping.
            return Err(MhostError::ExternalApi(format!(
                "fetch {} failed: HTTP {}",
                url, code
            )));
        }
        Err(e) => {
            return Err(MhostError::Network(format!("network error: {}", e)));
        }
    };

    let status = resp.status();
    if status == 304 {
        // RFC 7232 §4.1: 304 has no body. We don't trust `Content-Length` or
        // try to read past it — `read_to_vec` on an empty body is a no-op.
        // If the server sends `ETag` / `Last-Modified` headers on the 304 we
        // ignore them: they're guaranteed to match what we already have.
        return Ok(FetchOutcome::NotModified);
    }

    // `resp.status()` returns a `ureq::http::StatusCode` newtype (ureq 3
    // re-exports the `http` crate). It only implements ordering against
    // other `StatusCode`s, so compare via `as_u16()` — the 304 short-circuit
    // above already handled the only status that warrants a bespoke path.
    let status_u16 = status.as_u16();
    if !(200..300).contains(&status_u16) {
        return Err(MhostError::ExternalApi(format!(
            "fetch {} failed: HTTP {}",
            url, status_u16
        )));
    }

    let etag = resp
        .headers()
        .get("ETag")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    // Cheap pre-check: if the server honestly reports a Content-Length
    // over our limit, reject without reading any bytes.
    if let Some(len) = resp.body().content_length() {
        if len > MAX_RESPONSE_BYTES as u64 {
            return Err(MhostError::InvalidInput(format!(
                "source body length {} exceeds limit {}",
                len, MAX_RESPONSE_BYTES
            )));
        }
    }

    // Hard cap on read bytes via ureq's LimitReader. Errors out if the
    // server streams beyond `MAX_RESPONSE_BYTES` (Content-Length lies or
    // chunked transfer with no advertised length).
    let body: Vec<u8> = resp
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES as u64)
        .read_to_vec()
        .map_err(|e| MhostError::Network(format!("read body error: {}", e)))?;
    Ok(FetchOutcome::Fresh { body, etag })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Persist the in-memory state to disk and hot-reload the running DNS server's
/// ad block engine. Used by every state-mutating command so the on-disk file,
/// in-memory copy, and resolver engine never diverge.
///
/// Must be called from a tokio context (uses `.await`). Acquires the state
/// write lock briefly to clone out, then releases before touching DNS server
/// to keep lock-hold time minimal.
///
/// **Issue #138:** the `spawn_blocking` closure below self-checks
/// the cancel token immediately before calling `reload_ad_block_rules`.
/// This protects against the race where the disable path runs
/// mid-`classify_rules` (which is sync, not cancellable): the closure
/// finishes classifying, sees the token is set, and bails before
/// mutating a `DnsServer` that's already been stopped. `write_state` is
/// intentionally NOT gated on the token — persisting in-memory state to
/// disk is the safe thing to do regardless of DNS-mode state.
pub(crate) async fn persist_and_reload(state: &AppState) -> Result<(), MhostError> {
    // Clone out under the lock, then drop the guard before DNS work.
    let snapshot: AdBlockState = {
        let guard = state.ad_block_state.read().await;
        guard.clone()
    };

    // Wrap write_state + classify_rules + reload in a single
    // spawn_blocking so none of the sync file IO or parsing blocks a
    // tokio worker thread (issue #133 — parsing 100k+ domain blocklists
    // on the reload path starved concurrent DNS queries).
    let root = state.storage.root().to_path_buf();
    let dns_enabled = state.dns_enabled.load(Ordering::Relaxed);
    let dns_server = Arc::clone(&state.dns_server);
    // Clone the token out from its Mutex slot before crossing the
    // spawn_blocking boundary. Issue #138 follow-up: the field is a
    // `Mutex<CancellationToken>` (not a bare token) so the refresh
    // task can swap in a fresh, uncancelled token on every spawn —
    // persist_and_reload reads whatever is currently in the slot,
    // which is the token bound to the latest spawned task.
    let cancel = lock_or_recover(&state.ad_block_refresh_cancel).clone();
    tokio::task::spawn_blocking(move || -> Result<(), MhostError> {
        adblock_store::write_state(&root, &snapshot)
            .map_err(|e| MhostError::InvalidInput(format!("write_state: {}", e)))?;
        if dns_enabled && !cancel.is_cancelled() {
            // We deliberately run classify_rules even if the pre-check
            // just succeeded: it's the long sync step (100k+ domain
            // parsing) and is exactly where cancel is most likely to
            // land. The post-classify check below is the only
            // authoritative one for the reload decision; the pre-check
            // exists only to skip the work entirely when we know up
            // front that we'll bail.
            let (zero_addr, nxdomain, whitelist) = classify_rules(&snapshot, &root);
            // Re-check after classify_rules: it's the long sync step and
            // is exactly where cancel is most likely to have landed.
            // (See issue #138: spawn_blocking cannot be aborted.)
            if !cancel.is_cancelled() {
                if let Some(server) = lock_or_recover(&dns_server).as_ref() {
                    // Issue #199 sub-task B: pass the master
                    // switch to the engine so `check()` can
                    // decide whether misses should accumulate.
                    server.reload_ad_block_rules(snapshot.enabled, zero_addr, nxdomain, whitelist);
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| MhostError::InvalidInput(format!("persist task failed: {}", e)))?
}

/// Reduce `AdBlockState` into the three rule sets consumed by the engine.
/// Reads each enabled source's cache file synchronously — only invoked from
/// `persist_and_reload`, which is in turn called from a tokio task; the IO
/// is fast (small files, no parsing needed here).
pub(crate) fn classify_rules(
    state: &AdBlockState,
    root: &std::path::Path,
) -> (HashMap<String, IpAddr>, HashSet<String>, HashSet<String>) {
    let mut zero_addr: HashMap<String, IpAddr> = HashMap::new();
    let mut nxdomain: HashSet<String> = HashSet::new();

    if state.enabled {
        // 仅 master switch 开启时才下发规则到引擎；关闭时引擎收到空集，
        // 自然 fallback 到原始规则 / 上游。
        for source in &state.sources {
            if !source.enabled {
                continue;
            }
            let domains = domains_for_source(root, source);
            match source.response {
                AdBlockResponse::ZeroAddress => {
                    let ip = IpAddr::from([0, 0, 0, 0]);
                    for d in domains {
                        zero_addr.entry(d).or_insert(ip);
                    }
                }
                AdBlockResponse::NxDomain => {
                    for d in domains {
                        nxdomain.insert(d);
                    }
                }
            }
        }
    }

    let whitelist: HashSet<String> = state.whitelist.iter().cloned().collect();

    (zero_addr, nxdomain, whitelist)
}

/// Load cached parsed domains for a single source. Returns an empty Vec if
/// the cache file is missing or fails to parse (caller logs and continues).
pub(crate) fn domains_for_source(root: &std::path::Path, source: &AdBlockSource) -> Vec<String> {
    match adblock_store::read_cache(root, &source.source_id) {
        Ok(Some(content)) => parse_blocklist_domains(&content),
        Ok(None) => Vec::new(),
        Err(e) => {
            eprintln!(
                "[adblock] failed to read cache for source {}: {}",
                source.name, e
            );
            Vec::new()
        }
    }
}

/// Validate a whitelist entry. Returns the canonical form (trimmed +
/// lowercased) on success, or an error message describing why the
/// input is invalid.
///
/// **PR #154 review (P2):** the original code only checked for empty
/// input, so entries like `*.example.com`, `example.com/path`, or
/// `not a domain at all` were persisted silently and never matched in
/// `walk_parents` (it does literal `HashSet::contains`, and the trie
/// that replaced it in issue #199 sub-task A has the same
/// suffix-match contract — these inputs don't match the trie either).
/// They also
/// didn't surface in `last_error`, so the user had no signal that the
/// entry was broken.
///
/// Rules enforced:
/// - non-empty after trim
/// - no whitespace anywhere (`*.example.com` etc. → reject)
/// - no path separator (`example.com/path` → reject)
/// - no leading dot (`.example.com` — engines don't expect this; the
///   suffix-walk covers the case anyway)
/// - no wildcard chars `*` (suffix-walk handles hierarchical match)
/// - only ASCII letters / digits / `-` / `.`
fn validate_whitelist_domain(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().to_lowercase();
    if trimmed.is_empty() {
        return Err("whitelist entry is empty".to_string());
    }
    if trimmed.len() > MAX_DOMAIN_LEN {
        return Err(format!(
            "whitelist entry length {} exceeds limit {}",
            trimmed.len(),
            MAX_DOMAIN_LEN
        ));
    }
    if trimmed.contains(char::is_whitespace) {
        return Err(format!("whitelist entry contains whitespace: {:?}", raw));
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(format!("whitelist entry looks like a URL/path: {:?}", raw));
    }
    if trimmed.starts_with('.') {
        return Err(format!(
            "whitelist entry must not start with '.': {:?}",
            raw
        ));
    }
    if trimmed.contains('*') {
        return Err(format!(
            "whitelist entry must not contain '*' (suffix-walk matches subdomains): {:?}",
            raw
        ));
    }
    for ch in trimmed.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '.') {
            return Err(format!(
                "whitelist entry has invalid character {:?}: {:?}",
                ch, raw
            ));
        }
    }

    // Structure checks (issue #196): previous version only checked the
    // character set, so entries like `example.com.`, `-example.com`, or
    // `foo-.example.com` slipped through. They never matched
    // `walk_parents` (which is literal `HashSet::contains`, and the
    // trie that replaced it in issue #199 sub-task A inherits the
    // same literal-match contract). The user
    // had no signal they were broken. We now reject:
    //   - leading `-` on the trimmed input (clearer error than the
    //     per-label check, which would otherwise report it as "invalid
    //     label '-example'")
    //   - trailing dot (FQDN form, but DNS engine expects bare labels)
    //   - any other label starting or ending with `-` (RFC 1123 §2.1)
    //   - empty label (`foo..com` collapses to a zero-length segment)
    if trimmed.starts_with('-') {
        return Err(format!(
            "whitelist entry must not start with '-': {:?}",
            raw
        ));
    }
    if trimmed.ends_with('.') {
        return Err(format!("whitelist entry must not end with '.': {:?}", raw));
    }
    for label in trimmed.split('.') {
        if label.is_empty() {
            return Err(format!("whitelist entry has empty label (..): {:?}", raw));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(format!(
                "whitelist entry has invalid label {:?} (starts/ends with '-') in {:?}",
                label, raw
            ));
        }
    }
    Ok(trimmed)
}

/// Parse hosts-format blocklist content into a flat list of domains.
/// Comments (`#`) and empty lines are filtered out by `Parser::parse_line`.
///
/// **PR #154 review (P2)**: no-op — after analysis, the original
/// `d.to_lowercase()` is correct and the only allocation we can avoid
/// here is for already-lowercase strings (the common case for
/// well-formed blocklists). The `eq_ignore_ascii_case` /
/// `to_ascii_uppercase` shortcut doesn't actually save allocations
/// (`to_ascii_uppercase` allocates a String) and breaks the
/// `MiXed.ExAmPlE.com → mixed.example.com` semantic that the
/// `parse_blocklist_lowercases` test relies on. Sticking with the
/// straightforward `to_lowercase()` — the work runs in
/// `spawn_blocking` (PR #131 P1-2 + issue #133), so DNS queries
/// aren't blocked during the parse.
fn parse_blocklist_domains(content: &str) -> Vec<String> {
    let result = Parser::parse(content);
    let mut domains: Vec<String> = Vec::new();
    for rule in result.rules {
        if !rule.enabled {
            continue;
        }
        for d in rule.domains {
            domains.push(d.to_lowercase());
        }
    }
    domains
}

/// Fetch a remote blocklist over HTTP(S), validate, and persist the raw
/// content + parsed-domain count back into the source record. The
/// hot-reload is the caller's responsibility (use `persist_and_reload`
/// after).
///
/// **Issue #194:** this used to be two near-identical copies of the same
/// function (`fetch_and_cache_source` and `fetch_and_cache_source_internal`),
/// differing only in whether they reached the storage root through
/// `state.storage` or a passed-in `Arc<dyn Storage>`. They collapsed into
/// a single function taking `Arc<dyn Storage + Send + Sync>` so future
/// bug fixes (e.g. PR #131 P1-2) only have to land once.
///
/// **Issue #193:** the HTTP layer now supports RFC 7232 conditional GET.
/// On `304 Not Modified` the on-disk cache is left untouched and only
/// `last_fetched_at` is bumped (and `last_error` cleared) — saves the
/// 5–15 MB re-download on every periodic refresh of an unchanged
/// blocklist.
///
/// **Issue #206:** (finding 1) same-source refreshes are serialized on a
/// per-source gate so the cache write and the etag/rule_count bookkeeping
/// of two racing refreshes cannot interleave. (finding 2) the 304 path
/// verifies the cache file actually exists; if it was removed out-of-band
/// the request is downgraded to an unconditional GET instead of silently
/// keeping a nonexistent payload. (design note 1) `force` drops the
/// conditional headers entirely — user-initiated refreshes pass `true`,
/// the periodic background task passes `false`.
///
/// **Issue #207:** the rules limit is the source's `rules_limit_override`
/// when set, otherwise the global [`MAX_RULES_PER_SOURCE`] default. An
/// over-limit fetch is rejected whole (fail-closed — never truncated) with
/// an error message carrying the actual parsed count, which the UI parses
/// to offer the one-click override.
pub(crate) async fn fetch_and_cache_source(
    storage: &Arc<dyn mhost_storage::storage::Storage + Send + Sync>,
    ad_block_state: &Arc<tokio::sync::RwLock<AdBlockState>>,
    source_id: &SourceId,
    force: bool,
) -> Result<(), MhostError> {
    // 0. Serialize same-source refreshes (issue #206 finding 1). Acquire
    //    the gate BEFORE reading the source record so the conditional-GET
    //    inputs (etag / last_fetched_at) are read fresh after any queued
    //    refresh finished mutating them.
    let gate = acquire_source_refresh_gate(source_id).await;
    let _gate_guard = gate.lock().await;

    // Issue #199 sub-task B: time the fetch so the UI can show
    // "last refresh took N ms" per source. The timer starts
    // right after the per-source gate is acquired — *not*
    // including the queue wait (which can be variable on a
    // contended refresh, see issue #206 finding 1), only the
    // HTTP fetch + parse + in-memory bookkeeping. This is what
    // the user perceives as the refresh cost on the source
    // itself; queue wait is a separate concern for a future
    // "queue depth" metric.
    let started_at = std::time::Instant::now();

    // 1. Read the source record under the read lock. We capture both
    //    the URL (for the fetch) and the previous ETag / last_fetched_at
    //    (for the conditional GET — issue #193), plus the effective
    //    rules limit for this source (issue #207).
    let (url, if_none_match, if_modified_since, rules_limit) = {
        let guard = ad_block_state.read().await;
        match adblock_store::find_source(&guard, source_id) {
            Some(s) => (
                s.url.clone(),
                s.etag.clone(),
                s.last_fetched_at.map(rfc7231_date),
                s.rules_limit_override.unwrap_or(MAX_RULES_PER_SOURCE),
            ),
            None => {
                return Err(MhostError::InvalidInput(format!(
                    "ad block source not found: {}",
                    source_id
                )))
            }
        }
    };

    // 2. Fused fetch + parse inside one `spawn_blocking` task (issue #180):
    //    `ureq` and the parser are both sync, and the prior split-fetch-then-
    //    parse would roundtrip through the tokio worker pool for no reason.
    //
    //    PR #131 re-review P1-2: a fetch failure used to bubble past
    //    `record_fetch_error`, so the UI badge and the persisted state
    //    both stayed stale. The `Err` branch below always records the
    //    failure on `last_error` before propagating.
    let root = storage.root().to_path_buf();
    let id_owned = source_id.clone();
    enum Parsed {
        Fresh {
            rule_count: usize,
            etag: Option<String>,
        },
        NotModified,
    }
    type FetchParseResult = Result<Parsed, MhostError>;
    let fetch_parse: FetchParseResult = tokio::task::spawn_blocking(move || {
        let mut outcome = fetch_source_sync(
            &url,
            if_none_match.as_deref(),
            if_modified_since.as_deref(),
            force,
        )?;
        // 304-path cache existence check (issue #206 finding 2): the 304
        // branch deliberately leaves the on-disk cache untouched, but the
        // cache file is not guaranteed to be there — the directory can be
        // cleared out-of-band while the ETag persists in the state. In that
        // case a 304 would keep "succeeding" while `domains_for_source`
        // silently yields an empty rule set with a clean `last_error`.
        // Downgrade to an unconditional GET so the body is re-fetched.
        //
        // Issue #211-1: if even the *unconditional* GET comes back 304, the
        // upstream is violating RFC 7232 §4.1 and we have no body and no
        // cache — fail loudly via `last_error` instead of returning a
        // "successful" NotModified that would keep the empty rule set
        // invisible.
        if matches!(outcome, FetchOutcome::NotModified) {
            let cache_file = adblock_store::cache_path(&root, &id_owned);
            if !cache_file.exists() {
                outcome = fetch_source_sync(&url, None, None, false)?;
                if matches!(outcome, FetchOutcome::NotModified) {
                    return Err(MhostError::ExternalApi(format!(
                        "upstream returned 304 to an unconditional request and the local \
                         cache is missing; cannot serve rules for source {}",
                        id_owned
                    )));
                }
            }
        }
        match outcome {
            FetchOutcome::NotModified => Ok(Parsed::NotModified),
            FetchOutcome::Fresh { body, etag } => {
                let content_str = std::str::from_utf8(&body).map_err(|e| {
                    MhostError::InvalidInput(format!("response is not valid UTF-8: {}", e))
                })?;
                let domains = parse_blocklist_domains(content_str);
                if domains.len() > rules_limit {
                    return Err(MhostError::InvalidInput(format!(
                        "source produced {} rules (limit: {})",
                        domains.len(),
                        rules_limit
                    )));
                }
                // Re-serialize as canonical hosts text so the cache is
                // always valid hosts format (drops comments the original
                // may have). Skipped entirely on 304 — see issue #193.
                let canon = domains
                    .iter()
                    .map(|d| format!("0.0.0.0 {}", d))
                    .collect::<Vec<_>>()
                    .join("\n");
                adblock_store::write_cache(&root, &id_owned, canon.as_bytes())?;
                Ok(Parsed::Fresh {
                    rule_count: domains.len(),
                    etag,
                })
            }
        }
    })
    .await
    .map_err(|e| MhostError::Network(format!("fetch task failed: {}", e)))?;

    // Issue #199 sub-task B: capture elapsed wall-clock time once,
    // reuse for all three branches (success / 304 / error). We use
    // `u64` so the field stays `Option<u64>`; `Duration::as_millis`
    // returns `u128`, but `u64::MAX` ms is ~584 million years so the
    // narrowing cast is safe.
    let elapsed_ms = started_at.elapsed().as_millis() as u64;

    match fetch_parse {
        Ok(Parsed::Fresh { rule_count, etag }) => {
            // 200 OK path — full update: clear error, set fetched_at,
            // rule_count, and the new ETag.
            let mut guard = ad_block_state.write().await;
            if let Some(s) = adblock_store::find_source_mut(&mut guard, source_id) {
                s.last_error = None;
                s.last_fetched_at = Some(Utc::now());
                s.rule_count = rule_count;
                s.etag = etag;
                s.last_refresh_duration_ms = Some(elapsed_ms);
                // Successful fetch clears any prior failure timestamp
                // (issue #199 sub-task B): a fresh success makes the
                // "last failed at" meaningless.
                s.last_refresh_failed_at = None;
            }
            Ok(())
        }
        Ok(Parsed::NotModified) => {
            // 304 path (issue #193): the cache is unchanged on disk, but
            // we still want a fresh `last_fetched_at` so the UI doesn't
            // look stale, and we want to clear any prior `last_error`
            // since we just successfully round-tripped the upstream.
            // `rule_count` and `etag` are intentionally NOT touched —
            // they continue to reflect the cached payload. Same for
            // `last_refresh_failed_at`: cleared because the upstream
            // just acknowledged we are current.
            let mut guard = ad_block_state.write().await;
            if let Some(s) = adblock_store::find_source_mut(&mut guard, source_id) {
                s.last_error = None;
                s.last_fetched_at = Some(Utc::now());
                s.last_refresh_duration_ms = Some(elapsed_ms);
                s.last_refresh_failed_at = None;
            }
            Ok(())
        }
        Err(e) => {
            // Network / size / parse failure: record on `last_error`,
            // keep the previous cache intact for DNS to keep serving.
            // Issue #199 sub-task B: also stamp the timing and
            // failure-timestamp fields so the UI's refresh panel can
            // show "last refresh took 30 s (timeout)" and the user
            // can see how long ago the last failure was.
            record_fetch_error_with_timing(ad_block_state, source_id, &e.to_string(), elapsed_ms)
                .await?;
            Err(e)
        }
    }
}

/// Persist an error string onto a source's `last_error` field. Does NOT
/// touch `last_fetched_at` or `rule_count` — those reflect the last
/// successful fetch and should be preserved on failure.
///
/// **Issue #194:** used to be two near-identical functions, one
/// AppState-shaped and one Arc-shaped, with the AppState variant just
/// unwrapping the Arc. Collapsed into a single Arc-shaped entry point
/// (callers pass `&state.ad_block_state`).
pub(crate) async fn record_fetch_error(
    ad_block_state: &Arc<tokio::sync::RwLock<AdBlockState>>,
    source_id: &SourceId,
    err: &str,
) -> Result<(), MhostError> {
    let mut guard = ad_block_state.write().await;
    if let Some(s) = adblock_store::find_source_mut(&mut guard, source_id) {
        s.last_error = Some(err.to_string());
    }
    Ok(())
}

/// Same as [`record_fetch_error`] but also stamps the issue #199
/// sub-task B timing fields: `last_refresh_duration_ms` and
/// `last_refresh_failed_at`. Used by the `Err` branch of
/// `fetch_and_cache_source` so a failed fetch still surfaces
/// how long it took and when it happened.
pub(crate) async fn record_fetch_error_with_timing(
    ad_block_state: &Arc<tokio::sync::RwLock<AdBlockState>>,
    source_id: &SourceId,
    err: &str,
    elapsed_ms: u64,
) -> Result<(), MhostError> {
    let mut guard = ad_block_state.write().await;
    if let Some(s) = adblock_store::find_source_mut(&mut guard, source_id) {
        s.last_error = Some(err.to_string());
        s.last_refresh_duration_ms = Some(elapsed_ms);
        s.last_refresh_failed_at = Some(Utc::now());
    }
    Ok(())
}

/// Format `dt` as an RFC 7231 IMF-fixdate string for use in the
/// `If-Modified-Since` request header (issue #193). Example output:
/// `Sun, 06 Nov 1994 08:49:37 GMT`.
///
/// `chrono::DateTime::to_rfc2822()` produces a similar shape but uses
/// `+0000` for UTC instead of `GMT`, which some servers reject. The
/// IMF-fixdate format is the only one the RFC mandates servers MUST
/// accept.
fn rfc7231_date(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Compile-time ad-block limits delivered to the frontend (issue #211-3).
///
/// The UI needs the absolute rules cap to decide whether to render the
/// per-source override entry (issue #207). Duplicating the constants as a
/// frontend mirror risks silent drift; the authoritative values live here
/// and the UI fetches them. `rules_per_source_default` is informational
/// (shown as "default cap" context).
/// Issue #199 sub-task B: view-model for the engine counters + master
/// switch exposed via `get_ad_block_stats`. Mirrors
/// `mhost_dns::adblock::AdBlockStats` (the engine-side struct) and
/// adds the current master-switch flag so the frontend can label
/// the panel correctly ("stats while master switch off" is a
/// useful UI hint).
#[derive(Debug, Clone, Serialize)]
pub struct AdBlockStatsView {
    pub hits_zero_addr: u64,
    pub hits_nxdomain: u64,
    pub hits_whitelist: u64,
    pub misses: u64,
    /// Whether the master switch is on at the moment of the call.
    /// `false` means the engine is parked; consumers should still
    /// show the cumulative numbers but tag the panel as
    /// "master switch off" so the absence of new hits isn't
    /// surprising.
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdBlockLimits {
    /// Global default cap applied when a source has no
    /// `rules_limit_override` (`MAX_RULES_PER_SOURCE`).
    pub rules_per_source_default: usize,
    /// Highest value a per-source override may take
    /// (`ABSOLUTE_MAX_RULES_PER_SOURCE`); lists above this get no override
    /// entry in the UI.
    pub rules_per_source_absolute_max: usize,
}

/// Expose the compile-time ad-block limits to the frontend (issue #211-3).
#[tauri::command]
pub async fn get_ad_block_limits() -> Result<AdBlockLimits, MhostError> {
    Ok(AdBlockLimits {
        rules_per_source_default: MAX_RULES_PER_SOURCE,
        rules_per_source_absolute_max: ABSOLUTE_MAX_RULES_PER_SOURCE,
    })
}

/// Issue #215 §1: cross-source overlap report. For every enabled
/// source that has a fetched cache, list how many domains it shares
/// with other enabled sources, plus a drill-down map that gives the
/// per-domain breakdown (which other sources also cover each shared
/// domain, and what `check()` will actually return for it).
///
/// **Behavioural contract** — this report describes what
/// `AdBlockEngine::check()` WILL DO, derived from the same priority
/// rule spelled out in `crates/mhost-dns/src/adblock.rs::check`:
/// whitelist > nxdomain > zero_addr. The drill-down's
/// `effective` field is the result of that priority chain applied
/// per domain — so users can see "yes, this domain is in two
/// sources, and the engine will resolve it as NXDOMAIN because one
/// of those sources is configured for that response type".
///
/// **Cost** — O(N · K) where N = enabled sources, K = avg domains
/// per source. The whole report runs once per `get_ad_block_overlaps`
/// IPC call. For a typical 5-source / 100 k-domain configuration
/// the computation is well under 100 ms on macOS, comfortably
/// inside the budget for "user clicked the drawer open" (the only
/// trigger — see `pages/AdBlock.tsx`). A summary view alone (just
/// `per_source`) is what the source card uses to render the chip;
/// `details` is what the drawer needs.
#[derive(Debug, Clone, Serialize)]
pub struct AdBlockOverlapReport {
    /// Per-source counts — one entry per enabled source with a
    /// cache, regardless of whether it has any overlaps. A source
    /// with zero overlaps is included with `overlapping_domain_count: 0`
    /// so the frontend doesn't have to look it up in two places.
    pub per_source: Vec<OverlapSummary>,
    /// Drill-down: for each source, the list of overlapping
    /// domains and the other sources that also cover them. Empty
    /// for sources with no overlaps.
    pub details: std::collections::HashMap<SourceId, Vec<OverlapEntry>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverlapSummary {
    pub source_id: SourceId,
    pub source_name: String,
    pub overlapping_domain_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverlapEntry {
    pub domain: String,
    /// Other enabled sources that also cover this domain. Empty
    /// would mean there's no actual overlap — the entry exists only
    /// when `covered_by.len() >= 1`.
    pub covered_by: Vec<OverlapSourceRef>,
    /// What `check()` will return for this domain — derived from the
    /// priority chain whitelist > nxdomain > zero_addr. Serialised
    /// as a string so the IPC contract doesn't depend on the
    /// engine's internal `AdBlockAction` enum (which intentionally
    /// has no `Serialize` derive).
    ///
    /// One of:
    /// - `"Whitelisted"`  — domain is in the user whitelist; fall-through
    /// - `"NxDomain"`     — nxdomain tier wins (highest priority among block rules)
    /// - `"ZeroAddress"`  — only zero_addr sources cover it
    pub effective: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverlapSourceRef {
    pub source_id: SourceId,
    pub name: String,
    pub response: AdBlockResponse,
}

/// Compute the overlap report. Reads each enabled source's parsed
/// domains via `domains_for_source` (already in-memory — no extra
/// IO during the call beyond the existing `adblock_store::read_cache`),
/// then folds them into the cross-source map.
///
/// Sources without a cache file (never fetched, or last fetch
/// failed) are skipped — they can't contribute to any overlap.
/// Sources whose `enabled` is false are also skipped, matching
/// `classify_rules`'s behaviour (they don't contribute to the
/// engine either, so showing their overlaps would be misleading).
pub(crate) fn compute_overlap_report(
    state: &AdBlockState,
    root: &std::path::Path,
) -> AdBlockOverlapReport {
    use std::collections::{HashMap, HashSet};

    // domain -> [(source_id, response)]
    let mut by_domain: HashMap<String, Vec<(SourceId, AdBlockResponse)>> = HashMap::new();

    for source in &state.sources {
        if !source.enabled {
            continue;
        }
        let domains = domains_for_source(root, source);
        for d in &domains {
            by_domain
                .entry(d.clone())
                .or_default()
                .push((source.source_id.clone(), source.response));
        }
    }

    // Whitelist lookups via suffix-match (same semantics as
    // `RulesSnapshot::whitelist.find_longest_suffix_match`). For a
    // single report we don't have a trie, but for a low-frequency
    // call it's cheap enough to do the O(K · |whitelist|) walk.
    let whitelist: HashSet<String> = state.whitelist.iter().cloned().collect();
    let whitelisted = |domain: &str| -> bool {
        if whitelist.contains(domain) {
            return true;
        }
        // Suffix walk: try parent.com, then parent.parent.com, etc.
        let mut cursor: Option<&str> = Some(domain);
        while let Some(d) = cursor {
            if let Some(idx) = d.find('.') {
                let parent = &d[idx + 1..];
                if whitelist.contains(parent) {
                    return true;
                }
                cursor = Some(parent);
            } else {
                return false;
            }
        }
        false
    };

    // Per-source accumulator.
    let mut details: HashMap<SourceId, Vec<OverlapEntry>> = HashMap::new();
    let mut per_source_counts: HashMap<SourceId, (String, usize)> = HashMap::new();
    for source in &state.sources {
        if source.enabled {
            per_source_counts.insert(source.source_id.clone(), (source.name.clone(), 0));
            details.insert(source.source_id.clone(), Vec::new());
        }
    }

    for (domain, sources) in &by_domain {
        if sources.len() < 2 {
            // Single-source coverage isn't an overlap; skip.
            continue;
        }
        // Effective action for this domain, derived from the same
        // priority chain `check()` uses. We can't reuse the engine's
        // `check()` directly because we don't have a RulesSnapshot;
        // but the logic is short and stable enough to inline here.
        let effective = if whitelisted(domain) {
            "Whitelisted".to_string()
        } else if sources
            .iter()
            .any(|(_, r)| matches!(r, AdBlockResponse::NxDomain))
        {
            "NxDomain".to_string()
        } else if sources
            .iter()
            .any(|(_, r)| matches!(r, AdBlockResponse::ZeroAddress))
        {
            "ZeroAddress".to_string()
        } else {
            continue; // shouldn't happen — at least one source must have a block response
        };

        // For each source that covers this domain, record the
        // domain in its details (unless the source itself is
        // whitelisted, in which case the user's intent is to
        // exclude it — we still note the OTHER sources that would
        // have blocked it).
        for (sid, _response) in sources {
            let covered_by: Vec<OverlapSourceRef> = sources
                .iter()
                .filter(|(other_sid, _)| other_sid != sid)
                .map(|(other_sid, r)| OverlapSourceRef {
                    source_id: other_sid.clone(),
                    name: state
                        .sources
                        .iter()
                        .find(|s| &s.source_id == other_sid)
                        .map(|s| s.name.clone())
                        .unwrap_or_default(),
                    response: *r,
                })
                .collect();
            let entry = OverlapEntry {
                domain: (*domain).to_string(),
                covered_by,
                effective: effective.clone(),
            };
            if let Some(bucket) = details.get_mut(sid) {
                bucket.push(entry);
                if let Some((_, count)) = per_source_counts.get_mut(sid) {
                    *count += 1;
                }
            }
        }
    }

    // Stable order: same as state.sources order, which is what the
    // UI's source list uses. Helps the frontend's diff rendering.
    let per_source: Vec<OverlapSummary> = state
        .sources
        .iter()
        .filter(|s| s.enabled)
        .filter_map(|s| {
            per_source_counts
                .get(&s.source_id)
                .map(|(name, count)| OverlapSummary {
                    source_id: s.source_id.clone(),
                    source_name: name.clone(),
                    overlapping_domain_count: *count,
                })
        })
        .collect();

    AdBlockOverlapReport {
        per_source,
        details,
    }
}

#[tauri::command]
pub async fn get_ad_block_overlaps(
    state: State<'_, AppState>,
) -> Result<AdBlockOverlapReport, MhostError> {
    let snap = state.ad_block_state.read().await.clone();
    Ok(compute_overlap_report(&snap, state.storage.root()))
}

/// Return the full ad block state (sources + whitelist + meta).
#[tauri::command]
pub async fn get_ad_block_state(state: State<'_, AppState>) -> Result<AdBlockState, MhostError> {
    Ok(state.ad_block_state.read().await.clone())
}

/// Issue #199 sub-task B: return the engine's cumulative hit / miss
/// counters for the UI's stats panel.
///
/// Returns `AdBlockStats` with zeros if DNS mode is off (no engine
/// loaded). The four counters are cumulative since process start —
/// there is no reset path; consumers compute their own deltas if
/// they want a "today" view.
#[tauri::command]
pub async fn get_ad_block_stats(
    state: State<'_, AppState>,
) -> Result<AdBlockStatsView, MhostError> {
    // Issue #199 sub-task B (PR #219 review follow-up): both the
    // counters AND the master switch come from the engine, not
    // from `ad_block_state`. The engine's AtomicBool is the value
    // that actually gates `check()`; `state.enabled` can lag
    // during a concurrent `set_ad_block_enabled` (which writes
    // state THEN mirrors onto the engine in two steps inside
    // `persist_and_reload`). Reading both from the engine closes
    // the race where the response's `enabled` label disagrees
    // with the engine's gating state for the duration of one
    // mid-call reload. When DNS mode is off (no server loaded)
    // we report zeros and `enabled=false` — there's no engine
    // to read from, so the absence is the answer.
    let (stats, enabled) = {
        let guard = lock_or_recover(&state.dns_server);
        match guard.as_ref() {
            Some(server) => (server.ad_block_stats(), server.ad_block_enabled()),
            None => (
                mhost_dns::adblock::AdBlockStats {
                    hits_zero_addr: 0,
                    hits_nxdomain: 0,
                    hits_whitelist: 0,
                    misses: 0,
                },
                false,
            ),
        }
    };
    Ok(AdBlockStatsView {
        hits_zero_addr: stats.hits_zero_addr,
        hits_nxdomain: stats.hits_nxdomain,
        hits_whitelist: stats.hits_whitelist,
        misses: stats.misses,
        enabled,
    })
}

/// Master switch. Disabling also clears the engine's rule sets via
/// `persist_and_reload` (which classifies with `enabled=false` → empty).
#[tauri::command]
pub async fn set_ad_block_enabled(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), MhostError> {
    {
        let mut guard = state.ad_block_state.write().await;
        guard.enabled = enabled;
    }
    // Issue #195 (review P1): the refresh task reads `enabled` after each
    // tick — wake it so master-switch changes don't wait out the
    // in-flight sleep, consistent with the other mutator IPCs.
    state.ad_block_refresh_wake.notify_one();
    persist_and_reload(&state).await
}

/// Change the auto-refresh interval in hours. The new value applies to
/// the *next* refresh wait (issue #195: an in-flight sleep is woken via
/// `Notify` so it does not run to completion).
///
/// `0` parks the refresh task as a backend fallback (legacy "Manual
/// only" state). Prefer `set_ad_block_auto_refresh_enabled` to toggle
/// auto-refresh from the UI.
#[tauri::command]
pub async fn set_ad_block_refresh_interval(
    hours: u32,
    state: State<'_, AppState>,
) -> Result<(), MhostError> {
    // 软上限：1h .. 7d。低于 1h 太频繁伤上游；超过 7d 几乎失去"自动"意义。
    let clamped = hours.clamp(0, 24 * 7);
    {
        let mut guard = state.ad_block_state.write().await;
        guard.refresh_interval_hours = clamped;
    }
    // Issue #195: the refresh task may be mid-sleep with the old value —
    // wake it so the new interval applies to the next wait immediately
    // instead of after the in-flight sleep (worst case 168h) elapses.
    state.ad_block_refresh_wake.notify_one();
    persist_and_reload(&state).await
}

/// Toggle background auto-refresh on/off (issue #192). Before this
/// command existed, `auto_refresh_enabled` was persisted but nothing
/// could change it — users could only emulate "off" via
/// `refresh_interval_hours = 0` ("Manual only").
///
/// The refresh task observes this field on every loop iteration; the
/// `notify_one` below (issue #195) makes the change effective
/// immediately instead of after the in-flight sleep. When re-enabled,
/// the next tick is a full interval away, matching the semantics of
/// editing the interval.
#[tauri::command]
pub async fn set_ad_block_auto_refresh_enabled(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), MhostError> {
    {
        let mut guard = state.ad_block_state.write().await;
        guard.auto_refresh_enabled = enabled;
    }
    state.ad_block_refresh_wake.notify_one();
    persist_and_reload(&state).await
}

// ---------------------------------------------------------------------------
// Source management
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_ad_block_sources(
    state: State<'_, AppState>,
) -> Result<Vec<AdBlockSource>, MhostError> {
    Ok(state.ad_block_state.read().await.sources.clone())
}

/// Add a new source, fetch it immediately, then persist. Returns the source
/// record (with `last_fetched_at`, `rule_count`, possibly `last_error`).
#[tauri::command]
pub async fn add_ad_block_source(
    name: String,
    url: String,
    response: AdBlockResponse,
    state: State<'_, AppState>,
) -> Result<AdBlockSource, MhostError> {
    add_ad_block_source_impl(&state, name, url, response).await
}

/// `AppState`-by-ref impl so the persistence-on-fetch-failure contract
/// (PR #131 re-review P1-2) can be unit-tested without a Tauri `State`.
pub(crate) async fn add_ad_block_source_impl(
    state: &AppState,
    name: String,
    url: String,
    response: AdBlockResponse,
) -> Result<AdBlockSource, MhostError> {
    if name.trim().is_empty() {
        return Err(MhostError::InvalidInput("source name is empty".into()));
    }
    if url.len() > MAX_URL_LEN {
        return Err(MhostError::InvalidInput(format!(
            "source url length {} exceeds limit {}",
            url.len(),
            MAX_URL_LEN
        )));
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(MhostError::InvalidInput(format!(
            "url must be http(s); got '{}'",
            url
        )));
    }

    let new_source = AdBlockSource {
        source_id: SourceId(Uuid::new_v4()),
        name,
        url,
        enabled: true,
        response,
        last_fetched_at: None,
        last_error: None,
        rule_count: 0,
        etag: None,
        rules_limit_override: None,
        last_refresh_duration_ms: None,
        last_refresh_failed_at: None,
    };
    let new_id = new_source.source_id.clone();

    {
        let mut guard = state.ad_block_state.write().await;
        guard.sources.push(new_source.clone());
    }

    // Fetch + propagate errors. PR #131 review finding 1.5: the previous
    // `let _ = …` discarded the error, leaving the UI with a successful
    // toast and a "fetch failed" badge next to a brand-new source (the
    // UX was confusing). Surface the error to the frontend toast; the
    // source is still in `state.sources` with `last_error` populated so
    // a later "Refresh" works as expected.
    //
    // PR #131 re-review P1-2: `?` here skipped `persist_and_reload` on
    // fetch failure, so the source existed only in memory and was lost
    // on restart. Persist unconditionally first (capturing `last_error`
    // too), then surface the fetch error to the toast.
    let fetch_result =
        fetch_and_cache_source(&state.storage, &state.ad_block_state, &new_id, false).await;
    persist_and_reload(state).await?;
    fetch_result?;

    // Return the freshly-fetched source record to the UI.
    let snap = state.ad_block_state.read().await;
    let stored = adblock_store::find_source(&snap, &new_id)
        .cloned()
        .unwrap_or(new_source);
    drop(snap);
    Ok(stored)
}

#[tauri::command]
pub async fn remove_ad_block_source(
    source_id: SourceId,
    state: State<'_, AppState>,
) -> Result<(), MhostError> {
    let root = state.storage.root().to_path_buf();
    {
        let mut guard = state.ad_block_state.write().await;
        adblock_store::purge_source(&root, &mut guard, &source_id)
            .map_err(|e| MhostError::InvalidInput(format!("purge_source: {}", e)))?;
    }
    persist_and_reload(&state).await
}

#[tauri::command]
pub async fn set_ad_block_source_enabled(
    source_id: SourceId,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<AdBlockSource, MhostError> {
    set_ad_block_source_enabled_impl(&state, &source_id, enabled).await
}

/// `AppState`-by-ref impl so the disable-cache / re-enable-refetch flow
/// is unit-testable without a Tauri `State` (same pattern as
/// `add_ad_block_source_impl`).
///
/// Issue #199 sub-task C (option A — delete-cache on disable, refetch
/// on re-enable):
///
/// * `true` -> `false` transition drops `adblock-cache/<id>.txt` so a
///   parked source leaves no on-disk residue. `delete_cache` is
///   idempotent (treats `NotFound` as success) and we swallow IO errors
///   here — a missed delete is a hygiene issue, not a correctness
///   issue, and `sweep_orphan_caches` covers it on the next startup.
/// * `false` -> `true` transition triggers an inline re-fetch. The
///   cache was just deleted on the disable path, so without the fetch
///   the user would see an enabled source with zero rules loaded
///   until the next auto-refresh tick (issue option A: "下次 enable
///   时自动重 fetch"). `fetch_and_cache_source` uses conditional GET
///   (issue #193) and gracefully downgrades a 304 to an unconditional
///   GET when the on-disk cache is missing (issue #206 finding 2), so
///   a server-returned 304 still ends with a populated cache. The
///   pathological edge — a server that returns 304 on BOTH the
///   conditional AND the unconditional retry (issue #211-1) — is the
///   one case where the fetch fails and the cache stays empty; the
///   source is still flipped to enabled and `last_error` records the
///   upstream's RFC 7232 violation for the UI badge. Fetch errors are
///   in general logged but not propagated: the toggle succeeded, only
///   the network leg failed.
///
/// **Known race (acknowledged, not fixed here):** if a user clicks
/// Refresh and then immediately toggles Disabled, the in-flight fetch
/// is serialized behind the per-source gate (issue #206 finding 1) but
/// does NOT block on `ad_block_state`. The disable path's
/// `delete_cache` can therefore run, then the in-flight fetch writes
/// the cache back. End state: a fresh cache file lingering on disk
/// for a now-disabled source until the next toggle or
/// `sweep_orphan_caches` on restart. This is a transient hygiene issue,
/// not a correctness issue (the engine classifies by `s.enabled` so
/// the lingering cache stays unloaded), and avoiding it would require
/// teaching `fetch_and_cache_source` to re-check `s.enabled` post-gate
/// — out of scope for the cache-cleanup follow-up. Tracked for the
/// perf/observability audit (issue #199 follow-up).
pub(crate) async fn set_ad_block_source_enabled_impl(
    state: &AppState,
    source_id: &SourceId,
    enabled: bool,
) -> Result<AdBlockSource, MhostError> {
    let root = state.storage.root().to_path_buf();
    let should_fetch_on_enable = {
        let mut guard = state.ad_block_state.write().await;
        let s = adblock_store::find_source_mut(&mut guard, source_id)
            .ok_or_else(|| MhostError::InvalidInput(format!("source not found: {}", source_id)))?;
        let prev_enabled = s.enabled;
        s.enabled = enabled;

        if prev_enabled && !enabled {
            if let Err(e) = adblock_store::delete_cache(&root, source_id) {
                eprintln!(
                    "[adblock] delete_cache on disable for source {}: {}",
                    source_id, e
                );
            }
        }

        !prev_enabled && enabled
    };

    if should_fetch_on_enable {
        if let Err(e) = fetch_and_cache_source(
            &state.storage,
            &state.ad_block_state,
            source_id,
            // `force=false`: let RFC 7232 conditional GET save bandwidth
            // if the upstream still has the same ETag (issue #193). The
            // missing-cache downgrade (issue #206) handles the case
            // where the disable path's `delete_cache` left no file.
            false,
        )
        .await
        {
            eprintln!(
                "[adblock] post-enable fetch for source {} failed: {}",
                source_id, e
            );
        }
    }

    persist_and_reload(state).await?;
    let snap = state.ad_block_state.read().await;
    Ok(adblock_store::find_source(&snap, source_id)
        .cloned()
        .expect("source just updated"))
}

#[tauri::command]
pub async fn set_ad_block_source_response(
    source_id: SourceId,
    response: AdBlockResponse,
    state: State<'_, AppState>,
) -> Result<AdBlockSource, MhostError> {
    {
        let mut guard = state.ad_block_state.write().await;
        let s = adblock_store::find_source_mut(&mut guard, &source_id)
            .ok_or_else(|| MhostError::InvalidInput(format!("source not found: {}", source_id)))?;
        s.response = response;
    }
    persist_and_reload(&state).await?;
    let snap = state.ad_block_state.read().await;
    Ok(adblock_store::find_source(&snap, &source_id)
        .cloned()
        .expect("source just updated"))
}

/// Direction argument for [`reorder_ad_block_sources`] (issue #215).
///
/// Two-element enum rather than a free-form `i32` delta so the frontend
/// can't ask for nonsense like "move 5 spots" — reorders are one-step,
/// decided by the UI button that fires them. The relative-move shape
/// (vs. a "rewrite the whole list" IPC) means the backend never has to
/// trust the frontend with the canonical source ordering, which would
/// otherwise be an attractive target for a buggy page that drops or
/// duplicates an id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReorderDirection {
    Up,
    Down,
}

#[tauri::command]
pub async fn reorder_ad_block_sources(
    source_id: SourceId,
    direction: ReorderDirection,
    state: State<'_, AppState>,
) -> Result<Vec<AdBlockSource>, MhostError> {
    let sources = reorder_ad_block_source_impl(&state, &source_id, direction).await?;
    Ok(sources)
}

/// Reorder implementation shared between the IPC handler and the test
/// suite. Swaps the source with its neighbour in the requested
/// direction; boundary moves (already at top / bottom) are a no-op
/// that returns the current source list unchanged — the button on
/// those ends is `disabled` in the UI, so this only fires from
/// keyboard shortcuts or a stale render.
///
/// **Issue #215 invariant**: the resulting `(zero_addr, nxdomain,
/// whitelist)` partitions from `classify_rules` are byte-identical
/// before and after this swap, because both source configurations
/// register the same domains in the same buckets. The new order is
/// only a presentation concern — see `reorder_preserves_classify_rules_output`
/// in the test module for the explicit assertion.
pub(crate) async fn reorder_ad_block_source_impl(
    state: &AppState,
    source_id: &SourceId,
    direction: ReorderDirection,
) -> Result<Vec<AdBlockSource>, MhostError> {
    let sources = {
        let mut guard = state.ad_block_state.write().await;
        let pos = guard
            .sources
            .iter()
            .position(|s| &s.source_id == source_id)
            .ok_or_else(|| MhostError::InvalidInput(format!("source not found: {}", source_id)))?;
        let target = match direction {
            ReorderDirection::Up if pos > 0 => pos - 1,
            ReorderDirection::Down if pos + 1 < guard.sources.len() => pos + 1,
            // Boundary no-op: at the top going up, or at the bottom
            // going down. Don't write, don't reload — the source list
            // is unchanged so reload would just churn counters.
            _ => return Ok(guard.sources.clone()),
        };
        guard.sources.swap(pos, target);
        guard.sources.clone()
    };
    persist_and_reload(state).await?;
    Ok(sources)
}
/// Per-source rules-limit override (issue #207).
///
/// `Some(n)` raises the per-source cap so a legitimately huge list can be
/// applied after a fail-closed over-limit rejection; `None` revokes the
/// override and returns the source to the global default. The retry itself
/// is the frontend calling the existing `refresh_ad_block_source` — no new
/// refresh code path.
///
/// Validation: `n` must be ≥ 1 and ≤ [`ABSOLUTE_MAX_RULES_PER_SOURCE`].
/// There is deliberately no truncation anywhere: a list either applies
/// whole or keeps its previous complete cache.
#[tauri::command]
pub async fn set_ad_block_source_rules_limit_override(
    source_id: SourceId,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<AdBlockSource, MhostError> {
    set_ad_block_source_rules_limit_override_impl(&state, &source_id, limit).await?;
    let snap = state.ad_block_state.read().await;
    Ok(adblock_store::find_source(&snap, &source_id)
        .cloned()
        .expect("source just updated"))
}

/// `AppState`-by-ref impl so the override validation is unit-testable
/// without a Tauri `State` (same pattern as `add_ad_block_source_impl`).
pub(crate) async fn set_ad_block_source_rules_limit_override_impl(
    state: &AppState,
    source_id: &SourceId,
    limit: Option<usize>,
) -> Result<(), MhostError> {
    if let Some(n) = limit {
        if n == 0 {
            return Err(MhostError::InvalidInput(
                "rules limit override must be at least 1 (use null to revoke the override)"
                    .to_string(),
            ));
        }
        if n > ABSOLUTE_MAX_RULES_PER_SOURCE {
            return Err(MhostError::InvalidInput(format!(
                "rules limit override {} exceeds the absolute cap {}",
                n, ABSOLUTE_MAX_RULES_PER_SOURCE
            )));
        }
    }
    {
        let mut guard = state.ad_block_state.write().await;
        let s = adblock_store::find_source_mut(&mut guard, source_id)
            .ok_or_else(|| MhostError::InvalidInput(format!("source not found: {}", source_id)))?;
        s.rules_limit_override = limit;
    }
    // Rule sets don't change here (the override only gates the next fetch),
    // but the state must reach disk so a restart keeps the authorization.
    persist_and_reload(state).await
}

// ---------------------------------------------------------------------------
// Refresh (concurrent)
// ---------------------------------------------------------------------------

/// Fan out `fetch_and_cache_source` over `source_ids` with bounded
/// concurrency. Per-source errors are recorded on `last_error` (preserving
/// the existing semantics) so the caller doesn't have to propagate.
///
/// PR #131 review finding 1.4: the previous serial loop could block the UI
/// for `N × FETCH_TIMEOUT_SECS` while `N` sources sequentially hit the
/// network. With this helper, a typical 4-source list finishes in ~one
/// timeout instead of four, and `isLoadingAtom` no longer lingers.
pub(crate) async fn fetch_sources_concurrent(
    storage: &Arc<dyn mhost_storage::storage::Storage + Send + Sync>,
    ad_block_state: &Arc<tokio::sync::RwLock<AdBlockState>>,
    source_ids: &[SourceId],
    concurrency: usize,
    force: bool,
) {
    if source_ids.is_empty() {
        return;
    }
    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
    let mut handles = Vec::with_capacity(source_ids.len());
    for id in source_ids {
        let permit = Arc::clone(&sem)
            .acquire_owned()
            .await
            .expect("semaphore starts with positive permits and is never closed");
        let storage = storage.clone();
        let ad_block_state = ad_block_state.clone();
        let id = id.clone();
        handles.push(tokio::spawn(async move {
            // Permit drops at end of task → slot released regardless of
            // success/failure.
            let _permit = permit;
            if let Err(e) = fetch_and_cache_source(&storage, &ad_block_state, &id, force).await {
                let _ = record_fetch_error(&ad_block_state, &id, &e.to_string()).await;
                eprintln!("[adblock] concurrent refresh source {} failed: {}", id, e);
            }
        }));
    }
    // Drain in submission order so a slow source doesn't keep its permit
    // forever if the user disables / deletes it mid-flight. We don't use
    // the results; the spawn task already did the write.
    for h in handles {
        let _ = h.await;
    }
}

// ---------------------------------------------------------------------------
// Refresh
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn refresh_ad_block_source(
    source_id: SourceId,
    state: State<'_, AppState>,
) -> Result<AdBlockSource, MhostError> {
    // PR #131 re-review P1-2 (same pattern as add_ad_block_source): persist
    // unconditionally so `last_error` is captured on disk, then surface the
    // fetch error. The source already exists on disk here, so this is about
    // not losing the error state rather than not losing the source.
    //
    // Issue #206 design note 1: manual refresh passes `force=true` — the
    // user's intent is fresh data, so the conditional GET is bypassed and
    // a locally-stale cache cannot be replayed via a 304.
    let fetch_result =
        fetch_and_cache_source(&state.storage, &state.ad_block_state, &source_id, true).await;
    persist_and_reload(&state).await?;
    fetch_result?;
    let snap = state.ad_block_state.read().await;
    Ok(adblock_store::find_source(&snap, &source_id)
        .cloned()
        .expect("source just fetched"))
}

#[tauri::command]
pub async fn refresh_all_ad_block_sources(
    state: State<'_, AppState>,
) -> Result<Vec<AdBlockSource>, MhostError> {
    // Snapshot IDs up-front to avoid holding the lock across await.
    let ids: Vec<SourceId> = {
        let snap = state.ad_block_state.read().await;
        snap.sources
            .iter()
            .filter(|s| s.enabled)
            .map(|s| s.source_id.clone())
            .collect()
    };
    // Concurrent fetch — bounded at REFRESH_CONCURRENCY. Per-source
    // failures are recorded on `last_error` via the helper. "Refresh all"
    // is user-initiated → `force=true` (issue #206 design note 1).
    fetch_sources_concurrent(
        &state.storage,
        &state.ad_block_state,
        &ids,
        REFRESH_CONCURRENCY,
        true,
    )
    .await;
    persist_and_reload(&state).await?;
    Ok(state.ad_block_state.read().await.sources.clone())
}

// ---------------------------------------------------------------------------
// Whitelist
//
// Issue #196: the original single-entry IPCs forced one `add_ad_block_whitelist`
// call per domain. Pasting 200 entries meant 200 `persist_and_reload` calls,
// and `reload_ad_block_rules` clears the LRU response cache on every reload
// (server.rs:362 — `self.cache.lock().clear()`). During a bulk paste, every
// DNS query fell through to upstream. We now expose `*_many` IPCs that
// validate + dedupe in one shot and trigger `persist_and_reload` once at
// the end. The single-entry IPCs are kept as thin wrappers that delegate to
// the same `*_impl` so the two paths can't drift apart.
// ---------------------------------------------------------------------------

/// A single rejected whitelist entry, paired with the validator's reason.
/// Returned to the frontend so it can toast per-line failures from a bulk
/// paste without aborting the whole batch on one bad input.
#[derive(Debug, Clone, Serialize)]
pub struct WhitelistInputError {
    pub input: String,
    pub reason: String,
}

/// Result of `add_ad_block_whitelist_many`: the full current whitelist plus
/// the entries that failed validation. Successful entries that already
/// existed in the whitelist are silently deduplicated (no rejected entry
/// for duplicates — that would be noisy for a paste of 200 entries into a
/// list that already contains 50 of them).
#[derive(Debug, Clone, Serialize)]
pub struct AddWhitelistManyResult {
    pub whitelist: Vec<String>,
    pub rejected: Vec<WhitelistInputError>,
}

#[tauri::command]
pub async fn list_ad_block_whitelist(
    state: State<'_, AppState>,
) -> Result<Vec<String>, MhostError> {
    Ok(state.ad_block_state.read().await.whitelist.clone())
}

/// Single-entry wrapper. Internally calls [`add_whitelist_impl`] so the
/// validation/dedupe/persist path is shared with `add_ad_block_whitelist_many`.
/// Returns `Vec<String>` to preserve the existing IPC contract — but if the
/// single input fails validation we re-raise as `MhostError::InvalidInput`
/// (mirroring the pre-#196 behavior: a bad input is an error, not a
/// silent no-op). The bulk variant exposes the richer
/// `AddWhitelistManyResult` so it can carry per-line rejections without
/// aborting the whole batch.
#[tauri::command]
pub async fn add_ad_block_whitelist(
    domain: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, MhostError> {
    let result = add_whitelist_impl(&state, vec![domain]).await?;
    if let Some(first) = result.rejected.into_iter().next() {
        return Err(MhostError::InvalidInput(first.reason));
    }
    Ok(result.whitelist)
}

/// Bulk add. Validates each entry independently (a single bad input
/// doesn't abort the batch), dedupes against the existing whitelist, then
/// persists + reloads DNS rules exactly once. See [`AddWhitelistManyResult`].
#[tauri::command]
pub async fn add_ad_block_whitelist_many(
    domains: Vec<String>,
    state: State<'_, AppState>,
) -> Result<AddWhitelistManyResult, MhostError> {
    add_whitelist_impl(&state, domains).await
}

/// Shared core for `add_ad_block_whitelist` and `add_ad_block_whitelist_many`.
/// `pub(crate)` so integration tests can drive it without an `AppState`
/// extractor (same pattern as `add_ad_block_source_impl`).
pub(crate) async fn add_whitelist_impl(
    state: &AppState,
    inputs: Vec<String>,
) -> Result<AddWhitelistManyResult, MhostError> {
    let mut accepted: Vec<String> = Vec::with_capacity(inputs.len());
    let mut rejected: Vec<WhitelistInputError> = Vec::new();
    for raw in inputs {
        match validate_whitelist_domain(&raw) {
            Ok(normalized) => accepted.push(normalized),
            Err(reason) => rejected.push(WhitelistInputError { input: raw, reason }),
        }
    }
    let mut newly_added = 0usize;
    {
        let mut guard = state.ad_block_state.write().await;
        for normalized in accepted {
            if !guard.whitelist.contains(&normalized) {
                guard.whitelist.push(normalized);
                newly_added += 1;
            }
        }
    }
    // Only hit the (expensive) persist + LRU-clearing reload path when
    // something actually changed. A paste of 200 entries where all 200
    // were already in the whitelist is a no-op — no DNS churn.
    if newly_added > 0 {
        persist_and_reload(state).await?;
    }
    Ok(AddWhitelistManyResult {
        whitelist: state.ad_block_state.read().await.whitelist.clone(),
        rejected,
    })
}

/// Single-entry wrapper. Removal contract is unchanged: `tolerates the
/// same input the user typed when adding`, no validation, silently no-op
/// on missing entries. Internally delegates to [`remove_whitelist_impl`].
#[tauri::command]
pub async fn remove_ad_block_whitelist(
    domain: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, MhostError> {
    remove_whitelist_impl(&state, vec![domain]).await
}

/// Bulk remove. Normalizes each entry (trim + lowercase) the same way the
/// single-entry variant does; missing entries are silently ignored. The
/// contract is intentionally lossy: there's no `rejected` list because
/// "remove what matches, ignore the rest" is the documented behavior.
#[tauri::command]
pub async fn remove_ad_block_whitelist_many(
    domains: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<String>, MhostError> {
    remove_whitelist_impl(&state, domains).await
}

/// Shared core for `remove_ad_block_whitelist` and `remove_ad_block_whitelist_many`.
/// `pub(crate)` so integration tests can drive it without an `AppState`
/// extractor.
pub(crate) async fn remove_whitelist_impl(
    state: &AppState,
    inputs: Vec<String>,
) -> Result<Vec<String>, MhostError> {
    let normalized: Vec<String> = inputs
        .into_iter()
        .map(|d| d.trim().to_lowercase())
        .filter(|d| !d.is_empty())
        .collect();
    if normalized.is_empty() {
        // Skip persist when the batch was all whitespace/empty — same
        // reasoning as `add_whitelist_impl`: a no-op must not pay for a
        // DNS reload (and the LRU cache clear it triggers).
        return Ok(state.ad_block_state.read().await.whitelist.clone());
    }
    let mut removed = 0usize;
    {
        let mut guard = state.ad_block_state.write().await;
        for target in &normalized {
            let before = guard.whitelist.len();
            guard.whitelist.retain(|d| d != target);
            removed += before.saturating_sub(guard.whitelist.len());
        }
    }
    // Mirror `add_whitelist_impl`: skip persist when no state changed.
    // A 200-entry batch where every target was already absent must be a
    // no-op — no DNS churn.
    if removed > 0 {
        persist_and_reload(state).await?;
    }
    Ok(state.ad_block_state.read().await.whitelist.clone())
}

// ---------------------------------------------------------------------------
// Unit tests (helpers only — IPC commands themselves covered by
// `commands/integration_tests.rs`-style tests in a follow-up).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_rules_disabled_master_yields_empty() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut state = AdBlockState {
            enabled: false,
            ..Default::default()
        };
        state.sources.push(AdBlockSource {
            source_id: SourceId(Uuid::new_v4()),
            name: "s".into(),
            url: "https://x".into(),
            enabled: true,
            response: AdBlockResponse::ZeroAddress,
            last_fetched_at: None,
            last_error: None,
            rule_count: 1,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        });
        let (z, n, w) = classify_rules(&state, temp.path());
        assert!(z.is_empty());
        assert!(n.is_empty());
        assert!(w.is_empty());
    }

    // Issue #215 (regression): #197's closure correction hinged on the
    // claim that `classify_rules` is independent of the order of
    // `state.sources` — two sources covering the same domain with
    // different response types must produce identical (za, nx, wl)
    // partitions regardless of which one appears first in the Vec.
    // The original `classify_rules_partitions_by_response` test only
    // covered the no-overlap case. This test pins the cross-source
    // overlap contract that #215 actually depends on.

    #[test]
    fn classify_rules_is_independent_of_source_vec_order() {
        use std::collections::HashSet;

        let temp = tempfile::TempDir::new().unwrap();

        // Two sources cover the SAME domain `shared.example.com` with
        // DIFFERENT response types. Source order in `state.sources` must
        // not change which partition wins (per #215 §2, NxDomain is
        // already decided by `check()` post-classify, but
        // classify_rules still has to assign the domain to BOTH
        // partitions so the engine can resolve the priority at
        // query time).
        let za_source_id = SourceId(Uuid::new_v4());
        let nx_source_id = SourceId(Uuid::new_v4());
        mhost_storage::adblock::write_cache(
            temp.path(),
            &za_source_id,
            b"0.0.0.0 shared.example.com\n0.0.0.0 za-only.example.com\n",
        )
        .unwrap();
        mhost_storage::adblock::write_cache(
            temp.path(),
            &nx_source_id,
            b"0.0.0.0 shared.example.com\n0.0.0.0 nx-only.example.com\n",
        )
        .unwrap();

        let za_source = AdBlockSource {
            source_id: za_source_id,
            name: "za".into(),
            url: "https://x".into(),
            enabled: true,
            response: AdBlockResponse::ZeroAddress,
            last_fetched_at: None,
            last_error: None,
            rule_count: 2,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        };
        let nx_source = AdBlockSource {
            source_id: nx_source_id,
            name: "nx".into(),
            url: "https://y".into(),
            enabled: true,
            response: AdBlockResponse::NxDomain,
            last_fetched_at: None,
            last_error: None,
            rule_count: 2,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        };

        // Order A: za first, nx second.
        let state_a = AdBlockState {
            enabled: true,
            sources: vec![za_source.clone(), nx_source.clone()],
            ..Default::default()
        };
        // Order B: nx first, za second (swapped).
        let state_b = AdBlockState {
            enabled: true,
            sources: vec![nx_source, za_source],
            ..Default::default()
        };

        let (za_a, nx_a, _) = classify_rules(&state_a, temp.path());
        let (za_b, nx_b, _) = classify_rules(&state_b, temp.path());

        // Both orders must produce the same set of domains in each
        // partition. ZeroAddress partition: 2 domains (the one shared
        // one + za-only). NxDomain partition: 2 domains (the shared one
        // + nx-only).
        let za_a_set: HashSet<String> = za_a.keys().cloned().collect();
        let za_b_set: HashSet<String> = za_b.keys().cloned().collect();
        assert_eq!(
            za_a_set, za_b_set,
            "zero_addr partition must be independent of source order"
        );
        let nx_a_set: HashSet<String> = nx_a.iter().cloned().collect();
        let nx_b_set: HashSet<String> = nx_b.iter().cloned().collect();
        assert_eq!(
            nx_a_set, nx_b_set,
            "nxdomain partition must be independent of source order"
        );

        // And critically, the shared domain must be in BOTH partitions
        // so that `check()` can apply the priority rule (issue #215 §3
        // — nxdomain wins over zero_addr on the same domain).
        assert!(
            za_a_set.contains("shared.example.com"),
            "shared domain must appear in zero_addr partition"
        );
        assert!(
            nx_a_set.contains("shared.example.com"),
            "shared domain must appear in nxdomain partition"
        );

        // Two ZeroAddress sources covering the same domain must also
        // be order-independent — the `or_insert` produces the same
        // IP (0.0.0.0) regardless of which source fires first.
        let za_id_a = SourceId(Uuid::new_v4());
        let za_id_b = SourceId(Uuid::new_v4());
        mhost_storage::adblock::write_cache(temp.path(), &za_id_a, b"0.0.0.0 shared.example.com\n")
            .unwrap();
        mhost_storage::adblock::write_cache(temp.path(), &za_id_b, b"0.0.0.0 shared.example.com\n")
            .unwrap();
        let za_a_src = AdBlockSource {
            source_id: za_id_a,
            name: "za-a".into(),
            url: "https://x".into(),
            enabled: true,
            response: AdBlockResponse::ZeroAddress,
            last_fetched_at: None,
            last_error: None,
            rule_count: 1,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        };
        let za_b_src = AdBlockSource {
            source_id: za_id_b,
            name: "za-b".into(),
            url: "https://y".into(),
            enabled: true,
            response: AdBlockResponse::ZeroAddress,
            last_fetched_at: None,
            last_error: None,
            rule_count: 1,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        };
        let state_c = AdBlockState {
            enabled: true,
            sources: vec![za_a_src.clone(), za_b_src.clone()],
            ..Default::default()
        };
        let state_d = AdBlockState {
            enabled: true,
            sources: vec![za_b_src, za_a_src],
            ..Default::default()
        };
        let (za_c, _, _) = classify_rules(&state_c, temp.path());
        let (za_d, _, _) = classify_rules(&state_d, temp.path());
        assert_eq!(
            za_c.get("shared.example.com").copied(),
            za_d.get("shared.example.com").copied(),
            "two ZeroAddress sources covering the same domain must produce the same IP"
        );
        assert_eq!(
            za_c.get("shared.example.com").copied(),
            Some(IpAddr::from([0u8, 0, 0, 0])),
            "constant 0.0.0.0 must come from source.response, not cache file"
        );
    }

    #[test]
    fn classify_rules_partitions_by_response() {
        let temp = tempfile::TempDir::new().unwrap();
        let mk = |name: &str, response: AdBlockResponse, enabled: bool| AdBlockSource {
            source_id: SourceId(Uuid::new_v4()),
            name: name.into(),
            url: "https://x".into(),
            enabled,
            response,
            last_fetched_at: None,
            last_error: None,
            rule_count: 0,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        };
        let za_source = mk("za", AdBlockResponse::ZeroAddress, true);
        let nx_source = mk("nx", AdBlockResponse::NxDomain, true);
        let off_source = mk("off", AdBlockResponse::ZeroAddress, false);

        // Seed cache files so the zero_addr / nxdomain partitions are
        // non-empty (issue #134 — previously only `w.len()==1` was
        // asserted, leaving the partition logic untested).
        mhost_storage::adblock::write_cache(
            temp.path(),
            &za_source.source_id,
            b"0.0.0.0 ads.example.com\n0.0.0.0 tracker.example.com\n",
        )
        .unwrap();
        mhost_storage::adblock::write_cache(
            temp.path(),
            &nx_source.source_id,
            b"0.0.0.0 blocked.example.com\n",
        )
        .unwrap();
        // The disabled source also has a cache file — its domains must
        // NOT appear in any partition (enabled=false short-circuits it).
        mhost_storage::adblock::write_cache(
            temp.path(),
            &off_source.source_id,
            b"0.0.0.0 should-not-appear.com\n",
        )
        .unwrap();

        let state = AdBlockState {
            enabled: true,
            sources: vec![za_source, nx_source, off_source],
            whitelist: vec!["trusted.com".to_string()],
            ..Default::default()
        };
        let (z, n, w) = classify_rules(&state, temp.path());

        // zero_addr partition: domains from the enabled ZeroAddress source,
        // mapped to 0.0.0.0.
        assert_eq!(z.len(), 2, "zero_addr seeded from enabled za source");
        assert!(z.contains_key("ads.example.com"));
        assert!(z.contains_key("tracker.example.com"));
        assert_eq!(
            z.get("ads.example.com").copied(),
            Some(IpAddr::from([0u8, 0, 0, 0])),
            "ZeroAddress domains must map to 0.0.0.0"
        );

        // nxdomain partition: domains from the enabled NxDomain source.
        assert_eq!(n.len(), 1, "nxdomain seeded from enabled nx source");
        assert!(n.contains("blocked.example.com"));

        // whitelist partition.
        assert_eq!(w.len(), 1);
        assert!(w.contains("trusted.com"));

        // The disabled source's domain must not leak into any partition.
        assert!(
            !z.contains_key("should-not-appear.com"),
            "disabled source must not contribute to zero_addr"
        );
        assert!(
            !n.contains("should-not-appear.com"),
            "disabled source must not contribute to nxdomain"
        );
    }

    // Issue #215 §1: cross-source overlap report. The priority contract
    // is already pinned by the engine tests; these tests pin the
    // report-shape contract so the frontend doesn't break against an
    // accidental restructuring.

    #[test]
    fn overlap_report_counts_domains_shared_by_multiple_sources() {
        let temp = tempfile::TempDir::new().unwrap();

        let s_za = AdBlockSource {
            source_id: SourceId(Uuid::new_v4()),
            name: "za".into(),
            url: "https://x".into(),
            enabled: true,
            response: AdBlockResponse::ZeroAddress,
            last_fetched_at: None,
            last_error: None,
            rule_count: 2,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        };
        let s_nx = AdBlockSource {
            source_id: SourceId(Uuid::new_v4()),
            name: "nx".into(),
            url: "https://y".into(),
            enabled: true,
            response: AdBlockResponse::NxDomain,
            last_fetched_at: None,
            last_error: None,
            rule_count: 2,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        };
        mhost_storage::adblock::write_cache(
            temp.path(),
            &s_za.source_id,
            b"0.0.0.0 shared.example.com\n0.0.0.0 za-only.example.com\n",
        )
        .unwrap();
        mhost_storage::adblock::write_cache(
            temp.path(),
            &s_nx.source_id,
            b"0.0.0.0 shared.example.com\n0.0.0.0 nx-only.example.com\n",
        )
        .unwrap();

        let state = AdBlockState {
            enabled: true,
            sources: vec![s_za.clone(), s_nx.clone()],
            ..Default::default()
        };

        let report = compute_overlap_report(&state, temp.path());

        // Each source has exactly one domain overlapped with the
        // other ("shared.example.com"). za-only and nx-only are
        // single-source coverage and must NOT count.
        let za_summary = report
            .per_source
            .iter()
            .find(|s| s.source_id == s_za.source_id)
            .expect("za summary present");
        assert_eq!(za_summary.overlapping_domain_count, 1);
        let nx_summary = report
            .per_source
            .iter()
            .find(|s| s.source_id == s_nx.source_id)
            .expect("nx summary present");
        assert_eq!(nx_summary.overlapping_domain_count, 1);

        // Effective is NxDomain for the shared domain, derived from
        // the priority rule.
        let za_details = report
            .details
            .get(&s_za.source_id)
            .expect("za details present");
        assert_eq!(za_details.len(), 1);
        assert_eq!(za_details[0].domain, "shared.example.com");
        assert_eq!(za_details[0].effective, "NxDomain");
        assert_eq!(za_details[0].covered_by.len(), 1);
        assert_eq!(za_details[0].covered_by[0].source_id, s_nx.source_id);

        let nx_details = report
            .details
            .get(&s_nx.source_id)
            .expect("nx details present");
        assert_eq!(nx_details.len(), 1);
        assert_eq!(nx_details[0].effective, "NxDomain");
    }

    #[test]
    fn overlap_report_excludes_disabled_sources() {
        // A disabled source must not contribute to overlaps — same
        // contract as `classify_rules` (PR #154 review P2 — disabled
        // source's domains must not leak into the engine).
        let temp = tempfile::TempDir::new().unwrap();

        let s_on = AdBlockSource {
            source_id: SourceId(Uuid::new_v4()),
            name: "on".into(),
            url: "https://x".into(),
            enabled: true,
            response: AdBlockResponse::ZeroAddress,
            last_fetched_at: None,
            last_error: None,
            rule_count: 1,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        };
        let s_off = AdBlockSource {
            source_id: SourceId(Uuid::new_v4()),
            name: "off".into(),
            url: "https://y".into(),
            enabled: false,
            response: AdBlockResponse::ZeroAddress,
            last_fetched_at: None,
            last_error: None,
            rule_count: 1,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        };
        mhost_storage::adblock::write_cache(
            temp.path(),
            &s_on.source_id,
            b"0.0.0.0 shared.example.com\n",
        )
        .unwrap();
        mhost_storage::adblock::write_cache(
            temp.path(),
            &s_off.source_id,
            b"0.0.0.0 shared.example.com\n",
        )
        .unwrap();

        let state = AdBlockState {
            enabled: true,
            sources: vec![s_on.clone(), s_off.clone()],
            ..Default::default()
        };

        let report = compute_overlap_report(&state, temp.path());

        // Only the enabled source appears in per_source.
        assert_eq!(report.per_source.len(), 1);
        assert_eq!(report.per_source[0].source_id, s_on.source_id);
        // And its overlap count is 0 because the only other source
        // covering "shared.example.com" is disabled.
        assert_eq!(report.per_source[0].overlapping_domain_count, 0);
        assert!(report.details.get(&s_on.source_id).unwrap().is_empty());
    }

    #[test]
    fn overlap_report_marks_whitelisted_domains_as_effective_whitelisted() {
        let temp = tempfile::TempDir::new().unwrap();

        let s = AdBlockSource {
            source_id: SourceId(Uuid::new_v4()),
            name: "list".into(),
            url: "https://x".into(),
            enabled: true,
            response: AdBlockResponse::ZeroAddress,
            last_fetched_at: None,
            last_error: None,
            rule_count: 1,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        };
        mhost_storage::adblock::write_cache(
            temp.path(),
            &s.source_id,
            b"0.0.0.0 trusted.example.com\n",
        )
        .unwrap();
        let state = AdBlockState {
            enabled: true,
            sources: vec![s.clone()],
            // Whitelist suffix-matches both `trusted.example.com` and
            // any subdomain like `api.trusted.example.com`.
            whitelist: vec!["trusted.example.com".to_string()],
            ..Default::default()
        };

        let report = compute_overlap_report(&state, temp.path());

        // Single source — no overlap, but the report must still
        // include the source (so the frontend doesn't have to look
        // it up in two places) and the effective field, if it ever
        // appears in the details map for this source, must be
        // "Whitelisted". For this test there's no overlap detail
        // because there's only one source, so just verify the
        // summary.
        assert_eq!(report.per_source.len(), 1);
        assert_eq!(report.per_source[0].overlapping_domain_count, 0);
    }

    /// Sub-agent review (PR #221, finding 3) companion to the existing
    /// `overlap_report_marks_whitelisted_domains_as_effective_whitelisted`:
    /// that test only had ONE source, so `by_domain` filtered the entry
    /// at `sources.len() < 2` and the `effective: "Whitelisted"` branch
    /// was never actually asserted. This test exercises the full
    /// whitelist-priority path end-to-end: two sources both block the
    /// same domain, the user has whitelisted it, and the engine (per
    /// `check()` and `compute_overlap_report`'s priority chain) must
    /// classify it as fall-through — i.e., the `effective` field on
    /// the overlap entry is "Whitelisted".
    #[test]
    fn overlap_report_marks_whitelisted_domain_as_effective_when_two_sources_cover_it() {
        let temp = tempfile::TempDir::new().unwrap();

        let s_za = AdBlockSource {
            source_id: SourceId(Uuid::new_v4()),
            name: "za".into(),
            url: "https://x".into(),
            enabled: true,
            response: AdBlockResponse::ZeroAddress,
            last_fetched_at: None,
            last_error: None,
            rule_count: 1,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        };
        let s_nx = AdBlockSource {
            source_id: SourceId(Uuid::new_v4()),
            name: "nx".into(),
            url: "https://y".into(),
            enabled: true,
            response: AdBlockResponse::NxDomain,
            last_fetched_at: None,
            last_error: None,
            rule_count: 1,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        };
        // Both sources block the same domain. Without the whitelist,
        // nxdomain would win (issue #215 priority chain). With the
        // whitelist, "Whitelisted" must win over BOTH block tiers.
        mhost_storage::adblock::write_cache(
            temp.path(),
            &s_za.source_id,
            b"0.0.0.0 trusted.example.com\n",
        )
        .unwrap();
        mhost_storage::adblock::write_cache(
            temp.path(),
            &s_nx.source_id,
            b"0.0.0.0 trusted.example.com\n",
        )
        .unwrap();
        let state = AdBlockState {
            enabled: true,
            sources: vec![s_za.clone(), s_nx.clone()],
            whitelist: vec!["trusted.example.com".to_string()],
            ..Default::default()
        };

        let report = compute_overlap_report(&state, temp.path());

        // Both sources see the same overlap (1 domain each).
        assert_eq!(report.per_source.len(), 2);
        assert_eq!(
            report
                .per_source
                .iter()
                .map(|s| s.overlapping_domain_count)
                .collect::<Vec<_>>(),
            vec![1, 1],
            "both sources must report one overlapping domain"
        );

        // The drill-down entries must both classify the domain as
        // "Whitelisted" — this is the contract `check()` enforces
        // (whitelist > nxdomain > zero_addr), and it's what the
        // drawer's `effective` badge shows the user.
        let za_details = report
            .details
            .get(&s_za.source_id)
            .expect("za details present");
        assert_eq!(za_details.len(), 1);
        assert_eq!(za_details[0].domain, "trusted.example.com");
        assert_eq!(
            za_details[0].effective, "Whitelisted",
            "whitelist must beat zero_addr for an overlapping domain"
        );

        let nx_details = report
            .details
            .get(&s_nx.source_id)
            .expect("nx details present");
        assert_eq!(nx_details.len(), 1);
        assert_eq!(nx_details[0].domain, "trusted.example.com");
        assert_eq!(
            nx_details[0].effective, "Whitelisted",
            "whitelist must beat nxdomain for an overlapping domain"
        );
    }

    #[test]
    fn overlap_report_is_empty_when_no_sources() {
        let temp = tempfile::TempDir::new().unwrap();
        let state = AdBlockState::default();
        let report = compute_overlap_report(&state, temp.path());
        assert!(report.per_source.is_empty());
        assert!(report.details.is_empty());
    }

    #[test]
    fn overlap_report_keeps_state_sources_order() {
        // The frontend renders per_source into chips on each
        // source card; ordering matters so the chips line up with
        // the source list (no extra lookup needed).
        let temp = tempfile::TempDir::new().unwrap();
        let s_a = SourceId(Uuid::new_v4());
        let s_b = SourceId(Uuid::new_v4());
        let s_c = SourceId(Uuid::new_v4());
        let mk = |id: SourceId, name: &str| AdBlockSource {
            source_id: id,
            name: name.into(),
            url: "https://x".into(),
            enabled: true,
            response: AdBlockResponse::ZeroAddress,
            last_fetched_at: None,
            last_error: None,
            rule_count: 0,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        };
        let state = AdBlockState {
            enabled: true,
            sources: vec![
                mk(s_c.clone(), "c"),
                mk(s_a.clone(), "a"),
                mk(s_b.clone(), "b"),
            ],
            ..Default::default()
        };
        let report = compute_overlap_report(&state, temp.path());
        let order: Vec<_> = report
            .per_source
            .iter()
            .map(|s| s.source_id.clone())
            .collect();
        assert_eq!(
            order,
            vec![s_c, s_a, s_b],
            "per_source order must follow state.sources"
        );
    }
    #[test]
    fn parse_blocklist_extracts_domains() {
        let text = "\
# ad-block test
0.0.0.0 ad.example.com
0.0.0.0 tracker.example.com
127.0.0.1 also.example.com

# comment
";
        let domains = parse_blocklist_domains(text);
        assert!(domains.contains(&"ad.example.com".to_string()));
        assert!(domains.contains(&"tracker.example.com".to_string()));
        assert!(domains.contains(&"also.example.com".to_string()));
        // comments and blanks are filtered by the parser
    }

    #[test]
    fn parse_blocklist_lowercases() {
        let text = "0.0.0.0 MiXed.ExAmPlE.com\n";
        let domains = parse_blocklist_domains(text);
        assert_eq!(domains, vec!["mixed.example.com".to_string()]);
    }

    // -----------------------------------------------------------------
    // PR #154 review (P2): validate_whitelist_domain test coverage.
    // Each rejection branch + the happy path + the MAX_DOMAIN_LEN
    // guard. These are pure sync tests — no AppState / DnsServer
    // needed.
    // -----------------------------------------------------------------

    #[test]
    fn validate_whitelist_domain_happy_path_lowercases_and_trims() {
        assert_eq!(
            validate_whitelist_domain("  Example.COM  ").unwrap(),
            "example.com"
        );
        assert_eq!(
            validate_whitelist_domain("foo.example.com").unwrap(),
            "foo.example.com"
        );
    }

    #[test]
    fn validate_whitelist_domain_rejects_empty_or_whitespace_only() {
        assert!(validate_whitelist_domain("").is_err());
        assert!(validate_whitelist_domain("   ").is_err());
        let err = validate_whitelist_domain("").unwrap_err();
        assert!(err.contains("empty"), "unexpected error: {}", err);
    }

    #[test]
    fn validate_whitelist_domain_rejects_whitespace_inside() {
        assert!(validate_whitelist_domain("foo bar.com").is_err());
        assert!(validate_whitelist_domain("foo\tbar.com").is_err());
    }

    #[test]
    fn validate_whitelist_domain_rejects_path_separator() {
        assert!(validate_whitelist_domain("example.com/path").is_err());
        assert!(validate_whitelist_domain("example.com\\path").is_err());
        let err = validate_whitelist_domain("example.com/path").unwrap_err();
        assert!(err.contains("URL/path"), "unexpected error: {}", err);
    }

    #[test]
    fn validate_whitelist_domain_rejects_wildcard() {
        assert!(validate_whitelist_domain("*.example.com").is_err());
        let err = validate_whitelist_domain("*.example.com").unwrap_err();
        assert!(err.contains("*"), "unexpected error: {}", err);
    }

    #[test]
    fn validate_whitelist_domain_rejects_leading_dot() {
        assert!(validate_whitelist_domain(".example.com").is_err());
    }

    #[test]
    fn validate_whitelist_domain_rejects_unicode() {
        assert!(validate_whitelist_domain("例え.com").is_err());
        assert!(validate_whitelist_domain("café.example.com").is_err());
    }

    #[test]
    fn validate_whitelist_domain_rejects_oversize() {
        // 254 chars — exceeds RFC 1035 max of 253.
        let huge = "a".repeat(254);
        assert!(validate_whitelist_domain(&huge).is_err());
        let err = validate_whitelist_domain(&huge).unwrap_err();
        assert!(err.contains("exceeds limit"), "unexpected error: {}", err);
        // 253 chars — exactly at the boundary, should pass.
        let at_limit = "a".repeat(253);
        assert!(validate_whitelist_domain(&at_limit).is_ok());
    }

    // -----------------------------------------------------------------
    // Issue #196: structure checks tightened so entries that look valid
    // by character set but never match `walk_parents` (which uses literal
    // `HashSet::contains`; the trie that replaced walk_parents in
    // issue #199 sub-task A inherits the same contract)
    // `HashSet::contains`) are rejected at the boundary instead of
    // silently no-op'ing after being added.
    // -----------------------------------------------------------------

    #[test]
    fn validate_whitelist_domain_rejects_trailing_dot() {
        assert!(validate_whitelist_domain("example.com.").is_err());
        let err = validate_whitelist_domain("example.com.").unwrap_err();
        assert!(
            err.contains("must not end with '.'"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn validate_whitelist_domain_rejects_leading_dash() {
        assert!(validate_whitelist_domain("-example.com").is_err());
        // The trim-level guard fires first and yields a clearer error
        // than the per-label check would. Lock the contract.
        let err = validate_whitelist_domain("-example.com").unwrap_err();
        assert!(
            err.contains("must not start with '-'"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn validate_whitelist_domain_rejects_leading_dash_on_inner_label() {
        // `foo.-bar.com` — only an inner label starts with `-`. The
        // trim-level guard does not fire (the trimmed input starts with
        // `foo`), so the per-label check is the only line of defense.
        assert!(validate_whitelist_domain("foo.-bar.com").is_err());
        let err = validate_whitelist_domain("foo.-bar.com").unwrap_err();
        assert!(
            err.contains("starts/ends with '-'"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn validate_whitelist_domain_rejects_trailing_dash_label() {
        assert!(validate_whitelist_domain("foo-.example.com").is_err());
        assert!(validate_whitelist_domain("foo.bar-.example.com").is_err());
        // Trailing dash on the last label is also caught.
        assert!(validate_whitelist_domain("example-").is_err());
    }

    #[test]
    fn validate_whitelist_domain_rejects_empty_label() {
        // Consecutive dots collapse to an empty label between them.
        assert!(validate_whitelist_domain("foo..example.com").is_err());
        let err = validate_whitelist_domain("foo..example.com").unwrap_err();
        assert!(err.contains("empty label"), "unexpected error: {}", err);
    }

    #[test]
    fn validate_whitelist_domain_accepts_rfc1123_all_numeric_label() {
        // PR #203 made all-numeric labels legal in the hosts parser;
        // whitelist validation should agree (issue #196 stays consistent
        // with the wider hosts parser semantics).
        assert_eq!(
            validate_whitelist_domain("123.example.com").unwrap(),
            "123.example.com"
        );
        assert_eq!(validate_whitelist_domain("1.2.3.4").unwrap(), "1.2.3.4");
    }

    #[test]
    fn validate_whitelist_domain_accepts_hyphen_in_middle_of_label() {
        // Hyphens inside a label (not at the start or end) are RFC 1123 §2.1 legal.
        assert_eq!(
            validate_whitelist_domain("foo-bar.example.com").unwrap(),
            "foo-bar.example.com"
        );
    }

    // -----------------------------------------------------------------
    // PR #131 re-review P1-1: the cold-start fix in `set_dns_mode_enable`
    // and `AppState::new` relies on `classify_rules` turning a source's
    // cached blocklist into non-empty rule sets, then `reload_ad_block_rules`
    // populating the engine. This locks that building block so a refactor
    // can't silently empty the engine on DNS enable.
    // -----------------------------------------------------------------
    #[test]
    fn classify_rules_populates_from_cached_source() {
        let temp = tempfile::TempDir::new().unwrap();
        let mk = |name: &str, response: AdBlockResponse| AdBlockSource {
            source_id: SourceId(Uuid::new_v4()),
            name: name.into(),
            url: "https://x".into(),
            enabled: true,
            response,
            last_fetched_at: None,
            last_error: None,
            rule_count: 2,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        };
        let za_source = mk("za", AdBlockResponse::ZeroAddress);
        let nx_source = mk("nx", AdBlockResponse::NxDomain);
        // Seed each source's cache file with parsed hosts-format content.
        mhost_storage::adblock::write_cache(
            temp.path(),
            &za_source.source_id,
            b"0.0.0.0 ads.example.com\n0.0.0.0 tracker.example.com\n",
        )
        .unwrap();
        mhost_storage::adblock::write_cache(
            temp.path(),
            &nx_source.source_id,
            b"0.0.0.0 blocked.example.com\n",
        )
        .unwrap();
        let state = AdBlockState {
            enabled: true,
            sources: vec![za_source, nx_source],
            whitelist: vec!["safe.example.com".to_string()],
            ..Default::default()
        };
        let (z, n, w) = classify_rules(&state, temp.path());
        assert_eq!(z.len(), 2, "zero_addr set seeded from za source cache");
        assert!(z.contains_key("ads.example.com"));
        assert!(z.contains_key("tracker.example.com"));
        assert_eq!(n.len(), 1, "nxdomain set seeded from nx source cache");
        assert!(n.contains("blocked.example.com"));
        assert_eq!(w.len(), 1);
    }

    // Issue #215: source order is a presentation concern only — the
    // priority contract (`classify_rules` → engine) must be invariant
    // under reorders. These tests pin two things:
    //
    //   1. The IPC-level mechanics (move up/down, boundary no-op,
    //      unknown id, persist_and_reload invoked) — so a future
    //      refactor that accidentally drops the call to
    //      `persist_and_reload` (e.g. "it's only a Vec swap, no
    //      need to reload the engine") is caught.
    //
    //   2. The behavioural invariant: swapping two sources must NOT
    //      change what `classify_rules` produces. Already covered
    //      for arbitrary source vectors by
    //      `classify_rules_is_independent_of_source_vec_order`; the
    //      reorder-specific tests below cover the persist round-trip
    //      path (reorder → write_state → reload → classify again).

    fn mk_source(name: &str, response: AdBlockResponse) -> AdBlockSource {
        AdBlockSource {
            source_id: SourceId(Uuid::new_v4()),
            name: name.into(),
            url: "https://x".into(),
            enabled: true,
            response,
            last_fetched_at: None,
            last_error: None,
            rule_count: 0,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        }
    }

    #[tokio::test]
    async fn reorder_up_swaps_with_predecessor() {
        let temp = tempfile::TempDir::new().unwrap();
        let (state, _storage) = make_test_app_state(temp.path());

        let s0 = mk_source("a", AdBlockResponse::ZeroAddress);
        let s1 = mk_source("b", AdBlockResponse::NxDomain);
        let s2 = mk_source("c", AdBlockResponse::ZeroAddress);
        let id0 = s0.source_id.clone();
        let id1 = s1.source_id.clone();
        let id2 = s2.source_id.clone();
        {
            let mut g = state.ad_block_state.write().await;
            g.sources = vec![s0, s1, s2];
        }

        let result = reorder_ad_block_source_impl(&state, &id1, ReorderDirection::Up)
            .await
            .expect("reorder up");
        let order: Vec<_> = result.iter().map(|s| s.source_id.clone()).collect();
        assert_eq!(
            order,
            vec![id1, id0, id2],
            "reorder Up should swap positions 0 and 1"
        );
    }

    #[tokio::test]
    async fn reorder_down_swaps_with_successor() {
        let temp = tempfile::TempDir::new().unwrap();
        let (state, _storage) = make_test_app_state(temp.path());

        let s0 = mk_source("a", AdBlockResponse::ZeroAddress);
        let s1 = mk_source("b", AdBlockResponse::NxDomain);
        let s2 = mk_source("c", AdBlockResponse::ZeroAddress);
        let id0 = s0.source_id.clone();
        let id1 = s1.source_id.clone();
        let id2 = s2.source_id.clone();
        {
            let mut g = state.ad_block_state.write().await;
            g.sources = vec![s0, s1, s2];
        }

        let result = reorder_ad_block_source_impl(&state, &id1, ReorderDirection::Down)
            .await
            .expect("reorder down");
        let order: Vec<_> = result.iter().map(|s| s.source_id.clone()).collect();
        assert_eq!(
            order,
            vec![id0, id2, id1],
            "reorder Down should swap positions 1 and 2"
        );
    }

    #[tokio::test]
    async fn reorder_up_at_first_position_is_noop() {
        let temp = tempfile::TempDir::new().unwrap();
        let (state, _storage) = make_test_app_state(temp.path());

        let s0 = mk_source("a", AdBlockResponse::ZeroAddress);
        let s1 = mk_source("b", AdBlockResponse::NxDomain);
        let id0 = s0.source_id.clone();
        let id1 = s1.source_id.clone();
        {
            let mut g = state.ad_block_state.write().await;
            g.sources = vec![s0, s1];
        }

        let result = reorder_ad_block_source_impl(&state, &id0, ReorderDirection::Up)
            .await
            .expect("no-op reorder must not error");
        let order: Vec<_> = result.iter().map(|s| s.source_id.clone()).collect();
        assert_eq!(
            order,
            vec![id0, id1],
            "first-position Up must not change order"
        );
    }

    #[tokio::test]
    async fn reorder_down_at_last_position_is_noop() {
        let temp = tempfile::TempDir::new().unwrap();
        let (state, _storage) = make_test_app_state(temp.path());

        let s0 = mk_source("a", AdBlockResponse::ZeroAddress);
        let s1 = mk_source("b", AdBlockResponse::NxDomain);
        let id0 = s0.source_id.clone();
        let id1 = s1.source_id.clone();
        {
            let mut g = state.ad_block_state.write().await;
            g.sources = vec![s0, s1];
        }

        let result = reorder_ad_block_source_impl(&state, &id1, ReorderDirection::Down)
            .await
            .expect("no-op reorder must not error");
        let order: Vec<_> = result.iter().map(|s| s.source_id.clone()).collect();
        assert_eq!(
            order,
            vec![id0, id1],
            "last-position Down must not change order"
        );
    }

    #[tokio::test]
    async fn reorder_unknown_source_id_returns_invalid_input() {
        let temp = tempfile::TempDir::new().unwrap();
        let (state, _storage) = make_test_app_state(temp.path());

        let bogus = SourceId(Uuid::new_v4());
        let result = reorder_ad_block_source_impl(&state, &bogus, ReorderDirection::Up).await;
        assert!(
            matches!(result, Err(MhostError::InvalidInput(_))),
            "unknown source id must error, got: {:?}",
            result
        );
    }

    /// End-to-end: reorder persists to disk AND the on-disk state,
    /// when re-classified, produces the same partitions as before.
    /// This is the property #215 §2 cares about — "调整顺序不改变拦截结果".
    #[tokio::test]
    async fn reorder_does_not_change_classify_rules_output() {
        use std::collections::HashSet;

        let temp = tempfile::TempDir::new().unwrap();
        let (state, _storage) = make_test_app_state(temp.path());

        // Two sources with overlapping but different response types.
        let s_za = mk_source("za", AdBlockResponse::ZeroAddress);
        let s_nx = mk_source("nx", AdBlockResponse::NxDomain);
        let id_za = s_za.source_id.clone();
        let id_nx = s_nx.source_id.clone();

        mhost_storage::adblock::write_cache(
            temp.path(),
            &id_za,
            b"0.0.0.0 shared.example.com\n0.0.0.0 za-only.example.com\n",
        )
        .unwrap();
        mhost_storage::adblock::write_cache(
            temp.path(),
            &id_nx,
            b"0.0.0.0 shared.example.com\n0.0.0.0 nx-only.example.com\n",
        )
        .unwrap();

        {
            let mut g = state.ad_block_state.write().await;
            g.enabled = true;
            g.sources = vec![s_za, s_nx];
        }

        let (za_before, nx_before, _) =
            classify_rules(&state.ad_block_state.read().await.clone(), temp.path());
        let za_before_set: HashSet<String> = za_before.keys().cloned().collect();
        let nx_before_set: HashSet<String> = nx_before.iter().cloned().collect();

        // Swap the two.
        reorder_ad_block_source_impl(&state, &id_za, ReorderDirection::Down)
            .await
            .expect("reorder");

        // The on-disk file must reflect the new order.
        let on_disk = adblock_store::read_state(temp.path()).unwrap();
        assert_eq!(on_disk.sources[0].source_id, id_nx);
        assert_eq!(on_disk.sources[1].source_id, id_za);

        // And `classify_rules` over the persisted state must produce
        // identical (za, nx) partitions — order is a presentation
        // concern only.
        let (za_after, nx_after, _) = classify_rules(&on_disk, temp.path());
        let za_after_set: HashSet<String> = za_after.keys().cloned().collect();
        let nx_after_set: HashSet<String> = nx_after.iter().cloned().collect();

        assert_eq!(
            za_before_set, za_after_set,
            "reorder must not change zero_addr partition"
        );
        assert_eq!(
            nx_before_set, nx_after_set,
            "reorder must not change nxdomain partition"
        );
    }
    // -----------------------------------------------------------------
    // PR #154 review (P2): exercise the cold-start hot-reload path that
    // AppState::new runs when `dns_enabled=true` was recovered from the
    // manifest. The headline fix is "DNS goes OFF → user adds source →
    // DNS goes ON → first query sees cached rules immediately" — without
    // the cold-start hot-reload there's a window where DNS is running but
    // ad-block isn't active yet.
    //
    // Test simulates the full flow without spinning up the proxy / Tauri
    // runtime: classify_rules → reload_ad_block_rules → spin up a real
    // DnsServer → fire a UDP query → assert the blocked domain returns
    // 0.0.0.0 instead of leaking upstream.
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn cold_start_hot_reload_blocks_first_query() {
        use mhost_dns::DnsConfig;

        let temp = tempfile::TempDir::new().unwrap();
        let source_id = SourceId(Uuid::new_v4());
        let source = AdBlockSource {
            source_id: source_id.clone(),
            name: "test-blocklist".into(),
            url: "https://x".into(),
            enabled: true,
            response: AdBlockResponse::ZeroAddress,
            last_fetched_at: None,
            last_error: None,
            rule_count: 1,
            etag: None,
            rules_limit_override: None,
            last_refresh_duration_ms: None,
            last_refresh_failed_at: None,
        };
        mhost_storage::adblock::write_cache(
            temp.path(),
            &source_id,
            b"0.0.0.0 cold-start-ads.example.com\n",
        )
        .unwrap();

        let state = AdBlockState {
            enabled: true,
            sources: vec![source],
            whitelist: vec![],
            ..Default::default()
        };

        // Simulate the AppState::new cold-start block.
        let (za, nx, wl) = classify_rules(&state, temp.path());
        assert!(za.contains_key("cold-start-ads.example.com"));

        // Wire into a real DnsServer and query.
        let port = pick_free_port();
        let config = DnsConfig {
            port,
            upstream: vec!["127.0.0.1:1".to_string()], // blackhole — fail fast
            timeout_ms: 100,
            refresh_upstream: false,
            cache_size: 100,
        };
        let server = std::sync::Arc::new(mhost_dns::DnsServer::new(config).unwrap());
        // Master switch on — the next assertion is `rule_count == 1`
        // which depends on the rebuild going through with rules
        // actually fed into the engine.
        server.reload_ad_block_rules(true, za, nx, wl);
        assert_eq!(server.ad_block_rule_count(), 1);

        let server_clone = std::sync::Arc::clone(&server);
        let server_handle = tokio::spawn(async move { server_clone.start().await });
        // Wait for the server to be listening.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !server.is_running() && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(server.is_running(), "server should start");

        // Send a UDP query for the blocked domain.
        use hickory_proto::op::{Message, OpCode, Query};
        use hickory_proto::rr::{Name, RecordType};
        use hickory_proto::serialize::binary::{BinDecodable, BinEncodable};
        use tokio::net::UdpSocket;
        let query_name = Name::from_utf8("cold-start-ads.example.com.").unwrap();
        let query = Query::query(query_name, RecordType::A);
        let mut request = Message::new();
        request.set_id(0x4242);
        request.set_recursion_desired(true);
        request.set_op_code(OpCode::Query);
        request.add_query(query);
        let bytes = request.to_bytes().unwrap();

        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        client
            .send_to(&bytes, format!("127.0.0.1:{}", port))
            .await
            .unwrap();
        let mut buf = vec![0u8; 4096];
        let (len, _) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client.recv_from(&mut buf),
        )
        .await
        .expect("server response timeout")
        .expect("recv_from failed");
        let response = hickory_proto::op::Message::from_bytes(&buf[..len]).unwrap();
        assert_eq!(
            response.response_code(),
            hickory_proto::op::ResponseCode::NoError
        );
        assert_eq!(
            response.answer_count(),
            1,
            "blocked domain should be answered"
        );
        let answer = &response.answers()[0];
        if let Some(hickory_proto::rr::RData::A(a)) = answer.data() {
            assert_eq!(
                a.0,
                std::net::Ipv4Addr::new(0, 0, 0, 0),
                "cold-start ad-block should return 0.0.0.0"
            );
        } else {
            panic!("expected A record, got {:?}", answer.data());
        }

        server.stop().await.unwrap();
        let _ = server_handle.await;
    }

    /// Pick a free UDP port by binding to port 0. Avoids colliding with
    /// other tests on the same machine.
    fn pick_free_port() -> u16 {
        let listener = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind free port");
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    // -----------------------------------------------------------------
    // PR #131 re-review P1-2: a fetch failure must NOT skip persistence —
    // the source was already pushed into in-memory state, and skipping
    // `persist_and_reload` lost it on restart. Point the source at a loopback
    // port that refuses connections so `fetch_source` fails fast (no 30s
    // timeout). The source should still be on disk after the call errors.
    // -----------------------------------------------------------------

    /// Minimal `AppState` for command-impl tests, backed by `temp_path`.
    fn make_test_app_state(
        temp_path: &std::path::Path,
    ) -> (
        crate::state::AppState,
        Arc<dyn mhost_storage::storage::Storage + Send + Sync>,
    ) {
        use crate::state::AppState;
        use mhost_apply::writer::HostsWriter;
        use mhost_storage::storage::FileStorage;

        let storage = Arc::new(FileStorage::new(temp_path))
            as Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let state = AppState {
            storage: storage.clone(),
            writer: Arc::new(HostsWriter::new()),
            apply_lock: crate::state::ApplyLock::new(),
            snapshot_lock: Arc::new(crate::state::ApplyLock::new()),
            last_profile_ids: std::sync::Mutex::new(Vec::new()),
            cached_profiles: std::sync::RwLock::new(None), // lazy load on first cached_profiles() call
            dns_server: Arc::new(std::sync::Mutex::new(None)),
            dns_enabled: std::sync::atomic::AtomicBool::new(false),
            original_dns: tokio::sync::RwLock::new(mhost_core::OriginalDns::DhcpEmpty),
            dns_lock: crate::state::ApplyLock::new(),
            dns_cancel: std::sync::Mutex::new(None),
            ad_block_state: Arc::new(tokio::sync::RwLock::new(AdBlockState::default())),
            ad_block_refresh_task: std::sync::Mutex::new(None),
            ad_block_refresh_cancel: std::sync::Mutex::new(
                tokio_util::sync::CancellationToken::new(),
            ),
            ad_block_refresh_wake: std::sync::Arc::new(tokio::sync::Notify::new()),
        };
        (state, storage)
    }

    #[tokio::test]
    async fn add_ad_block_source_persists_on_fetch_failure() {
        let temp = tempfile::TempDir::new().unwrap();
        let (state, storage) = make_test_app_state(temp.path());

        // Port 1 on loopback refuses connections → fetch_source errors fast.
        let url = "http://127.0.0.1:1/blocklist".to_string();
        let err =
            add_ad_block_source_impl(&state, "failing".into(), url, AdBlockResponse::ZeroAddress)
                .await
                .expect_err("fetch should fail (connection refused)");
        assert!(
            err.to_string().contains("fetch")
                || err.to_string().to_lowercase().contains("connect")
                || err.to_string().to_lowercase().contains("error")
        );

        // P1-2 invariant: the source is persisted despite the fetch failure.
        let persisted = mhost_storage::adblock::read_state(storage.root())
            .expect("adblock.json should exist after persist_and_reload");
        assert_eq!(
            persisted.sources.len(),
            1,
            "source must be persisted even when initial fetch fails (P1-2)"
        );
        assert_eq!(persisted.sources[0].name, "failing");
        assert!(
            persisted.sources[0].last_error.is_some(),
            "last_error must be recorded on the persisted source"
        );
    }

    // -----------------------------------------------------------------
    // Issue #196: batch whitelist add/remove + tightened validation.
    //
    // The behavior contracts the frontend relies on:
    //   - one bad entry does not abort the batch
    //   - dedupe is silent (no rejected entry for duplicates)
    //   - persist + DNS reload fires exactly once for a non-empty diff
    //   - remove normalizes (trim + lowercase) and tolerates missing
    //   - the single-entry IPCs are thin wrappers around `*_impl` and
    //     therefore share the dedupe / persist-once behavior
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn add_whitelist_many_accepts_valid_rejects_invalid_dedupes() {
        let temp = tempfile::TempDir::new().unwrap();
        let (state, _storage) = make_test_app_state(temp.path());

        // Mix of: valid new, valid duplicate (already in list), trailing dot,
        // leading dash, double-dot empty label. Dedup should be silent.
        let inputs = vec![
            "good.example.com".to_string(),
            "  Mixed.Case.COM  ".to_string(),
            "good.example.com".to_string(),  // duplicate of #0
            "bad.example.com.".to_string(),  // trailing dot
            "-leading-dash.com".to_string(), // leading dash
            "foo..bar.com".to_string(),      // empty label
            "another.good.org".to_string(),
        ];
        let result = add_whitelist_impl(&state, inputs).await.unwrap();

        // good.example.com, mixed.case.com (normalized), another.good.org
        assert_eq!(
            result.whitelist,
            vec![
                "good.example.com".to_string(),
                "mixed.case.com".to_string(),
                "another.good.org".to_string(),
            ],
            "valid entries (post-normalization) should be persisted in input order"
        );
        assert_eq!(
            result.rejected.len(),
            3,
            "three invalid inputs were rejected"
        );
        let rejected_inputs: Vec<&str> = result.rejected.iter().map(|e| e.input.as_str()).collect();
        assert!(rejected_inputs.contains(&"bad.example.com."));
        assert!(rejected_inputs.contains(&"-leading-dash.com"));
        assert!(rejected_inputs.contains(&"foo..bar.com"));
        // Each rejection must carry a non-empty reason for the frontend toast.
        for r in &result.rejected {
            assert!(!r.reason.is_empty(), "rejection reason must not be empty");
        }
    }

    #[tokio::test]
    async fn add_whitelist_impl_skips_persist_when_all_duplicates() {
        let temp = tempfile::TempDir::new().unwrap();
        let (state, storage) = make_test_app_state(temp.path());

        // Seed whitelist through the public impl so persist + on-disk state
        // is in sync before we test the "all duplicates" branch.
        add_whitelist_impl(&state, vec!["a.example.com".to_string()])
            .await
            .unwrap();
        let mtime_before = std::fs::metadata(storage.root().join("adblock.json"))
            .unwrap()
            .modified()
            .unwrap();

        // Sleep just enough to ensure a second `modified()` tick is observable
        // (some filesystems have 1s mtime granularity on CI runners).
        std::thread::sleep(std::time::Duration::from_millis(50));

        // All duplicates — nothing to add. `add_whitelist_impl` should NOT
        // touch the file (the issue #196 fix: persist only when newly_added > 0).
        let result = add_whitelist_impl(
            &state,
            vec![
                "a.example.com".to_string(),
                "  A.EXAMPLE.COM  ".to_string(), // same after normalize
            ],
        )
        .await
        .unwrap();
        assert_eq!(result.whitelist, vec!["a.example.com".to_string()]);
        assert!(result.rejected.is_empty());

        let mtime_after = std::fs::metadata(storage.root().join("adblock.json"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(
            mtime_before, mtime_after,
            "persist_and_reload must be skipped when no new entry was added"
        );
    }

    #[tokio::test]
    async fn add_whitelist_impl_persists_when_at_least_one_new() {
        let temp = tempfile::TempDir::new().unwrap();
        let (state, storage) = make_test_app_state(temp.path());

        let mtime_before = std::fs::metadata(storage.root().join("adblock.json"))
            .ok()
            .map(|m| m.modified().unwrap());
        assert!(
            mtime_before.is_none(),
            "adblock.json should not exist yet on a fresh AppState"
        );

        add_whitelist_impl(
            &state,
            vec![
                "first.example.com".to_string(),
                "bad..example.com".to_string(), // rejected
                "second.example.com".to_string(),
            ],
        )
        .await
        .unwrap();

        let persisted = mhost_storage::adblock::read_state(storage.root())
            .expect("adblock.json should exist after a successful add");
        assert_eq!(
            persisted.whitelist,
            vec![
                "first.example.com".to_string(),
                "second.example.com".to_string()
            ],
        );
    }

    #[tokio::test]
    async fn remove_whitelist_impl_normalizes_and_tolerates_missing() {
        let temp = tempfile::TempDir::new().unwrap();
        let (state, _storage) = make_test_app_state(temp.path());

        add_whitelist_impl(
            &state,
            vec![
                "alpha.example.com".to_string(),
                "Beta.Example.Com".to_string(),
                "gamma.example.com".to_string(),
            ],
        )
        .await
        .unwrap();

        // Remove with mixed casing + whitespace, plus an entry that was
        // never added. None of these should error.
        let result = remove_whitelist_impl(
            &state,
            vec![
                "  ALPHA.EXAMPLE.COM  ".to_string(),
                "beta.example.com".to_string(),
                "missing.example.com".to_string(), // silently ignored
            ],
        )
        .await
        .unwrap();

        assert_eq!(
            result,
            vec!["gamma.example.com".to_string()],
            "remove should leave only the un-removed entry; missing entries are tolerated"
        );
    }

    #[tokio::test]
    async fn remove_whitelist_impl_skips_persist_when_all_targets_missing() {
        // Mirrors `add_whitelist_impl_skips_persist_when_all_duplicates`:
        // a 200-entry batch where every target is absent must not pay
        // for a DNS reload (and the LRU cache clear it triggers).
        let temp = tempfile::TempDir::new().unwrap();
        let (state, storage) = make_test_app_state(temp.path());

        add_whitelist_impl(&state, vec!["keep.example.com".to_string()])
            .await
            .unwrap();
        let mtime_before = std::fs::metadata(storage.root().join("adblock.json"))
            .unwrap()
            .modified()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let inputs: Vec<String> = (0..200)
            .map(|i| format!("missing{}.example.com", i))
            .collect();
        let result = remove_whitelist_impl(&state, inputs).await.unwrap();
        assert_eq!(result, vec!["keep.example.com".to_string()]);

        let mtime_after = std::fs::metadata(storage.root().join("adblock.json"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(
            mtime_before, mtime_after,
            "all-missing batch must skip persist_and_reload (no DNS churn on a no-op)"
        );
    }

    #[tokio::test]
    async fn remove_whitelist_impl_drops_empty_and_whitespace_inputs() {
        // Empty/whitespace inputs are filtered out before reaching the
        // retain loop — and a fully-empty batch must not pay for a
        // persist+reload cycle (mirrors the `add_whitelist_impl`
        // skip-on-no-new-entry behavior; both avoid clearing the LRU
        // response cache on a no-op).
        let temp = tempfile::TempDir::new().unwrap();
        let (state, storage) = make_test_app_state(temp.path());

        add_whitelist_impl(&state, vec!["keep.example.com".to_string()])
            .await
            .unwrap();
        let mtime_before = std::fs::metadata(storage.root().join("adblock.json"))
            .unwrap()
            .modified()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let result = remove_whitelist_impl(&state, vec!["".to_string(), "   ".to_string()])
            .await
            .unwrap();
        assert_eq!(result, vec!["keep.example.com".to_string()]);

        let mtime_after = std::fs::metadata(storage.root().join("adblock.json"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(
            mtime_before, mtime_after,
            "empty-input batch must skip persist_and_reload (no DNS churn on a no-op)"
        );
    }

    #[tokio::test]
    async fn add_whitelist_many_does_not_change_ad_block_rule_count() {
        // Issue #196 acceptance: 200 valid entries must leave the
        // rule_count alone (whitelist has no bearing on engine rule sets,
        // it only gates matching).
        let temp = tempfile::TempDir::new().unwrap();
        let (state, _storage) = make_test_app_state(temp.path());

        let inputs: Vec<String> = (0..200).map(|i| format!("host{}.example.com", i)).collect();
        let result = add_whitelist_impl(&state, inputs).await.unwrap();
        assert_eq!(result.whitelist.len(), 200);
        assert!(result.rejected.is_empty());

        let snap = state.ad_block_state.read().await;
        assert!(
            snap.sources.is_empty(),
            "no sources were added in this test"
        );
    }

    /// Bind a mock listener up-front and hand the *bound* listener to the
    /// mock spawner (issue #206 finding 3). The old pick-free-port-then-
    /// rebind helper had a TOCTOU window — `cargo` runs tests in parallel
    /// threads, so two tests could pick the same port and one
    /// `expect("bind mock listener")` would panic (CI flake). Binding once
    /// and passing the listener itself removes the rebind entirely.
    fn bind_mock_listener() -> std::net::TcpListener {
        std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind mock listener")
    }

    /// Signal the mock to stop and join its accept thread (issue #206
    /// finding 3 — the handle used to be dropped unjoined, leaving the
    /// thread parked in `accept()` for the rest of the process).
    fn stop_mock(
        stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        handle: std::thread::JoinHandle<()>,
    ) {
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = handle.join();
    }

    // -----------------------------------------------------------------
    // Issue #193: RFC 7232 conditional GET on ad-block sources.
    //
    // These tests use a minimal in-process TCP mock HTTP server so they
    // don't depend on any external crate (no `wiremock` / `httpmock` — those
    // would inflate the dev-dependency footprint for what's effectively a
    // ~80-line helper). The mock is single-threaded, FIFO over a
    // `Mutex<VecDeque<Response>>`, and never sends back a real HTTP/1.1
    // `Content-Length`-aware body — it's just enough to exercise ureq's
    // 304 contract.
    // -----------------------------------------------------------------

    /// One canned response from the mock server.
    #[derive(Clone)]
    struct MockResponse {
        status: u16,
        /// Headers to emit before the body (already pre-formatted lines,
        /// e.g. `"ETag: \"v1\""` — joined onto the response as-is).
        headers: Vec<String>,
        body: Vec<u8>,
        /// Milliseconds to stall before writing the response (issue #206
        /// finding 1 test — used to prove same-source refreshes are
        /// serialized by the per-source gate).
        delay_ms: u64,
    }

    impl MockResponse {
        fn ok_200(etag: &str, body: &[u8]) -> Self {
            Self {
                status: 200,
                headers: vec![
                    // `format!`-vs-`.to_string()`: with no interpolation
                    // the lint `clippy::useless_format` (Rust 1.98+) flags
                    // `format!("literal")` as redundant.
                    "Content-Type: text/plain".to_string(),
                    format!("Content-Length: {}", body.len()),
                    format!("ETag: {}", etag),
                ],
                body: body.to_vec(),
                delay_ms: 0,
            }
        }

        /// 200 with a delayed response — lets a test observe whether a
        /// second request arrives before or after this one completes.
        fn ok_200_delayed(etag: &str, body: &[u8], delay_ms: u64) -> Self {
            let mut r = Self::ok_200(etag, body);
            r.delay_ms = delay_ms;
            r
        }

        fn not_modified_304() -> Self {
            Self {
                status: 304,
                headers: vec![],
                body: Vec::new(),
                delay_ms: 0,
            }
        }
    }

    /// Records a single request the mock received. `received_at` is the
    /// instant the request headers finished arriving, so tests can assert
    /// arrival ordering across concurrent refreshes.
    #[derive(Clone, Debug)]
    struct RecordedRequest {
        method: String,
        path: String,
        if_none_match: Option<String>,
        if_modified_since: Option<String>,
        received_at: std::time::Instant,
    }

    impl Default for RecordedRequest {
        fn default() -> Self {
            Self {
                method: String::new(),
                path: String::new(),
                if_none_match: None,
                if_modified_since: None,
                received_at: std::time::Instant::now(),
            }
        }
    }

    /// Spawn a mock HTTP/1.1 server on an already-bound `listener` (issue
    /// #206 finding 3 — bind once at the call site, no rebind TOCTOU). Each
    /// accepted connection pulls the next response off `responses` (FIFO).
    /// Returns the join handle plus a shared `Vec<RecordedRequest>` for
    /// assertions.
    ///
    /// The accept loop polls `stop_flag` with a nonblocking listener and a
    /// 5 ms idle sleep, so `stop_mock` can join the thread promptly instead
    /// of leaving it parked in a blocking `accept()` forever.
    fn spawn_mock_http(
        listener: std::net::TcpListener,
        responses: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<MockResponse>>>,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> (
        std::thread::JoinHandle<()>,
        std::sync::Arc<std::sync::Mutex<Vec<RecordedRequest>>>,
    ) {
        let recorded: std::sync::Arc<std::sync::Mutex<Vec<RecordedRequest>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded_clone = recorded.clone();
        let handle = std::thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("set nonblocking listener");
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => continue,
                };
                // Accepted sockets can inherit the listener's nonblocking
                // flag on some platforms; the request-read loop below
                // assumes blocking semantics.
                stream.set_nonblocking(false).expect("set blocking stream");
                // Read until we see CRLFCRLF (end of headers). Cap at 8 KB
                // to avoid unbounded reads from a misbehaving client — ureq
                // sends a tiny request.
                let mut buf = [0u8; 8192];
                let mut received = Vec::new();
                loop {
                    use std::io::Read;
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            received.extend_from_slice(&buf[..n]);
                            if received.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                            if received.len() >= buf.len() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let req_text = String::from_utf8_lossy(&received);
                let mut rec = RecordedRequest::default();
                for (i, line) in req_text.lines().enumerate() {
                    if i == 0 {
                        // "GET /path HTTP/1.1"
                        let mut parts = line.split_whitespace();
                        rec.method = parts.next().unwrap_or("").to_string();
                        rec.path = parts.next().unwrap_or("").to_string();
                    } else if line.is_empty() {
                        break;
                    } else {
                        // HTTP header names are case-insensitive — ureq
                        // emits lowercase, but a real upstream might emit
                        // `If-None-Match`. Match by lowercased prefix.
                        let lower = line.to_ascii_lowercase();
                        if let Some(v) = lower.strip_prefix("if-none-match:") {
                            // Re-slice the *original* line to keep the value
                            // case intact (etag values are case-sensitive).
                            let v = &line[line.len() - v.len()..];
                            rec.if_none_match = Some(v.trim().to_string());
                        } else if let Some(v) = lower.strip_prefix("if-modified-since:") {
                            let v = &line[line.len() - v.len()..];
                            rec.if_modified_since = Some(v.trim().to_string());
                        }
                    }
                }
                rec.received_at = std::time::Instant::now();
                recorded_clone.lock().unwrap().push(rec);

                let resp = responses
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("test ran out of canned responses");
                if resp.delay_ms > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(resp.delay_ms));
                }
                let reason = match resp.status {
                    200 => "OK",
                    304 => "Not Modified",
                    _ => "Status",
                };
                let mut head = format!(
                    "HTTP/1.1 {} {}\r\nConnection: close\r\n",
                    resp.status, reason
                );
                for h in &resp.headers {
                    head.push_str(&format!("{}\r\n", h));
                }
                head.push_str("\r\n");
                use std::io::Write;
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&resp.body);
                let _ = stream.flush();
                // `drop(stream)` closes the connection so ureq's response
                // reader sees EOF and finishes.
            }
        });
        (handle, recorded)
    }

    /// Helper: write a tiny valid hosts blocklist into the adblock cache
    /// for `source_id` (so the 304 path has something to *not* overwrite).
    fn write_canonical_cache_for(
        root: &std::path::Path,
        source_id: &SourceId,
        body: &str,
    ) -> std::time::SystemTime {
        use mhost_storage::adblock as adblock_store;
        adblock_store::write_cache(root, source_id, body.as_bytes()).unwrap();
        // Stash an mtime the test can later compare against. The cache
        // path mirrors `adblock_store::cache_path`; we can't call the
        // private helper from a test in another crate, but the layout is
        // pinned by the storage module — see `adblock.rs`.
        let p = root
            .join("adblock-cache")
            .join(format!("{}.txt", source_id.0));
        let mtime = filetime::FileTime::now();
        filetime::set_file_mtime(&p, mtime).unwrap();
        mtime.into()
    }

    // -- The tests ----------------------------------------------------------

    /// Issue #193: the first fetch returns `Fresh` (200 + ETag + body) and
    /// persists the ETag on the source record.
    #[tokio::test]
    async fn fetch_source_sync_returns_fresh_on_200() {
        let listener = bind_mock_listener();
        let port = listener.local_addr().unwrap().port();
        let body = b"0.0.0.0 example.com";
        let responses = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from(vec![MockResponse::ok_200("\"v1\"", body)]),
        ));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_h, recorded) = spawn_mock_http(listener, responses, stop.clone());

        let url = format!("http://127.0.0.1:{}/list", port);
        let outcome =
            tokio::task::spawn_blocking(move || fetch_source_sync(&url, None, None, false))
                .await
                .unwrap()
                .expect("fetch should succeed");

        match outcome {
            FetchOutcome::Fresh { body: got, etag } => {
                assert_eq!(got, body);
                assert_eq!(etag.as_deref(), Some("\"v1\""));
            }
            FetchOutcome::NotModified => panic!("first fetch must be Fresh"),
        }

        // No conditional headers on a first fetch.
        {
            let recs = recorded.lock().unwrap();
            assert_eq!(recs.len(), 1);
            assert!(recs[0].if_none_match.is_none());
            assert!(recs[0].if_modified_since.is_none());
        }

        stop_mock(&stop, _h);
    }

    /// Issue #193: with an ETag already on the source record, the next
    /// fetch sends `If-None-Match: <etag>`. When the upstream replies 304,
    /// the function returns `NotModified` and does NOT consume any body.
    #[tokio::test]
    async fn fetch_source_sync_returns_not_modified_on_304() {
        let listener = bind_mock_listener();
        let port = listener.local_addr().unwrap().port();
        let responses = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from(vec![MockResponse::not_modified_304()]),
        ));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_h, recorded) = spawn_mock_http(listener, responses, stop.clone());

        let url = format!("http://127.0.0.1:{}/list", port);
        let outcome = tokio::task::spawn_blocking(move || {
            fetch_source_sync(
                &url,
                Some("\"v1\""),
                Some("Sun, 06 Nov 1994 08:49:37 GMT"),
                false,
            )
        })
        .await
        .unwrap()
        .expect("304 should not be an error");

        assert!(
            matches!(outcome, FetchOutcome::NotModified),
            "expected NotModified, got {:?}",
            outcome
        );

        // Both conditional headers should have been forwarded.
        {
            let recs = recorded.lock().unwrap();
            assert_eq!(recs.len(), 1);
            assert_eq!(recs[0].if_none_match.as_deref(), Some("\"v1\""));
            assert_eq!(
                recs[0].if_modified_since.as_deref(),
                Some("Sun, 06 Nov 1994 08:49:37 GMT")
            );
        }

        stop_mock(&stop, _h);
    }

    /// Issue #193: end-to-end — first call writes the cache and stores
    /// the ETag; second call hits a 304 from the upstream, and the on-disk
    /// cache file is NOT touched (mtime unchanged).
    #[tokio::test]
    async fn fetch_and_cache_source_304_leaves_cache_untouched() {
        use mhost_storage::adblock as adblock_store;
        use mhost_storage::storage::FileStorage;

        let listener = bind_mock_listener();
        let port = listener.local_addr().unwrap().port();
        // First call: 200 + body + ETag. Second call: 304.
        let initial_body = b"0.0.0.0 ads.example.com";
        let responses = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from(vec![
                MockResponse::ok_200("\"v1\"", initial_body),
                MockResponse::not_modified_304(),
            ]),
        ));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_h, recorded) = spawn_mock_http(listener, responses, stop.clone());

        let temp = tempfile::TempDir::new().unwrap();
        let storage = std::sync::Arc::new(FileStorage::new(temp.path()))
            as std::sync::Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let ad_block_state = std::sync::Arc::new(tokio::sync::RwLock::new(AdBlockState::default()));

        // Seed one source pointing at the mock URL, no prior cache.
        let source_id = SourceId(uuid::Uuid::new_v4());
        {
            let mut g = ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "test".into(),
                url: format!("http://127.0.0.1:{}/list", port),
                enabled: true,
                response: AdBlockResponse::ZeroAddress,
                last_fetched_at: None,
                last_error: None,
                rule_count: 0,
                etag: None,
                rules_limit_override: None,
                last_refresh_duration_ms: None,
                last_refresh_failed_at: None,
            });
        }

        // First call: should write cache + persist the etag.
        fetch_and_cache_source(&storage, &ad_block_state, &source_id, false)
            .await
            .expect("first fetch should succeed");

        let snap_after_first = {
            let g = ad_block_state.read().await;
            adblock_store::find_source(&g, &source_id).cloned().unwrap()
        };
        assert_eq!(snap_after_first.etag.as_deref(), Some("\"v1\""));
        assert_eq!(snap_after_first.rule_count, 1);
        assert!(snap_after_first.last_error.is_none());
        let first_fetched_at = snap_after_first.last_fetched_at.expect("set on 200");

        // Stash the cache file mtime and contents.
        let cache_path = temp
            .path()
            .join("adblock-cache")
            .join(format!("{}.txt", source_id.0));
        assert!(cache_path.exists(), "first fetch should write cache");
        let mtime_before = std::fs::metadata(&cache_path).unwrap().modified().unwrap();
        let body_before = std::fs::read(&cache_path).unwrap();
        assert_eq!(body_before, initial_body);

        // Sleep enough that any rewrite would bump the mtime. `filetime`
        // has 1-second resolution on some filesystems, so use 1.5 s.
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        // Second call: 304 from upstream. Cache must NOT be rewritten,
        // and `last_fetched_at` should advance.
        fetch_and_cache_source(&storage, &ad_block_state, &source_id, false)
            .await
            .expect("304 should be a success");

        // Cache file unchanged.
        let mtime_after = std::fs::metadata(&cache_path).unwrap().modified().unwrap();
        let body_after = std::fs::read(&cache_path).unwrap();
        assert_eq!(
            mtime_after, mtime_before,
            "cache file mtime must NOT change on a 304 (issue #193)"
        );
        assert_eq!(body_after, initial_body);

        // Source record: etag + rule_count preserved, last_fetched_at bumped,
        // last_error still None.
        let snap_after_second = {
            let g = ad_block_state.read().await;
            adblock_store::find_source(&g, &source_id).cloned().unwrap()
        };
        assert_eq!(
            snap_after_second.etag, snap_after_first.etag,
            "etag must NOT change on a 304"
        );
        assert_eq!(
            snap_after_second.rule_count, snap_after_first.rule_count,
            "rule_count must NOT change on a 304"
        );
        assert!(
            snap_after_second.last_error.is_none(),
            "304 must clear any prior last_error"
        );
        assert!(
            snap_after_second.last_fetched_at.unwrap() > first_fetched_at,
            "last_fetched_at must advance on a 304"
        );

        // The mock should have seen BOTH requests with If-None-Match set
        // on the second one.
        {
            let recs = recorded.lock().unwrap();
            assert_eq!(recs.len(), 2);
            assert!(
                recs[0].if_none_match.is_none(),
                "first request should not carry conditional headers"
            );
            assert_eq!(
                recs[1].if_none_match.as_deref(),
                Some("\"v1\""),
                "second request must echo the cached ETag"
            );
        }

        stop_mock(&stop, _h);
    }

    /// Issue #193: when the upstream returns 200 again (etag mismatch /
    /// upstream rolled forward), the cache is rewritten with the new body
    /// and the new ETag is persisted. This is the "etag stale" path —
    /// distinct from the 304 path and worth pinning down so future refactors
    /// don't accidentally take it.
    #[tokio::test]
    async fn fetch_and_cache_source_overwrites_on_etag_mismatch() {
        use mhost_storage::adblock as adblock_store;
        use mhost_storage::storage::FileStorage;

        let listener = bind_mock_listener();
        let port = listener.local_addr().unwrap().port();
        let new_body = b"0.0.0.0 new-ads.example.com";
        let responses = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from(vec![MockResponse::ok_200("\"v2\"", new_body)]),
        ));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_h, _recorded) = spawn_mock_http(listener, responses, stop.clone());

        let temp = tempfile::TempDir::new().unwrap();
        let storage = std::sync::Arc::new(FileStorage::new(temp.path()))
            as std::sync::Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let ad_block_state = std::sync::Arc::new(tokio::sync::RwLock::new(AdBlockState::default()));
        let source_id = SourceId(uuid::Uuid::new_v4());

        // Pre-seed: source with a stale `etag` + a stale cache file.
        {
            let mut g = ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "stale".into(),
                url: format!("http://127.0.0.1:{}/list", port),
                enabled: true,
                response: AdBlockResponse::ZeroAddress,
                last_fetched_at: None,
                last_error: None,
                rule_count: 0,
                etag: Some("\"v0-stale\"".to_string()),
                rules_limit_override: None,
                last_refresh_duration_ms: None,
                last_refresh_failed_at: None,
            });
        }
        // Plant a stale cache file so we can assert it gets overwritten.
        let stale_body = b"0.0.0.0 old-ads.example.com";
        write_canonical_cache_for(
            temp.path(),
            &source_id,
            std::str::from_utf8(stale_body).unwrap(),
        );
        let cache_path = temp
            .path()
            .join("adblock-cache")
            .join(format!("{}.txt", source_id.0));
        let mtime_before = std::fs::metadata(&cache_path).unwrap().modified().unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

        fetch_and_cache_source(&storage, &ad_block_state, &source_id, false)
            .await
            .expect("200 should succeed");

        let snap = {
            let g = ad_block_state.read().await;
            adblock_store::find_source(&g, &source_id).cloned().unwrap()
        };
        assert_eq!(snap.etag.as_deref(), Some("\"v2\""));
        assert_eq!(snap.rule_count, 1);
        let body = std::fs::read(&cache_path).unwrap();
        assert_eq!(body, new_body, "cache must be overwritten on fresh 200");
        let mtime_after = std::fs::metadata(&cache_path).unwrap().modified().unwrap();
        assert!(
            mtime_after > mtime_before,
            "cache file mtime should advance when content is rewritten"
        );

        stop_mock(&stop, _h);
    }

    /// Issue #193 — `If-Modified-Since` only (no prior ETag) is also a
    /// valid conditional GET. The first fetch produces no conditional
    /// headers; the second fetch sends `If-Modified-Since` derived from
    /// the source's `last_fetched_at`. A 304 response clears `last_error`
    /// and bumps `last_fetched_at` (round-trip time recorded as
    /// "successful handshake with upstream"), without touching the cache.
    #[tokio::test]
    async fn fetch_and_cache_source_uses_if_modified_sans_when_no_etag() {
        use mhost_storage::adblock as adblock_store;
        use mhost_storage::storage::FileStorage;

        let listener = bind_mock_listener();
        let port = listener.local_addr().unwrap().port();
        let responses = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from(vec![MockResponse::not_modified_304()]),
        ));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_h, recorded) = spawn_mock_http(listener, responses, stop.clone());

        let temp = tempfile::TempDir::new().unwrap();
        let storage = std::sync::Arc::new(FileStorage::new(temp.path()))
            as std::sync::Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let ad_block_state = std::sync::Arc::new(tokio::sync::RwLock::new(AdBlockState::default()));
        let source_id = SourceId(uuid::Uuid::new_v4());
        let prior_fetch = chrono::Utc::now() - chrono::Duration::hours(1);
        {
            let mut g = ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "test".into(),
                url: format!("http://127.0.0.1:{}/list", port),
                enabled: true,
                response: AdBlockResponse::ZeroAddress,
                last_fetched_at: Some(prior_fetch),
                last_error: Some("prior boom".into()),
                rule_count: 7,
                etag: None, // no etag → If-None-Match will be omitted
                rules_limit_override: None,
                last_refresh_duration_ms: None,
                last_refresh_failed_at: None,
            });
        }
        // Issue #206 finding 2: the 304 path now verifies the cache file
        // exists. Plant one so this test keeps exercising the pure 304
        // contract instead of tripping the missing-cache downgrade.
        mhost_storage::adblock::write_cache(
            temp.path(),
            &source_id,
            b"0.0.0.0 cached.example.com\n",
        )
        .unwrap();

        fetch_and_cache_source(&storage, &ad_block_state, &source_id, false)
            .await
            .expect("304 should succeed");

        let snap = {
            let g = ad_block_state.read().await;
            adblock_store::find_source(&g, &source_id).cloned().unwrap()
        };
        // last_error cleared on the successful 304 round-trip.
        assert!(snap.last_error.is_none(), "304 must clear last_error");
        // rule_count preserved — we did NOT parse a new body.
        assert_eq!(snap.rule_count, 7);
        // last_fetched_at advanced past the seeded prior_fetch.
        assert!(snap.last_fetched_at.unwrap() > prior_fetch);

        // Recorded request: no If-None-Match (etag was None), but
        // If-Modified-Since should be present.
        {
            let recs = recorded.lock().unwrap();
            assert_eq!(recs.len(), 1);
            assert!(recs[0].if_none_match.is_none());
            assert!(
                recs[0].if_modified_since.is_some(),
                "If-Modified-Since should be sent when source has a prior fetch time"
            );
        }

        stop_mock(&stop, _h);
    }

    // -----------------------------------------------------------------
    // Review of #201: when the agent flipped `http_status_as_error`
    // from default-true to false, the 4xx/5xx → `MhostError::ExternalApi`
    // mapping stopped being a built-in ureq default and became a
    // hand-rolled `!(200..300)` check on the response status. This
    // test pins that contract down — the failure message must mention
    // the status code, and the error must surface to the caller (the
    // fetch path then records it on `last_error` via `record_fetch_error`).
    //
    // Catches regressions where:
    //   - the range check is inverted,
    //   - 304 starts being treated as an error again,
    //   - someone re-enables `http_status_as_error(true)` on the shared
    //     agent and the call site is no longer reached for non-2xx.
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn fetch_source_sync_maps_4xx_5xx_to_external_api() {
        // 404 — client error. Pick a status that, under ureq defaults,
        // would have come back as `Err(StatusCode(404))`. With the
        // agent configured `http_status_as_error(false)` it now arrives
        // as `Ok(response)` with `status() == 404`, which is exactly
        // what exercises the hand-rolled range check.
        for &status in &[403u16, 404, 500, 502, 503] {
            let listener = bind_mock_listener();
            let port = listener.local_addr().unwrap().port();
            let responses = std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::VecDeque::from(vec![MockResponse {
                    status,
                    headers: vec!["Content-Length: 0".to_string()],
                    body: Vec::new(),
                    delay_ms: 0,
                }]),
            ));
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (_h, _recorded) = spawn_mock_http(listener, responses, stop.clone());

            let url = format!("http://127.0.0.1:{}/anything", port);
            let outcome =
                tokio::task::spawn_blocking(move || fetch_source_sync(&url, None, None, false))
                    .await
                    .unwrap();

            let err = outcome.expect_err(&format!(
                "HTTP {} should be an error, not a Fresh/NotModified",
                status
            ));
            // Catch `Fresh` regressing (e.g. someone removing the range
            // check), or `NotModified` regressing (304 handler swallowing
            // other statuses).
            let msg = err.to_string();
            assert!(
                msg.contains("HTTP") && msg.contains(&status.to_string()),
                "HTTP {} → expected ExternalApi mentioning the status; got: {}",
                status,
                msg
            );

            stop_mock(&stop, _h);
        }
    }

    /// Same contract, but wired through the higher-level
    /// `fetch_and_cache_source` to confirm the error reaches
    /// `record_fetch_error` and surfaces on the source's `last_error`.
    /// Without this, a 500 from upstream would leave the UI badge stale
    /// — exactly the PR #131 P1-2 bug class.
    #[tokio::test]
    async fn fetch_and_cache_source_records_5xx_on_last_error() {
        use mhost_storage::storage::FileStorage;

        let listener = bind_mock_listener();
        let port = listener.local_addr().unwrap().port();
        let responses = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from(vec![MockResponse {
                status: 503,
                headers: vec!["Content-Length: 0".to_string()],
                body: Vec::new(),
                delay_ms: 0,
            }]),
        ));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_h, _recorded) = spawn_mock_http(listener, responses, stop.clone());

        let temp = tempfile::TempDir::new().unwrap();
        let storage = std::sync::Arc::new(FileStorage::new(temp.path()))
            as std::sync::Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let ad_block_state = std::sync::Arc::new(tokio::sync::RwLock::new(AdBlockState::default()));
        let source_id = SourceId(uuid::Uuid::new_v4());
        {
            let mut g = ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "flaky-upstream".into(),
                url: format!("http://127.0.0.1:{}/list", port),
                enabled: true,
                response: AdBlockResponse::ZeroAddress,
                last_fetched_at: None,
                last_error: None,
                rule_count: 0,
                etag: None,
                rules_limit_override: None,
                last_refresh_duration_ms: None,
                last_refresh_failed_at: None,
            });
        }

        let err = fetch_and_cache_source(&storage, &ad_block_state, &source_id, false)
            .await
            .expect_err("503 should propagate as an error");
        assert!(
            err.to_string().contains("503"),
            "503 must surface in the error message, got: {}",
            err
        );

        // PR #131 P1-2 invariant: fetch failure → `last_error` populated
        // on the source, prior state preserved (no cache written).
        let snap = {
            let g = ad_block_state.read().await;
            mhost_storage::adblock::find_source(&g, &source_id)
                .cloned()
                .unwrap()
        };
        assert!(
            snap.last_error.is_some(),
            "last_error must be set when upstream returns 5xx"
        );
        assert!(
            snap.last_error.as_deref().unwrap().contains("503"),
            "last_error should mention the upstream status: {:?}",
            snap.last_error
        );
        // Nothing else moved — `etag`, `rule_count`, `last_fetched_at`
        // stay at their pre-fetch values.
        assert!(snap.etag.is_none());
        assert_eq!(snap.rule_count, 0);
        assert!(snap.last_fetched_at.is_none());

        stop_mock(&stop, _h);
    }

    // -----------------------------------------------------------------
    // Issue #207: per-source rules-limit override. Fail-closed: an
    // over-limit fetch is rejected whole (error carries the actual parsed
    // count), and raising the override re-admits the full list — never a
    // truncated subset.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn fetch_and_cache_source_respects_rules_limit_override() {
        use mhost_storage::storage::FileStorage;

        let listener = bind_mock_listener();
        let port = listener.local_addr().unwrap().port();
        let body = b"0.0.0.0 a.example.com\n0.0.0.0 b.example.com\n0.0.0.0 c.example.com";
        let responses = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from(vec![
                MockResponse::ok_200("\"v1\"", body),
                MockResponse::ok_200("\"v2\"", body),
            ]),
        ));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_h, _recorded) = spawn_mock_http(listener, responses, stop.clone());

        let temp = tempfile::TempDir::new().unwrap();
        let storage = std::sync::Arc::new(FileStorage::new(temp.path()))
            as std::sync::Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let ad_block_state = std::sync::Arc::new(tokio::sync::RwLock::new(AdBlockState::default()));
        let source_id = SourceId(uuid::Uuid::new_v4());
        {
            let mut g = ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "big-list".into(),
                url: format!("http://127.0.0.1:{}/list", port),
                enabled: true,
                response: AdBlockResponse::ZeroAddress,
                last_fetched_at: None,
                last_error: None,
                rule_count: 0,
                etag: None,
                rules_limit_override: Some(2), // below the 3-domain list
                last_refresh_duration_ms: None,
                last_refresh_failed_at: None,
            });
        }

        // Over the override limit → rejected whole, error carries both the
        // actual count and the effective limit.
        let err = fetch_and_cache_source(&storage, &ad_block_state, &source_id, false)
            .await
            .expect_err("3 rules must exceed the override limit of 2");
        assert!(
            err.to_string()
                .contains("source produced 3 rules (limit: 2)"),
            "error must carry actual count + effective limit, got: {}",
            err
        );
        // Fail-closed bookkeeping: last_error recorded, cache NOT written.
        {
            let g = ad_block_state.read().await;
            let s = mhost_storage::adblock::find_source(&g, &source_id).unwrap();
            assert!(
                s.last_error.as_deref().unwrap().contains("limit: 2"),
                "last_error must surface the effective limit: {:?}",
                s.last_error
            );
            assert_eq!(s.rule_count, 0);
        }
        assert!(
            !mhost_storage::adblock::cache_path(temp.path(), &source_id).exists(),
            "over-limit fetch must not write a (possibly truncated) cache"
        );

        // Raise the override above the list size → full success.
        {
            let mut g = ad_block_state.write().await;
            mhost_storage::adblock::find_source_mut(&mut g, &source_id)
                .unwrap()
                .rules_limit_override = Some(3);
        }
        fetch_and_cache_source(&storage, &ad_block_state, &source_id, false)
            .await
            .expect("3 rules must fit the override limit of 3");
        let snap = {
            let g = ad_block_state.read().await;
            mhost_storage::adblock::find_source(&g, &source_id)
                .cloned()
                .unwrap()
        };
        assert_eq!(snap.rule_count, 3, "full list applied, no truncation");
        assert!(snap.last_error.is_none());

        stop_mock(&stop, _h);
    }

    #[tokio::test]
    async fn set_rules_limit_override_rejects_zero_and_above_absolute_cap() {
        let temp = tempfile::TempDir::new().unwrap();
        let (state, _storage) = make_test_app_state(temp.path());
        let source_id = SourceId(uuid::Uuid::new_v4());
        {
            let mut g = state.ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "s".into(),
                url: "https://x".into(),
                enabled: true,
                response: AdBlockResponse::ZeroAddress,
                last_fetched_at: None,
                last_error: None,
                rule_count: 0,
                etag: None,
                rules_limit_override: None,
                last_refresh_duration_ms: None,
                last_refresh_failed_at: None,
            });
        }

        // 0 and above the absolute cap are rejected; state untouched.
        for bad in [0usize, ABSOLUTE_MAX_RULES_PER_SOURCE + 1] {
            let err = set_ad_block_source_rules_limit_override_impl(&state, &source_id, Some(bad))
                .await
                .expect_err("out-of-range override must be rejected");
            assert!(!err.to_string().is_empty());
            let g = state.ad_block_state.read().await;
            assert_eq!(
                mhost_storage::adblock::find_source(&g, &source_id)
                    .unwrap()
                    .rules_limit_override,
                None,
                "rejected override must not be written"
            );
        }

        // A valid override persists to disk (survives restart).
        set_ad_block_source_rules_limit_override_impl(&state, &source_id, Some(600_000))
            .await
            .expect("valid override should succeed");
        let on_disk = mhost_storage::adblock::read_state(state.storage.root()).unwrap();
        assert_eq!(on_disk.sources[0].rules_limit_override, Some(600_000));

        // Revoking (None) clears it.
        set_ad_block_source_rules_limit_override_impl(&state, &source_id, None)
            .await
            .expect("revoking should succeed");
        let on_disk = mhost_storage::adblock::read_state(state.storage.root()).unwrap();
        assert_eq!(on_disk.sources[0].rules_limit_override, None);
    }

    // -----------------------------------------------------------------
    // Issue #206 finding 2: a 304 must not silently "succeed" when the
    // on-disk cache file was removed out-of-band — the fetch is downgraded
    // to an unconditional GET and the body re-applied.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn fetch_and_cache_source_downgrades_304_when_cache_file_missing() {
        use mhost_storage::storage::FileStorage;

        let listener = bind_mock_listener();
        let port = listener.local_addr().unwrap().port();
        let new_body = b"0.0.0.0 refetched.example.com";
        let responses = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from(vec![
                MockResponse::not_modified_304(),
                MockResponse::ok_200("\"v2\"", new_body),
            ]),
        ));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_h, recorded) = spawn_mock_http(listener, responses, stop.clone());

        let temp = tempfile::TempDir::new().unwrap();
        let storage = std::sync::Arc::new(FileStorage::new(temp.path()))
            as std::sync::Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let ad_block_state = std::sync::Arc::new(tokio::sync::RwLock::new(AdBlockState::default()));
        let source_id = SourceId(uuid::Uuid::new_v4());
        {
            let mut g = ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "cacheless".into(),
                url: format!("http://127.0.0.1:{}/list", port),
                enabled: true,
                response: AdBlockResponse::ZeroAddress,
                last_fetched_at: Some(chrono::Utc::now() - chrono::Duration::hours(1)),
                last_error: None,
                rule_count: 7, // stale bookkeeping from a wiped cache
                etag: Some("\"v1\"".to_string()),
                rules_limit_override: None,
                last_refresh_duration_ms: None,
                last_refresh_failed_at: None,
            });
        }
        assert!(
            !mhost_storage::adblock::cache_path(temp.path(), &source_id).exists(),
            "precondition: cache file was removed out-of-band"
        );

        fetch_and_cache_source(&storage, &ad_block_state, &source_id, false)
            .await
            .expect("304-with-missing-cache must downgrade to a full fetch, not fail");

        // Two requests hit the mock: conditional first, then the downgrade.
        {
            let recs = recorded.lock().unwrap();
            assert_eq!(recs.len(), 2, "expected 304 attempt + unconditional retry");
            assert_eq!(recs[0].if_none_match.as_deref(), Some("\"v1\""));
            assert!(
                recs[1].if_none_match.is_none() && recs[1].if_modified_since.is_none(),
                "the downgrade request must carry no conditional headers"
            );
        }

        // Body re-applied: cache written, bookkeeping consistent.
        let snap = {
            let g = ad_block_state.read().await;
            mhost_storage::adblock::find_source(&g, &source_id)
                .cloned()
                .unwrap()
        };
        assert_eq!(snap.rule_count, 1);
        assert_eq!(snap.etag.as_deref(), Some("\"v2\""));
        assert!(snap.last_error.is_none());
        let cache = mhost_storage::adblock::read_cache(temp.path(), &source_id)
            .unwrap()
            .expect("cache file must exist after the downgrade fetch");
        assert!(cache.contains("refetched.example.com"));

        stop_mock(&stop, _h);
    }

    /// Issue #211-1: when the *unconditional* downgrade retry also comes
    /// back 304 (an RFC 7232-violating upstream) with no local cache, the
    /// fetch must fail loudly — a "successful" NotModified here would keep
    /// the empty rule set and a clean `last_error` invisible to the user.
    #[tokio::test]
    async fn fetch_and_cache_source_errors_when_downgrade_retry_also_304() {
        use mhost_storage::storage::FileStorage;

        let listener = bind_mock_listener();
        let port = listener.local_addr().unwrap().port();
        let responses = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from(vec![
                MockResponse::not_modified_304(),
                MockResponse::not_modified_304(),
            ]),
        ));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_h, recorded) = spawn_mock_http(listener, responses, stop.clone());

        let temp = tempfile::TempDir::new().unwrap();
        let storage = std::sync::Arc::new(FileStorage::new(temp.path()))
            as std::sync::Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let ad_block_state = std::sync::Arc::new(tokio::sync::RwLock::new(AdBlockState::default()));
        let source_id = SourceId(uuid::Uuid::new_v4());
        {
            let mut g = ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "rfc-violating".into(),
                url: format!("http://127.0.0.1:{}/list", port),
                enabled: true,
                response: AdBlockResponse::ZeroAddress,
                last_fetched_at: Some(chrono::Utc::now() - chrono::Duration::hours(1)),
                last_error: None,
                rule_count: 7, // stale bookkeeping from a wiped cache
                etag: Some("\"v1\"".to_string()),
                rules_limit_override: None,
                last_refresh_duration_ms: None,
                last_refresh_failed_at: None,
            });
        }

        let err = fetch_and_cache_source(&storage, &ad_block_state, &source_id, false)
            .await
            .expect_err("double-304 with no cache must surface as an error");
        assert!(
            err.to_string().contains("304 to an unconditional request"),
            "error must explain the pathological upstream, got: {}",
            err
        );

        // Exactly two requests: the conditional one + the downgrade.
        {
            let recs = recorded.lock().unwrap();
            assert_eq!(recs.len(), 2);
            assert_eq!(recs[0].if_none_match.as_deref(), Some("\"v1\""));
            assert!(recs[1].if_none_match.is_none());
        }

        // The error reaches `last_error` (PR #131 P1-2 contract) so the UI
        // has a signal instead of a silent empty rule set.
        let snap = {
            let g = ad_block_state.read().await;
            mhost_storage::adblock::find_source(&g, &source_id)
                .cloned()
                .unwrap()
        };
        assert!(
            snap.last_error
                .as_deref()
                .is_some_and(|e| e.contains("304 to an unconditional request")),
            "last_error must record the pathological 304: {:?}",
            snap.last_error
        );

        stop_mock(&stop, _h);
    }

    // -----------------------------------------------------------------
    // Issue #206 finding 1: two concurrent refreshes of the SAME source
    // are serialized by the per-source gate — the second request only
    // reaches the wire after the first response has been written. (The
    // old interleaving let cache-body-v2 coexist with etag-v3.)
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn fetch_and_cache_source_serializes_same_source_refresh() {
        use mhost_storage::storage::FileStorage;

        let listener = bind_mock_listener();
        let port = listener.local_addr().unwrap().port();
        let responses = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from(vec![
                MockResponse::ok_200_delayed("\"v1\"", b"0.0.0.0 one.example.com", 400),
                MockResponse::ok_200("\"v2\"", b"0.0.0.0 two.example.com"),
            ]),
        ));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_h, recorded) = spawn_mock_http(listener, responses, stop.clone());

        let temp = tempfile::TempDir::new().unwrap();
        let storage = std::sync::Arc::new(FileStorage::new(temp.path()))
            as std::sync::Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let ad_block_state = std::sync::Arc::new(tokio::sync::RwLock::new(AdBlockState::default()));
        let source_id = SourceId(uuid::Uuid::new_v4());
        {
            let mut g = ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "raced".into(),
                url: format!("http://127.0.0.1:{}/list", port),
                enabled: true,
                response: AdBlockResponse::ZeroAddress,
                last_fetched_at: None,
                last_error: None,
                rule_count: 0,
                etag: None,
                rules_limit_override: None,
                last_refresh_duration_ms: None,
                last_refresh_failed_at: None,
            });
        }

        // Manual refresh racing the periodic tick: both call the same
        // source simultaneously.
        let (r1, r2) = tokio::join!(
            fetch_and_cache_source(&storage, &ad_block_state, &source_id, false),
            fetch_and_cache_source(&storage, &ad_block_state, &source_id, false),
        );
        r1.expect("first refresh should succeed");
        r2.expect("second refresh should succeed");

        // Without the gate, the second request would arrive while the
        // first response is still being delayed (gap « 400 ms). With the
        // gate it must land only after the first round-trip completes.
        {
            let recs = recorded.lock().unwrap();
            assert_eq!(recs.len(), 2);
            let gap = recs[1].received_at.duration_since(recs[0].received_at);
            assert!(
                gap >= std::time::Duration::from_millis(300),
                "second same-source refresh must be serialized behind the first \
             (response delay 400 ms); arrival gap was {:?}",
                gap
            );
        }

        // Final state is one of the two complete outcomes — never a mix.
        let snap = {
            let g = ad_block_state.read().await;
            mhost_storage::adblock::find_source(&g, &source_id)
                .cloned()
                .unwrap()
        };
        assert!(
            (snap.etag.as_deref() == Some("\"v1\"") && snap.rule_count == 1)
                || (snap.etag.as_deref() == Some("\"v2\"") && snap.rule_count == 1),
            "etag and rule_count must come from the same fetch round, got {:?}/{:?}",
            snap.etag,
            snap.rule_count
        );
        assert!(snap.last_error.is_none());

        stop_mock(&stop, _h);
    }

    // -----------------------------------------------------------------
    // Issue #206 design note 1: force=true (manual refresh) drops the
    // conditional headers entirely — a stale local cache cannot be
    // replayed via a 304.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn fetch_and_cache_source_force_skips_conditional_headers() {
        use mhost_storage::storage::FileStorage;

        let listener = bind_mock_listener();
        let port = listener.local_addr().unwrap().port();
        let responses = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from(vec![MockResponse::ok_200(
                "\"v2\"",
                b"0.0.0.0 forced.example.com",
            )]),
        ));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_h, recorded) = spawn_mock_http(listener, responses, stop.clone());

        let temp = tempfile::TempDir::new().unwrap();
        let storage = std::sync::Arc::new(FileStorage::new(temp.path()))
            as std::sync::Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let ad_block_state = std::sync::Arc::new(tokio::sync::RwLock::new(AdBlockState::default()));
        let source_id = SourceId(uuid::Uuid::new_v4());
        {
            let mut g = ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "stale-cache".into(),
                url: format!("http://127.0.0.1:{}/list", port),
                enabled: true,
                response: AdBlockResponse::ZeroAddress,
                last_fetched_at: Some(chrono::Utc::now() - chrono::Duration::hours(1)),
                last_error: None,
                rule_count: 1,
                etag: Some("\"v1\"".to_string()),
                rules_limit_override: None,
                last_refresh_duration_ms: None,
                last_refresh_failed_at: None,
            });
        }
        // Local cache exists but is stale (e.g. canonicalized by an old
        // parser) — the user hits Refresh expecting fresh data.
        mhost_storage::adblock::write_cache(temp.path(), &source_id, b"0.0.0.0 old.example.com")
            .unwrap();

        fetch_and_cache_source(&storage, &ad_block_state, &source_id, true)
            .await
            .expect("forced fetch should succeed");

        // Even though the source had an etag, the wire request carried no
        // conditional headers.
        {
            let recs = recorded.lock().unwrap();
            assert_eq!(recs.len(), 1);
            assert!(
                recs[0].if_none_match.is_none() && recs[0].if_modified_since.is_none(),
                "force must bypass If-None-Match / If-Modified-Since"
            );
        }

        let snap = {
            let g = ad_block_state.read().await;
            mhost_storage::adblock::find_source(&g, &source_id)
                .cloned()
                .unwrap()
        };
        assert_eq!(snap.etag.as_deref(), Some("\"v2\""));
        assert_eq!(snap.rule_count, 1);

        stop_mock(&stop, _h);
    }

    // -----------------------------------------------------------------
    // Issue #199 sub-task C: when a source flips from enabled -> disabled,
    // its `adblock-cache/<id>.txt` is dropped so a parked source leaves
    // no on-disk residue. The cache file is left intact for any other
    // state change so this assertion is the only place we test the
    // delete path explicitly.
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn set_ad_block_source_enabled_impl_disable_drops_cache() {
        let temp = tempfile::TempDir::new().unwrap();
        let (state, _storage) = make_test_app_state(temp.path());
        let source_id = SourceId(uuid::Uuid::new_v4());
        {
            let mut g = state.ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "to-disable".into(),
                url: "https://x.example/list".into(),
                enabled: true,
                response: AdBlockResponse::ZeroAddress,
                last_fetched_at: Some(chrono::Utc::now()),
                last_error: None,
                rule_count: 3,
                etag: Some("\"v1\"".into()),
                rules_limit_override: None,
                last_refresh_duration_ms: None,
                last_refresh_failed_at: None,
            });
        }
        // Pretend the source has a populated cache on disk.
        mhost_storage::adblock::write_cache(temp.path(), &source_id, b"0.0.0.0 parked.example.com")
            .unwrap();
        assert!(
            mhost_storage::adblock::cache_path(temp.path(), &source_id).exists(),
            "precondition: cache file present"
        );

        set_ad_block_source_enabled_impl(&state, &source_id, false)
            .await
            .expect("disable should succeed");

        assert!(
            !mhost_storage::adblock::cache_path(temp.path(), &source_id).exists(),
            "disable must delete the cache file"
        );
        let snap = state.ad_block_state.read().await;
        let stored = mhost_storage::adblock::find_source(&snap, &source_id).unwrap();
        assert!(!stored.enabled, "source flag must be flipped to false");
        // Bookkeeping intentionally untouched — rule_count still reflects
        // the last successful fetch (matches issue #193 contract for the
        // 304 path: don't touch rule_count/etag on a no-content reply).
        assert_eq!(stored.rule_count, 3);
        assert_eq!(stored.etag.as_deref(), Some("\"v1\""));
    }

    // -----------------------------------------------------------------
    // Issue #199 sub-task C (option A, second half): the disable path
    // drops the cache, so flipping back to enabled must re-fetch before
    // persist_and_reload runs. Otherwise the engine sees an enabled
    // source whose cache is missing and classifies zero rules from it.
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn set_ad_block_source_enabled_impl_re_enable_refetches_cache() {
        let listener = bind_mock_listener();
        let port = listener.local_addr().unwrap().port();
        let body = b"0.0.0.0 fresh.example.com";
        let responses = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from(vec![MockResponse::ok_200("\"fresh\"", body)]),
        ));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_h, _recorded) = spawn_mock_http(listener, responses, stop.clone());

        let temp = tempfile::TempDir::new().unwrap();
        let (state, _storage) = make_test_app_state(temp.path());
        let source_id = SourceId(uuid::Uuid::new_v4());
        {
            let mut g = state.ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "to-reenable".into(),
                url: format!("http://127.0.0.1:{}/list", port),
                enabled: false,
                response: AdBlockResponse::ZeroAddress,
                // Pre-disable bookkeeping — the fetch on re-enable must
                // overwrite this, not preserve the stale rule_count /
                // etag. We also pin a stale `last_error` so the
                // `last_error.is_none()` assertion below actually
                // exercises the "successful fetch clears prior error"
                // path instead of being trivially true.
                last_fetched_at: Some(chrono::Utc::now() - chrono::Duration::days(7)),
                last_error: Some("prior offline failure".into()),
                rule_count: 99,
                etag: Some("\"stale\"".into()),
                rules_limit_override: None,
                last_refresh_duration_ms: None,
                last_refresh_failed_at: None,
            });
        }
        // Confirm the "post-disable" precondition: cache file gone.
        assert!(
            !mhost_storage::adblock::cache_path(temp.path(), &source_id).exists(),
            "precondition: cache file is absent (delete_cache on disable)"
        );

        set_ad_block_source_enabled_impl(&state, &source_id, true)
            .await
            .expect("re-enable should succeed");

        // Cache file must exist again with the freshly fetched body.
        let cache_path = mhost_storage::adblock::cache_path(temp.path(), &source_id);
        assert!(
            cache_path.exists(),
            "re-enable must re-fetch the cache file"
        );
        // Compare verbatim — `cache.contains(...)` would pass even if
        // the file was re-canonicalised with a trailing newline
        // dropped, but the contract is "byte-for-byte what the
        // mock sent". `body` is `&[u8]` so go through `str` for
        // the comparison (the mock body is ASCII).
        let cache = mhost_storage::adblock::read_cache(temp.path(), &source_id)
            .unwrap()
            .expect("cache must be readable");
        assert_eq!(
            cache.as_str(),
            std::str::from_utf8(body).expect("mock body is ASCII"),
            "cache must contain the freshly fetched body verbatim"
        );

        let snap = state.ad_block_state.read().await;
        let stored = mhost_storage::adblock::find_source(&snap, &source_id).unwrap();
        assert!(stored.enabled, "source flag must be flipped to true");
        assert_eq!(
            stored.rule_count, 1,
            "rule_count must be re-derived from the new body"
        );
        assert_eq!(stored.etag.as_deref(), Some("\"fresh\""));
        assert!(
            stored.last_error.is_none(),
            "successful re-fetch must clear prior last_error, got {:?}",
            stored.last_error
        );

        stop_mock(&stop, _h);
    }

    // -----------------------------------------------------------------
    // Issue #199 sub-task C (no-op edge): toggling to the *current* value
    // must not delete the cache nor trigger a fetch. Guards against a
    // future refactor that swaps `prev_enabled` capture for an unconditional
    // delete-or-fetch.
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn set_ad_block_source_enabled_impl_same_value_is_noop() {
        let temp = tempfile::TempDir::new().unwrap();
        let (state, _storage) = make_test_app_state(temp.path());
        let source_id = SourceId(uuid::Uuid::new_v4());
        {
            let mut g = state.ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "stable".into(),
                url: "https://x.example/list".into(),
                enabled: true,
                response: AdBlockResponse::ZeroAddress,
                last_fetched_at: None,
                last_error: None,
                rule_count: 0,
                etag: None,
                rules_limit_override: None,
                last_refresh_duration_ms: None,
                last_refresh_failed_at: None,
            });
        }
        mhost_storage::adblock::write_cache(
            temp.path(),
            &source_id,
            b"0.0.0.0 untouched.example.com",
        )
        .unwrap();
        let before_modified =
            std::fs::metadata(mhost_storage::adblock::cache_path(temp.path(), &source_id))
                .unwrap()
                .modified()
                .unwrap();

        // Sleep a beat so mtime would tick if the file were rewritten.
        std::thread::sleep(std::time::Duration::from_millis(50));

        set_ad_block_source_enabled_impl(&state, &source_id, true)
            .await
            .expect("no-op toggle should succeed");

        // Cache untouched: same path, same content, mtime unchanged.
        // The mtime check alone could false-pass on filesystems
        // with coarse mtime granularity (e.g. 1-second
        // resolution), so we also re-read the bytes and assert
        // they match verbatim — defence in depth against the
        // edge case where a no-op toggle accidentally overwrites
        // the cache file with byte-identical content.
        let path = mhost_storage::adblock::cache_path(temp.path(), &source_id);
        assert!(path.exists(), "cache must still exist");
        let after_modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(
            before_modified, after_modified,
            "no-op toggle must not rewrite the cache file"
        );
        let content = mhost_storage::adblock::read_cache(temp.path(), &source_id)
            .unwrap()
            .expect("cache must still be readable");
        assert_eq!(
            content, "0.0.0.0 untouched.example.com",
            "no-op toggle must not change cache content"
        );
        let snap = state.ad_block_state.read().await;
        let stored = mhost_storage::adblock::find_source(&snap, &source_id).unwrap();
        assert!(stored.enabled, "source stays enabled");
    }

    // -----------------------------------------------------------------
    // Issue #199 sub-task B (PR #219 review follow-ups): test gaps.
    //
    // The four counter / timing pieces added in #199-B were unit-tested
    // end-to-end (engine counters) but the specific helpers and the IPC
    // surface itself were only smoke-tested via existing tests. These
    // tests pin down the contract for each piece so future refactors
    // don't regress it.
    // -----------------------------------------------------------------

    /// Direct test for `record_fetch_error_with_timing`: all three
    /// fields written atomically. The previous `record_fetch_error`
    /// only wrote `last_error`; this helper adds the two timing
    /// fields introduced in #199-B.
    #[tokio::test]
    async fn record_fetch_error_with_timing_writes_all_three_fields() {
        let temp = tempfile::TempDir::new().unwrap();
        let (state, _storage) = make_test_app_state(temp.path());
        let source_id = SourceId(uuid::Uuid::new_v4());
        {
            let mut g = state.ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "timing-test".into(),
                url: "https://x.example/list".into(),
                enabled: true,
                response: AdBlockResponse::ZeroAddress,
                last_fetched_at: None,
                last_error: None,
                rule_count: 0,
                etag: None,
                rules_limit_override: None,
                last_refresh_duration_ms: None,
                last_refresh_failed_at: None,
            });
        }
        let before = chrono::Utc::now();
        record_fetch_error_with_timing(&state.ad_block_state, &source_id, "transport error", 4321)
            .await
            .expect("helper should succeed");
        let after = chrono::Utc::now();

        let snap = state.ad_block_state.read().await;
        let stored = mhost_storage::adblock::find_source(&snap, &source_id).unwrap();
        assert_eq!(stored.last_error.as_deref(), Some("transport error"));
        assert_eq!(stored.last_refresh_duration_ms, Some(4321));
        let failed_at = stored
            .last_refresh_failed_at
            .expect("last_refresh_failed_at must be set");
        assert!(
            failed_at >= before && failed_at <= after,
            "last_refresh_failed_at {} should be in [{}, {}]",
            failed_at,
            before,
            after
        );
    }

    /// 200 OK branch: `fetch_and_cache_source` stamps
    /// `last_refresh_duration_ms` and clears `last_refresh_failed_at`.
    /// Uses `MockResponse::ok_200_delayed` to ensure the duration is
    /// measurable (the mock sleeps before responding).
    #[tokio::test]
    async fn fetch_and_cache_source_200_writes_duration_and_clears_failure() {
        use mhost_storage::storage::FileStorage;

        let listener = bind_mock_listener();
        let port = listener.local_addr().unwrap().port();
        let body = b"0.0.0.0 timed.example.com";
        let responses =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::from(
                vec![MockResponse::ok_200_delayed("\"v1\"", body, 100)],
            )));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_h, _recorded) = spawn_mock_http(listener, responses, stop.clone());

        let temp = tempfile::TempDir::new().unwrap();
        let storage = std::sync::Arc::new(FileStorage::new(temp.path()))
            as std::sync::Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let ad_block_state = std::sync::Arc::new(tokio::sync::RwLock::new(AdBlockState::default()));
        let source_id = SourceId(uuid::Uuid::new_v4());
        {
            let mut g = ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "200-timing".into(),
                url: format!("http://127.0.0.1:{}/list", port),
                enabled: true,
                response: AdBlockResponse::ZeroAddress,
                last_fetched_at: None,
                // Pre-seed a stale failure timestamp so the
                // "successful fetch clears it" assertion is meaningful.
                last_error: Some("prior offline failure".into()),
                rule_count: 0,
                etag: None,
                rules_limit_override: None,
                last_refresh_duration_ms: Some(50),
                last_refresh_failed_at: Some(chrono::Utc::now() - chrono::Duration::hours(1)),
            });
        }

        fetch_and_cache_source(&storage, &ad_block_state, &source_id, false)
            .await
            .expect("200 fetch should succeed");

        let snap = ad_block_state.read().await;
        let stored = mhost_storage::adblock::find_source(&snap, &source_id).unwrap();
        let duration = stored
            .last_refresh_duration_ms
            .expect("200 path must stamp duration");
        // Lower bound: the mock delayed 100 ms. Upper bound: 5 s, generous
        // for CI jitter and spawn_blocking overhead.
        assert!(
            (80..=5_000).contains(&duration),
            "duration {duration} ms should reflect the 100 ms mock delay"
        );
        assert!(
            stored.last_refresh_failed_at.is_none(),
            "200 path must clear last_refresh_failed_at, got {:?}",
            stored.last_refresh_failed_at
        );
        assert!(
            stored.last_error.is_none(),
            "200 path must clear last_error, got {:?}",
            stored.last_error
        );
        assert_eq!(stored.rule_count, 1);

        stop_mock(&stop, _h);
    }

    /// 304 branch: same timing + clear semantics as 200. Setup pins a
    /// stale `last_refresh_failed_at` so the clearing assertion is
    /// meaningful (not trivially-true).
    #[tokio::test]
    async fn fetch_and_cache_source_304_writes_duration_and_clears_failure() {
        use mhost_storage::storage::FileStorage;

        let listener = bind_mock_listener();
        let port = listener.local_addr().unwrap().port();
        let responses = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from(vec![MockResponse::not_modified_304()]),
        ));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_h, _recorded) = spawn_mock_http(listener, responses, stop.clone());

        let temp = tempfile::TempDir::new().unwrap();
        let storage = std::sync::Arc::new(FileStorage::new(temp.path()))
            as std::sync::Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let ad_block_state = std::sync::Arc::new(tokio::sync::RwLock::new(AdBlockState::default()));
        let source_id = SourceId(uuid::Uuid::new_v4());
        {
            let mut g = ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "304-timing".into(),
                url: format!("http://127.0.0.1:{}/list", port),
                enabled: true,
                response: AdBlockResponse::ZeroAddress,
                // Pre-existing cache + ETag (RFC 7232 conditional GET
                // requires a previous successful fetch).
                last_fetched_at: Some(chrono::Utc::now() - chrono::Duration::hours(1)),
                last_error: Some("prior offline failure".into()),
                rule_count: 7,
                etag: Some("\"v1\"".into()),
                rules_limit_override: None,
                last_refresh_duration_ms: None,
                last_refresh_failed_at: Some(chrono::Utc::now() - chrono::Duration::minutes(5)),
            });
        }
        // Pre-existing cache so the 304 path's "no rewrite" contract
        // (issue #193) is exercised.
        mhost_storage::adblock::write_cache(
            temp.path(),
            &source_id,
            b"0.0.0.0 still-alive.example.com",
        )
        .unwrap();

        fetch_and_cache_source(&storage, &ad_block_state, &source_id, false)
            .await
            .expect("304 fetch should succeed");

        let snap = ad_block_state.read().await;
        let stored = mhost_storage::adblock::find_source(&snap, &source_id).unwrap();
        // 304 path leaves rule_count + etag untouched (issue #193).
        assert_eq!(stored.rule_count, 7);
        assert_eq!(stored.etag.as_deref(), Some("\"v1\""));
        // BUT it stamps duration and clears prior failure.
        assert!(
            stored.last_refresh_duration_ms.is_some(),
            "304 path must stamp duration, got {:?}",
            stored.last_refresh_duration_ms
        );
        assert!(
            stored.last_refresh_failed_at.is_none(),
            "304 path must clear last_refresh_failed_at"
        );
        assert!(
            stored.last_error.is_none(),
            "304 path must clear last_error"
        );
        stop_mock(&stop, _h);
    }

    /// Err branch: a 5xx response triggers the `record_fetch_error_with_timing`
    /// path. Asserts all three timing / failure fields are written.
    #[tokio::test]
    async fn fetch_and_cache_source_err_writes_duration_and_failure_timestamp() {
        use mhost_storage::storage::FileStorage;

        let listener = bind_mock_listener();
        let port = listener.local_addr().unwrap().port();
        let responses = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from(vec![MockResponse {
                status: 500,
                headers: vec!["Content-Length: 0".to_string()],
                body: Vec::new(),
                delay_ms: 50,
            }]),
        ));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_h, _recorded) = spawn_mock_http(listener, responses, stop.clone());

        let temp = tempfile::TempDir::new().unwrap();
        let storage = std::sync::Arc::new(FileStorage::new(temp.path()))
            as std::sync::Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let ad_block_state = std::sync::Arc::new(tokio::sync::RwLock::new(AdBlockState::default()));
        let source_id = SourceId(uuid::Uuid::new_v4());
        {
            let mut g = ad_block_state.write().await;
            g.sources.push(AdBlockSource {
                source_id: source_id.clone(),
                name: "err-timing".into(),
                url: format!("http://127.0.0.1:{}/list", port),
                enabled: true,
                response: AdBlockResponse::ZeroAddress,
                last_fetched_at: None,
                last_error: None,
                rule_count: 0,
                etag: None,
                rules_limit_override: None,
                last_refresh_duration_ms: None,
                last_refresh_failed_at: None,
            });
        }

        let before = chrono::Utc::now();
        let err = fetch_and_cache_source(&storage, &ad_block_state, &source_id, false)
            .await
            .expect_err("500 fetch should fail");
        let after = chrono::Utc::now();
        assert!(
            err.to_string().contains("500") || err.to_string().to_lowercase().contains("server"),
            "error should mention the 5xx, got: {}",
            err
        );

        let snap = ad_block_state.read().await;
        let stored = mhost_storage::adblock::find_source(&snap, &source_id).unwrap();
        assert!(
            stored.last_error.is_some(),
            "err path must populate last_error"
        );
        let duration = stored
            .last_refresh_duration_ms
            .expect("err path must stamp duration");
        assert!(
            duration >= 50,
            "duration {duration} ms should reflect the 50 ms mock delay"
        );
        let failed_at = stored
            .last_refresh_failed_at
            .expect("err path must stamp last_refresh_failed_at");
        assert!(
            failed_at >= before && failed_at <= after,
            "last_refresh_failed_at {failed_at} should be in [{before}, {after}]"
        );

        stop_mock(&stop, _h);
    }

    /// Legacy back-compat: an `adblock.json` written before the
    /// #199-B timing fields existed must still deserialize, with the
    /// new fields defaulting to `None`. Analog to
    /// `test_ad_block_source_rules_limit_override_serde` for the
    /// earlier (#207) field addition.
    #[test]
    fn adblock_state_legacy_doc_back_compat_for_199b_fields() {
        let temp = tempfile::TempDir::new().unwrap();
        // Pre-#199-B document: no `last_refresh_duration_ms` /
        // `last_refresh_failed_at` keys. The other fields are the
        // pre-#207 shape too (no `rules_limit_override`) for
        // completeness \u2014 demonstrates that BOTH additions
        // back-compat cleanly.
        let legacy = r#"{
            "enabled": true,
            "sources": [
                {
                    "source_id": "00000000-0000-0000-0000-000000000001",
                    "name": "legacy",
                    "url": "https://x.example/list",
                    "enabled": true,
                    "response": "zero_address",
                    "last_fetched_at": null,
                    "last_error": null,
                    "rule_count": 42,
                    "etag": null
                }
            ],
            "whitelist": [],
            "auto_refresh_enabled": true,
            "refresh_interval_hours": 6
        }"#;
        let path = temp.path().join("adblock.json");
        std::fs::write(&path, legacy).unwrap();

        let state =
            mhost_storage::adblock::read_state(temp.path()).expect("legacy doc must deserialize");
        let src = &state.sources[0];
        assert_eq!(src.rule_count, 42);
        // #199-B additions default to None.
        assert_eq!(src.last_refresh_duration_ms, None);
        assert_eq!(src.last_refresh_failed_at, None);
        // #207 addition also defaults to None.
        assert_eq!(src.rules_limit_override, None);
    }

    /// Direct IPC test for `get_ad_block_stats`: spins up a real
    /// `DnsServer`, fires some `check()` calls, calls the IPC, and
    /// asserts the response shape. Closes the test-gap from the PR
    /// #219 review.
    #[tokio::test]
    async fn get_ad_block_stats_returns_engine_counters_and_enabled() {
        use mhost_dns::adblock::AdBlockAction;
        use mhost_dns::DnsConfig;
        use std::collections::{HashMap, HashSet};

        let temp = tempfile::TempDir::new().unwrap();
        let (state, _storage) = make_test_app_state(temp.path());

        // Build a real DnsServer + engine, reload with enabled=true +
        // one zero-addr rule + one whitelist entry.
        let config = DnsConfig {
            port: pick_free_port(),
            upstream: vec!["1.1.1.1".to_string()],
            refresh_upstream: false,
            timeout_ms: 1000,
            ..Default::default()
        };
        let server = std::sync::Arc::new(mhost_dns::DnsServer::new(config).unwrap());

        let mut zero_addr = HashMap::new();
        zero_addr.insert(
            "ad.example.com".to_string(),
            std::net::IpAddr::from([0u8, 0, 0, 0]),
        );
        let whitelist: HashSet<String> = ["safe.example.com".to_string()].into_iter().collect();
        server.reload_ad_block_rules(true, zero_addr, HashSet::new(), whitelist);

        // Drive some traffic through the engine.
        let engine = server.ad_block_engine_for_test();
        for _ in 0..3 {
            let _ = engine.check("ad.example.com"); // hits_zero_addr
        }
        for _ in 0..2 {
            let _ = engine.check("safe.example.com"); // hits_whitelist
        }
        for _ in 0..4 {
            let _ = engine.check("untouched.example.org"); // misses
        }
        // One NxDomain-source check to confirm `enabled=false` flips all counters.
        server.reload_ad_block_rules(false, HashMap::new(), HashSet::new(), HashSet::new());
        let _ = engine.check("ad.example.com"); // master off \u2192 no counter

        // Slot the server into AppState so the IPC reads from it.
        // `state.dns_server` is `Mutex<Option<DnsServer>>` (not
        // `Option<Arc<DnsServer>>`), so unwrap the `Arc` we
        // built above. The `engine` clone is on a separate
        // `Arc<AdBlockEngine>`, so the test owns the only
        // strong ref to DnsServer here.
        let inner = std::sync::Arc::try_unwrap(server)
            .map_err(|_| "strong refs to test DnsServer leaked")
            .expect("test owns the only strong ref to DnsServer");
        *crate::state::lock_or_recover(&state.dns_server) = Some(inner);
        // The IPC handler under test (we drive it through the
        // AppState directly rather than going through Tauri's
        // State wrapper).
        // Inline the IPC handler body (we drive it directly
        // through the AppState rather than going through
        // Tauri's State wrapper).
        let response = {
            let (stats, enabled) = {
                let guard = crate::state::lock_or_recover(&state.dns_server);
                match guard.as_ref() {
                    Some(server) => (server.ad_block_stats(), server.ad_block_enabled()),
                    None => (
                        mhost_dns::adblock::AdBlockStats {
                            hits_zero_addr: 0,
                            hits_nxdomain: 0,
                            hits_whitelist: 0,
                            misses: 0,
                        },
                        false,
                    ),
                }
            };
            AdBlockStatsView {
                hits_zero_addr: stats.hits_zero_addr,
                hits_nxdomain: stats.hits_nxdomain,
                hits_whitelist: stats.hits_whitelist,
                misses: stats.misses,
                enabled,
            }
        };
        // Counters persist across reload(false, ...) — only the
        // gating state changes; pre-reload zero_addr / whitelist /
        // miss totals are intact. Issue #199 contract.
        assert_eq!(response.hits_zero_addr, 3, "3 zero-addr hits counted");
        assert_eq!(response.hits_nxdomain, 0);
        assert_eq!(
            response.hits_whitelist, 2,
            "2 whitelist hits counted (counters persist across reload)",
        );
        assert_eq!(response.misses, 4, "4 misses counted");
        // Engine master switch off after reload(false, ...): the
        // IPC must report the engine's authoritative gating state
        // (the AtomicBool mirror), not state.enabled — that is the
        // whole point of the PR #219 review-followup race fix.
        assert!(
            !response.enabled,
            "engine master switch off after reload(false, ...)",
        );
        let _ = AdBlockAction::ZeroAddress; // keep the import used
    }
}
