//! Persistence layer for the DNS-mode ad block subsystem (issue #130).
//!
//! Layout under the storage root:
//!
//! ```text
//! {root}/
//!   adblock.json              # AdBlockState (single small JSON document)
//!   adblock-cache/{id}.txt    # raw fetched blocklist per source
//! ```
//!
//! All writes use `atomic_write` (defined in `storage.rs`) so a crash during
//! write never leaves a half-written config or cache file. Reads return
//! `AdBlockState::default()` when the JSON is missing — ad block is opt-in
//! so first run is fine with no file at all.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use mhost_core::{AdBlockSource, AdBlockState, SourceId};

use super::storage::atomic_write;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const STATE_FILE: &str = "adblock.json";
const CACHE_DIR: &str = "adblock-cache";

// ---------------------------------------------------------------------------
// State (adblock.json)
// ---------------------------------------------------------------------------

/// Read the persisted ad block state. Returns `AdBlockState::default()` if
/// the file does not exist (first run, ad block never enabled).
pub fn read_state(root: &Path) -> io::Result<AdBlockState> {
    let path = root.join(STATE_FILE);
    match fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("adblock.json is corrupted: {}", e),
            )
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(AdBlockState::default()),
        Err(e) => Err(e),
    }
}

/// Atomically write the ad block state.
pub fn write_state(root: &Path, state: &AdBlockState) -> io::Result<()> {
    let path = root.join(STATE_FILE);
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    atomic_write(&path, json.as_bytes())
}

// ---------------------------------------------------------------------------
// Source cache (adblock-cache/{id}.txt)
// ---------------------------------------------------------------------------

fn cache_dir(root: &Path) -> PathBuf {
    root.join(CACHE_DIR)
}

fn cache_path(root: &Path, source_id: &SourceId) -> PathBuf {
    cache_dir(root).join(format!("{}.txt", source_id))
}

/// Write the raw fetched blocklist content for a source. Atomic — partial
/// writes never leave a torn cache file.
pub fn write_cache(root: &Path, source_id: &SourceId, content: &[u8]) -> io::Result<()> {
    fs::create_dir_all(cache_dir(root))?;
    let path = cache_path(root, source_id);
    atomic_write(&path, content)
}

/// Read the raw blocklist content for a source. Returns `None` if no cache
/// has ever been written for this source.
pub fn read_cache(root: &Path, source_id: &SourceId) -> io::Result<Option<String>> {
    let path = cache_path(root, source_id);
    match fs::read_to_string(&path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Remove the cache file for a source. Idempotent — a missing file is OK.
pub fn delete_cache(root: &Path, source_id: &SourceId) -> io::Result<()> {
    let path = cache_path(root, source_id);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Bulk delete helpers (used when a source is removed)
// ---------------------------------------------------------------------------

/// Remove both the source's cache file and ensure the source's id is no
/// longer present in `state.sources`. Caller is responsible for `write_state`
/// after this call.
pub fn purge_source(root: &Path, state: &mut AdBlockState, source_id: &SourceId) -> io::Result<()> {
    state.sources.retain(|s| &s.source_id != source_id);
    delete_cache(root, source_id)
}

// ---------------------------------------------------------------------------
// Convenience: list sources from state (re-exported to avoid churn)
// ---------------------------------------------------------------------------

/// Find a source by id. Linear scan; expected N is small (single digits).
pub fn find_source<'a>(state: &'a AdBlockState, id: &SourceId) -> Option<&'a AdBlockSource> {
    state.sources.iter().find(|s| &s.source_id == id)
}

/// Mutable equivalent of [`find_source`].
pub fn find_source_mut<'a>(
    state: &'a mut AdBlockState,
    id: &SourceId,
) -> Option<&'a mut AdBlockSource> {
    state.sources.iter_mut().find(|s| &s.source_id == id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mhost_core::AdBlockResponse;
    use tempfile::TempDir;

    fn sample_source(name: &str) -> AdBlockSource {
        AdBlockSource {
            source_id: SourceId(uuid::Uuid::new_v4()),
            name: name.to_string(),
            url: format!("https://example.com/{}.txt", name),
            enabled: true,
            response: AdBlockResponse::ZeroAddress,
            last_fetched_at: None,
            last_error: None,
            rule_count: 0,
            etag: None,
        }
    }

    #[test]
    fn read_state_returns_default_when_missing() {
        let temp = TempDir::new().unwrap();
        let state = read_state(temp.path()).unwrap();
        assert_eq!(state, AdBlockState::default());
    }

    #[test]
    fn write_then_read_state_roundtrip() {
        let temp = TempDir::new().unwrap();
        let mut state = AdBlockState::default();
        state.enabled = true;
        state.sources.push(sample_source("a"));
        state.whitelist.push("trusted.example.com".to_string());

        write_state(temp.path(), &state).unwrap();
        let restored = read_state(temp.path()).unwrap();
        assert_eq!(state, restored);
    }

    #[test]
    fn write_state_is_atomic_no_tmp_files_leaked() {
        let temp = TempDir::new().unwrap();
        let state = AdBlockState::default();
        write_state(temp.path(), &state).unwrap();
        // atomic_write uses tempfile::NamedTempFile which cleans up on drop;
        // assert no stray .tmp file remains at root.
        let stray: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "tmp")
                    .unwrap_or(false)
            })
            .collect();
        assert!(stray.is_empty(), "found stray tmp files: {:?}", stray);
    }

    #[test]
    fn cache_write_read_delete_roundtrip() {
        let temp = TempDir::new().unwrap();
        let id = SourceId(uuid::Uuid::new_v4());

        // Missing → None
        assert!(read_cache(temp.path(), &id).unwrap().is_none());

        // Write + read
        write_cache(temp.path(), &id, b"0.0.0.0 ad.example\n").unwrap();
        let content = read_cache(temp.path(), &id).unwrap().unwrap();
        assert_eq!(content, "0.0.0.0 ad.example\n");

        // Delete
        delete_cache(temp.path(), &id).unwrap();
        assert!(read_cache(temp.path(), &id).unwrap().is_none());

        // Delete again is idempotent
        delete_cache(temp.path(), &id).unwrap();
    }

    #[test]
    fn purge_source_removes_cache_and_listing() {
        let temp = TempDir::new().unwrap();
        let mut state = AdBlockState::default();
        let s1 = sample_source("keep");
        let s2 = sample_source("drop");
        let keep_id = s1.source_id.clone();
        let drop_id = s2.source_id.clone();
        state.sources.push(s1);
        state.sources.push(s2);
        write_cache(temp.path(), &keep_id, b"keep").unwrap();
        write_cache(temp.path(), &drop_id, b"drop").unwrap();

        purge_source(temp.path(), &mut state, &drop_id).unwrap();

        assert!(read_cache(temp.path(), &drop_id).unwrap().is_none());
        assert_eq!(state.sources.len(), 1);
        assert_eq!(state.sources[0].source_id, keep_id);
        // Keep cache intact
        assert!(read_cache(temp.path(), &keep_id).unwrap().is_some());
    }

    #[test]
    fn read_state_corrupted_json_returns_error() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join(STATE_FILE), b"{not valid json").unwrap();
        let err = read_state(temp.path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn write_cache_creates_cache_dir() {
        let temp = TempDir::new().unwrap();
        let id = SourceId(uuid::Uuid::new_v4());
        write_cache(temp.path(), &id, b"x").unwrap();
        assert!(cache_dir(temp.path()).is_dir());
    }
}
