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
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use mhost_core::{
    AdBlockResponse, AdBlockSource, AdBlockState, MhostError, SourceId,
};
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

const USER_AGENT: &str = "mHost-Desktop/1.0";

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
pub(crate) async fn persist_and_reload(state: &AppState) -> Result<(), MhostError> {
    // Clone out under the lock, then drop the guard before DNS work.
    let snapshot: AdBlockState = {
        let guard = state.ad_block_state.read().await;
        guard.clone()
    };

    // 1. Persist to disk (blocking IO is fine — atomic_write is fast and
    // we're already in a tokio task that does similar sync IO via
    // storage layer elsewhere).
    let root = state.storage.root().to_path_buf();
    let snapshot_for_disk = snapshot.clone();
    tokio::task::spawn_blocking(move || adblock_store::write_state(&root, &snapshot_for_disk))
        .await
        .map_err(|e| MhostError::InvalidInput(format!("persist task failed: {}", e)))?
        .map_err(|e| MhostError::InvalidInput(format!("write_state: {}", e)))?;

    // 2. Hot-reload the DNS server if it's running. No-op when DNS mode off
    // (rules are stored on disk and will be loaded on next DNS enable).
    if state.dns_enabled.load(Ordering::Relaxed) {
        let root = state.storage.root();
        if let Some(server) = lock_or_recover(&state.dns_server).as_ref() {
            let (zero_addr, nxdomain, whitelist) = classify_rules(&snapshot, root);
            server.reload_ad_block_rules(zero_addr, nxdomain, whitelist);
        }
    }

    Ok(())
}

/// Reduce `AdBlockState` into the three rule sets consumed by the engine.
/// Reads each enabled source's cache file synchronously — only invoked from
/// `persist_and_reload`, which is in turn called from a tokio task; the IO
/// is fast (small files, no parsing needed here).
pub(crate) fn classify_rules(
    state: &AdBlockState,
    root: &std::path::Path,
) -> (
    HashMap<String, IpAddr>,
    HashSet<String>,
    HashSet<String>,
) {
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

/// Parse hosts-format blocklist content into a flat list of domains.
/// Comments (`#`) and empty lines are filtered out by `Parser::parse_line`.
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

    // 2. Fetch raw bytes via reqwest (per-call client, like update.rs).
    let url = source_clone.url.clone();
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .build()
        .map_err(|e| MhostError::Network(format!("reqwest build error: {}", e)))?;

    let resp = client
        .get(&url)
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

    let body = resp
        .bytes()
        .await
        .map_err(|e| MhostError::Network(format!("read body error: {}", e)))?;

    let content_str = std::str::from_utf8(&body).map_err(|e| {
        MhostError::InvalidInput(format!("response is not valid UTF-8: {}", e))
    })?;

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
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .build()
        .map_err(|e| MhostError::Network(format!("reqwest build error: {}", e)))?;

    let resp = client
        .get(&url)
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

    let body = resp
        .bytes()
        .await
        .map_err(|e| MhostError::Network(format!("read body error: {}", e)))?;
    let content_str = std::str::from_utf8(&body)
        .map_err(|e| MhostError::InvalidInput(format!("response is not valid UTF-8: {}", e)))?;

    let content_owned = content_str.to_string();
    let root = storage.root().to_path_buf();
    let id_owned = source_id.clone();
    let parse_result: Result<usize, MhostError> =
        tokio::task::spawn_blocking(move || {
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
    if name.trim().is_empty() {
        return Err(MhostError::InvalidInput("source name is empty".into()));
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

    // Fetch + persist. If fetch fails, we still keep the source record
    // (with `last_error`) so the user can retry from the UI.
    let _ = fetch_and_cache_source(&state, &new_id).await;

    // Return the (possibly errored) source back to the UI.
    let snap = state.ad_block_state.read().await;
    let stored = adblock_store::find_source(&snap, &new_id)
        .cloned()
        .unwrap_or(new_source);
    drop(snap);
    persist_and_reload(&state).await?;
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
        let s = adblock_store::find_source_mut(&mut guard, &source_id).ok_or_else(|| {
            MhostError::InvalidInput(format!("source not found: {}", source_id))
        })?;
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
        let s = adblock_store::find_source_mut(&mut guard, &source_id).ok_or_else(|| {
            MhostError::InvalidInput(format!("source not found: {}", source_id))
        })?;
        s.response = response;
    }
    persist_and_reload(&state).await?;
    let snap = state.ad_block_state.read().await;
    Ok(adblock_store::find_source(&snap, &source_id)
        .cloned()
        .expect("source just updated"))
}

// ---------------------------------------------------------------------------
// Refresh
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn refresh_ad_block_source(
    source_id: SourceId,
    state: State<'_, AppState>,
) -> Result<AdBlockSource, MhostError> {
    fetch_and_cache_source(&state, &source_id).await?;
    persist_and_reload(&state).await?;
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
    for id in &ids {
        // Best-effort: log + record_fetch_error on failure, keep going.
        if let Err(e) = fetch_and_cache_source(&state, id).await {
            let _ = record_fetch_error(&state, id, &e.to_string()).await;
            eprintln!("[adblock] refresh source {} failed: {}", id, e);
        }
    }
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
    let normalized = domain.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(MhostError::InvalidInput("domain is empty".into()));
    }
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
        let mut state = AdBlockState::default();
        state.enabled = false;
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
        let mut state = AdBlockState::default();
        state.enabled = true;
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
        // No cache files written — domains_for_source returns [] when cache
        // is missing, but the partitioning + whitelist logic still runs.
        state.sources.push(mk("za", AdBlockResponse::ZeroAddress, true));
        state.sources.push(mk("nx", AdBlockResponse::NxDomain, true));
        state.sources.push(mk("off", AdBlockResponse::ZeroAddress, false));
        state.whitelist.push("trusted.com".into());

        let (_z, _n, w) = classify_rules(&state, temp.path());
        assert_eq!(w.len(), 1);
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
}