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
use tauri::State;
use uuid::Uuid;

use crate::state::{lock_or_recover, AppState};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Hard upper bound on rules per source. The classic anti-pattern (issue #130
/// background) was writing 100k+ hosts entries into `/etc/hosts`; we keep the
/// same ceiling at the parser layer so a misconfigured source can't OOM the
/// process.
const MAX_RULES_PER_SOURCE: usize = 100_000;

/// HTTP fetch timeout. Blocklist refresh shouldn't block the UI thread;
/// 30 s is generous for typical hosts-format payloads.
const FETCH_TIMEOUT_SECS: u64 = 30;

/// Maximum response body size for an ad-block source (PR #131 review
/// finding 1.8 — a malicious or unbounded-misconfigured source can still
/// run for `FETCH_TIMEOUT_SECS` and start streaming bytes; cap the bytes).
/// `MAX_RULES_PER_SOURCE × ~30 bytes ≈ 3 MB`; 16 MB headroom is enough for
/// legitimate (well-annotated) lists.
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

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
// Shared HTTP client (PR #131 review findings 1.8 + 1.9)
// ---------------------------------------------------------------------------

/// Process-wide shared `reqwest::Client`. Building a client is non-trivial
/// (TLS keylog, DNS resolver, connection pool) and we were doing it on every
/// fetch + every background refresh tick. Reusing one client also means
/// HTTP keep-alive across fetches and a bounded connection pool.
///
/// `OnceLock::get_or_init` runs the closure synchronously on the first
/// call; all subsequent calls return the same `&'static` handle. Safe
/// because `reqwest::Client::build()` is sync.
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
            // reqwest 0.13 lacks `body_limit`; we enforce MAX_RESPONSE_BYTES
            // explicitly via `fetch_source_content` (early reject via the
            // Content-Length header + post-read size check).
            .build()
            .expect("reqwest client build must succeed with static config")
    })
}

/// Fetch `url` via the shared client. Returns the raw body bytes plus the
/// `ETag` header (if any), and rejects anything larger than
/// `MAX_RESPONSE_BYTES`. Two-stage guard:
///
///   1. Server-advertised `Content-Length` → reject without downloading.
///   2. Post-read size check → catches servers that lie about length.
///
/// PR #131 review finding 1.8 — a malicious or misconfigured source can
/// otherwise stream arbitrary bytes for `FETCH_TIMEOUT_SECS` before our
/// parser sees them.
async fn fetch_source(url: &str) -> Result<(Vec<u8>, Option<String>), MhostError> {
    let resp = http_client()
        .get(url)
        .send()
        .await
        .map_err(|e| MhostError::Network(format!("network error: {}", e)))?;
    if !resp.status().is_success() {
        return Err(MhostError::ExternalApi(format!(
            "fetch {} failed: HTTP {}",
            url,
            resp.status()
        )));
    }
    let etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    if let Some(len) = resp.content_length() {
        if len > MAX_RESPONSE_BYTES as u64 {
            return Err(MhostError::InvalidInput(format!(
                "source body length {} exceeds limit {}",
                len, MAX_RESPONSE_BYTES
            )));
        }
    }
    let body = resp
        .bytes()
        .await
        .map_err(|e| MhostError::Network(format!("read body error: {}", e)))?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(MhostError::InvalidInput(format!(
            "source body received {} bytes, exceeds limit {}",
            body.len(),
            MAX_RESPONSE_BYTES
        )));
    }
    Ok((body.to_vec(), etag))
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
                    server.reload_ad_block_rules(zero_addr, nxdomain, whitelist);
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
/// `walk_parents` (it does literal `HashSet::contains`). They also
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
/// content + parsed-domain count back into `state.sources[i]`. The hot-reload
/// is the caller's responsibility (use `persist_and_reload` after).
pub(crate) async fn fetch_and_cache_source(
    state: &AppState,
    source_id: &SourceId,
) -> Result<(), MhostError> {
    // 1. Read the source record under the read lock.
    let source_clone = {
        let guard = state.ad_block_state.read().await;
        match adblock_store::find_source(&guard, source_id) {
            Some(s) => s.clone(),
            None => {
                return Err(MhostError::InvalidInput(format!(
                    "ad block source not found: {}",
                    source_id
                )))
            }
        }
    };

    // 2. Fetch raw bytes via the shared reqwest client (PR #131 review
    // finding 1.9). The static client is built once; size enforcement
    // happens inside `fetch_source` (PR #131 review finding 1.8).
    //
    // PR #131 re-review P1-2: record a fetch failure on the source's
    // `last_error` before propagating — the parse-failure branch below
    // already did this, but a network/size failure returned via `?` with no
    // record, so the UI badge and the persisted state both stayed stale.
    let url = source_clone.url.clone();
    let body_and_etag = fetch_source(&url).await;
    let (body, etag) = match body_and_etag {
        Ok(v) => v,
        Err(e) => {
            let msg = e.to_string();
            let _ = record_fetch_error(state, source_id, &msg).await;
            return Err(e);
        }
    };
    let content_str = std::str::from_utf8(&body)
        .map_err(|e| MhostError::InvalidInput(format!("response is not valid UTF-8: {}", e)))?;

    // 3. Parse + enforce hard limit. Use spawn_blocking because the parser
    // is sync and the input can be large.
    let content_owned = content_str.to_string();
    let root = state.storage.root().to_path_buf();
    let id_owned = source_id.clone();
    let parse_result: Result<(usize, Vec<u8>), MhostError> =
        tokio::task::spawn_blocking(move || {
            let domains = parse_blocklist_domains(&content_owned);
            if domains.len() > MAX_RULES_PER_SOURCE {
                return Err(MhostError::InvalidInput(format!(
                    "source produced {} rules (limit: {})",
                    domains.len(),
                    MAX_RULES_PER_SOURCE
                )));
            }
            // Re-serialize as canonical hosts text so the cache is always
            // valid hosts format (drops comments the original may have).
            let canon = domains
                .iter()
                .map(|d| format!("0.0.0.0 {}", d))
                .collect::<Vec<_>>()
                .join("\n");
            adblock_store::write_cache(&root, &id_owned, canon.as_bytes())?;
            Ok((domains.len(), Vec::new()))
        })
        .await
        .map_err(|e| MhostError::InvalidInput(format!("parse task failed: {}", e)))?;

    let rule_count = match parse_result {
        Ok((count, _)) => count,
        Err(e) => {
            // Persist the failure on the source so the UI can show it,
            // but keep the previous cache intact for DNS to keep working.
            record_fetch_error(state, source_id, &e.to_string()).await?;
            return Err(e);
        }
    };

    // 4. Update source record: clear error, set fetched_at, rule_count, etag.
    {
        let mut guard = state.ad_block_state.write().await;
        if let Some(s) = adblock_store::find_source_mut(&mut guard, source_id) {
            s.last_error = None;
            s.last_fetched_at = Some(Utc::now());
            s.rule_count = rule_count;
            s.etag = etag;
        }
    }
    Ok(())
}

/// Persist an error string onto a source's `last_error` field. Does NOT
/// touch `last_fetched_at` or `rule_count` — those reflect the last
/// successful fetch and should be preserved on failure.
pub(crate) async fn record_fetch_error(
    state: &AppState,
    source_id: &SourceId,
    err: &str,
) -> Result<(), MhostError> {
    record_fetch_error_internal(&state.ad_block_state, source_id, err).await
}

/// `AppState`-free variant for the background refresh task (which clones
/// just the Arcs it needs at spawn time).
pub(crate) async fn record_fetch_error_internal(
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

/// `AppState`-free variant of `fetch_and_cache_source` for the background
/// refresh task. Skips the proxy detection of "is DNS still on" — that's
/// checked at the call site in `dns.rs` before invoking this.
pub(crate) async fn fetch_and_cache_source_internal(
    storage: &Arc<dyn mhost_storage::storage::Storage + Send + Sync>,
    ad_block_state: &Arc<tokio::sync::RwLock<AdBlockState>>,
    source_id: &SourceId,
) -> Result<(), MhostError> {
    let source_clone = {
        let guard = ad_block_state.read().await;
        match adblock_store::find_source(&guard, source_id) {
            Some(s) => s.clone(),
            None => {
                return Err(MhostError::InvalidInput(format!(
                    "ad block source not found: {}",
                    source_id
                )))
            }
        }
    };

    let url = source_clone.url.clone();
    let (body, etag) = fetch_source(&url).await?;
    let content_str = std::str::from_utf8(&body)
        .map_err(|e| MhostError::InvalidInput(format!("response is not valid UTF-8: {}", e)))?;

    let content_owned = content_str.to_string();
    let root = storage.root().to_path_buf();
    let id_owned = source_id.clone();
    let parse_result: Result<usize, MhostError> = tokio::task::spawn_blocking(move || {
        let domains = parse_blocklist_domains(&content_owned);
        if domains.len() > MAX_RULES_PER_SOURCE {
            return Err(MhostError::InvalidInput(format!(
                "source produced {} rules (limit: {})",
                domains.len(),
                MAX_RULES_PER_SOURCE
            )));
        }
        let canon = domains
            .iter()
            .map(|d| format!("0.0.0.0 {}", d))
            .collect::<Vec<_>>()
            .join("\n");
        adblock_store::write_cache(&root, &id_owned, canon.as_bytes())?;
        Ok(domains.len())
    })
    .await
    .map_err(|e| MhostError::InvalidInput(format!("parse task failed: {}", e)))?;

    let rule_count = match parse_result {
        Ok(n) => n,
        Err(e) => {
            record_fetch_error_internal(ad_block_state, source_id, &e.to_string()).await?;
            return Err(e);
        }
    };

    {
        let mut guard = ad_block_state.write().await;
        if let Some(s) = adblock_store::find_source_mut(&mut guard, source_id) {
            s.last_error = None;
            s.last_fetched_at = Some(Utc::now());
            s.rule_count = rule_count;
            s.etag = etag;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Return the full ad block state (sources + whitelist + meta).
#[tauri::command]
pub async fn get_ad_block_state(state: State<'_, AppState>) -> Result<AdBlockState, MhostError> {
    Ok(state.ad_block_state.read().await.clone())
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
    persist_and_reload(&state).await
}

/// Change the auto-refresh interval in hours. `0` disables background
/// refresh (frontend shows a hint to refresh manually).
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
    let fetch_result = fetch_and_cache_source(state, &new_id).await;
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
    {
        let mut guard = state.ad_block_state.write().await;
        let s = adblock_store::find_source_mut(&mut guard, &source_id)
            .ok_or_else(|| MhostError::InvalidInput(format!("source not found: {}", source_id)))?;
        s.enabled = enabled;
    }
    persist_and_reload(&state).await?;
    let snap = state.ad_block_state.read().await;
    Ok(adblock_store::find_source(&snap, &source_id)
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

// ---------------------------------------------------------------------------
// Refresh (concurrent)
// ---------------------------------------------------------------------------

/// Fan out `fetch_and_cache_source_internal` over `source_ids` with bounded
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
            if let Err(e) = fetch_and_cache_source_internal(&storage, &ad_block_state, &id).await {
                let _ = record_fetch_error_internal(&ad_block_state, &id, &e.to_string()).await;
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
    let fetch_result = fetch_and_cache_source(&state, &source_id).await;
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
    // failures are recorded on `last_error` via the helper.
    fetch_sources_concurrent(
        &state.storage,
        &state.ad_block_state,
        &ids,
        REFRESH_CONCURRENCY,
    )
    .await;
    persist_and_reload(&state).await?;
    Ok(state.ad_block_state.read().await.sources.clone())
}

// ---------------------------------------------------------------------------
// Whitelist
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_ad_block_whitelist(
    state: State<'_, AppState>,
) -> Result<Vec<String>, MhostError> {
    Ok(state.ad_block_state.read().await.whitelist.clone())
}

#[tauri::command]
pub async fn add_ad_block_whitelist(
    domain: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, MhostError> {
    let normalized = validate_whitelist_domain(&domain).map_err(MhostError::InvalidInput)?;
    {
        let mut guard = state.ad_block_state.write().await;
        if !guard.whitelist.contains(&normalized) {
            guard.whitelist.push(normalized);
        }
    }
    persist_and_reload(&state).await?;
    Ok(state.ad_block_state.read().await.whitelist.clone())
}

#[tauri::command]
pub async fn remove_ad_block_whitelist(
    domain: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, MhostError> {
    // Removal tolerates the same input the user typed when adding (i.e.
    // no validation — silently no-op on missing). This matches the
    // contract of "remove what matches; ignore the rest".
    let normalized = domain.trim().to_lowercase();
    {
        let mut guard = state.ad_block_state.write().await;
        guard.whitelist.retain(|d| d != &normalized);
    }
    persist_and_reload(&state).await?;
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
        });
        let (z, n, w) = classify_rules(&state, temp.path());
        assert!(z.is_empty());
        assert!(n.is_empty());
        assert!(w.is_empty());
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
        server.reload_ad_block_rules(za, nx, wl);
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
    #[tokio::test]
    async fn add_ad_block_source_persists_on_fetch_failure() {
        use crate::state::AppState;
        use mhost_apply::writer::HostsWriter;
        use mhost_storage::storage::FileStorage;

        let temp = tempfile::TempDir::new().unwrap();
        let storage = Arc::new(FileStorage::new(temp.path()))
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
        };

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
}
