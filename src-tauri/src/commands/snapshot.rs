use chrono::{DateTime, Utc};
use mhost_apply::writer::HostsWriter;
use mhost_core::{MhostError, ProfileMode, Snapshot, SnapshotMeta};
use mhost_storage::storage::{write_atomic_0600, Storage};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};

use crate::state::{lock_or_recover, AppState, ApplyLock};

const MAX_SNAPSHOTS: usize = 20;
const MAX_SNAPSHOT_NAME_LENGTH: usize = 100;
const MAX_SNAPSHOT_DESC_LENGTH: usize = 500;
const SNAPSHOT_INDEX_VERSION: u32 = 1;
const SNAPSHOT_INDEX_FILE: &str = "index.json";
const SNAPSHOT_TRANSACTION_DIR: &str = "pending";
const SNAPSHOT_TRANSACTION_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// ID validation
// ---------------------------------------------------------------------------

/// Validate that a snapshot id is a valid UUID v4 string.
/// Security fix (B1): Prevents path traversal via malicious id values.
fn validate_snapshot_id(id: &str) -> Result<(), MhostError> {
    if uuid::Uuid::parse_str(id).is_err() {
        return Err(MhostError::InvalidInput(format!(
            "invalid snapshot id: {}",
            id
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Pure logic functions (testable without Tauri State)
// ---------------------------------------------------------------------------

pub fn save_snapshot_logic(
    storage: &(dyn Storage + Send + Sync),
    name: String,
    description: Option<String>,
) -> Result<SnapshotMeta, MhostError> {
    // N4: Validate length limits
    if name.len() > MAX_SNAPSHOT_NAME_LENGTH {
        return Err(MhostError::InvalidInput(format!(
            "Snapshot name exceeds maximum length of {} characters",
            MAX_SNAPSHOT_NAME_LENGTH
        )));
    }
    if description.as_ref().map_or(0, |s| s.len()) > MAX_SNAPSHOT_DESC_LENGTH {
        return Err(MhostError::InvalidInput(format!(
            "Snapshot description exceeds maximum length of {} characters",
            MAX_SNAPSHOT_DESC_LENGTH
        )));
    }

    let profiles = storage.list_all_profiles()?;
    let id = uuid::Uuid::new_v4().to_string();
    let created_at = Utc::now();
    let snapshot = Snapshot {
        id: id.clone(),
        name: name.clone(),
        description: description.clone(),
        profiles,
        created_at,
    };

    let snapshots_dir = storage.root().join("snapshots");
    std::fs::create_dir_all(&snapshots_dir)?;
    std::fs::create_dir_all(snapshots_dir.join(SNAPSHOT_TRANSACTION_DIR))?;
    let snapshot_path = snapshots_dir.join(format!("{}.json", id));
    let json = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| MhostError::InvalidInput(format!("serialize snapshot failed: {}", e)))?;

    let meta = SnapshotMeta {
        id: id.clone(),
        name,
        description,
        profile_count: snapshot.profiles.len(),
        created_at,
    };

    // Resolve any stale markers left by an earlier interrupted save before
    // starting a new transaction.  This only reads markers and the index; it
    // does not scan the full snapshot directory on the normal hot path.
    reconcile_snapshot_index(storage, false)?;

    // The marker is the first durable state.  If the process stops after this
    // point, startup reconciliation can either complete the index entry or
    // remove a marker whose full snapshot file was never written.
    let transaction_path = snapshot_transaction_path(&snapshots_dir, &id);
    let transaction = SnapshotTransaction {
        version: SNAPSHOT_TRANSACTION_VERSION,
        meta: meta.clone(),
    };
    let transaction_json = serde_json::to_vec(&transaction).map_err(|e| {
        MhostError::InvalidInput(format!("serialize snapshot transaction failed: {}", e))
    })?;
    write_atomic_0600(&transaction_path, &transaction_json)?;

    // P-R18 (issue #181): 复用 mhost-storage 的 atomic_write_0600 替代手写
    // fs::write + rename。统一 0o600 + sync + atomic rename，避免分叉。
    // snapshot 内容包含完整 profile 规则，可能暴露内部主机名，
    // 必须 owner-only 不能用默认 umask。
    write_atomic_0600(&snapshot_path, json.as_bytes())?;

    // Prune old snapshots if exceeding MAX_SNAPSHOTS.  The index is the
    // metadata source of truth; after this first write, list_snapshots never
    // needs to read the full snapshot files again.
    let mut all = ensure_snapshot_index(storage)?;
    all.retain(|old| old.id != id);
    all.push(meta.clone());
    all.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));
    if all.len() > MAX_SNAPSHOTS {
        let mut cleanup_ok = true;
        for old in all.iter().skip(MAX_SNAPSHOTS) {
            let path = snapshots_dir.join(format!("{}.json", old.id));
            if let Err(error) = std::fs::remove_file(&path) {
                eprintln!("[mHost] Failed to prune snapshot {:?}: {}", path, error);
                cleanup_ok = false;
            }
        }
        if !cleanup_ok {
            return Err(MhostError::Io {
                kind: "Other".to_string(),
                message: format!("snapshot prune failed for transaction {}", id),
            });
        }
        all.truncate(MAX_SNAPSHOTS);
    }
    write_snapshot_index(&snapshots_dir, &all)?;

    // Keep the marker until both the full file and the updated index are
    // durable.  A failed marker cleanup is safe: startup will reconcile it.
    if all.len() <= MAX_SNAPSHOTS {
        if let Err(error) = std::fs::remove_file(&transaction_path) {
            eprintln!(
                "[mHost] Failed to remove snapshot transaction {:?}: {}",
                transaction_path, error
            );
        }
    }

    Ok(meta)
}

/// On-disk metadata index.  Keeping the version field makes it possible to
/// migrate the index when SnapshotMeta evolves without re-reading every full
/// snapshot file.
#[derive(Debug, Serialize, Deserialize)]
struct SnapshotIndex {
    version: u32,
    snapshots: Vec<SnapshotMeta>,
}

/// A crash marker is written before the complete snapshot file.  It lets the
/// next startup distinguish a complete, indexed snapshot from an interrupted
/// save, without making `list_snapshots` read every snapshot on its hot path.
#[derive(Debug, Serialize, Deserialize)]
struct SnapshotTransaction {
    version: u32,
    meta: SnapshotMeta,
}

/// Lightweight metadata-only structure for the one-time migration of legacy
/// snapshot directories (those created before `index.json` was introduced).
#[derive(Deserialize)]
struct SnapshotFileMeta {
    id: String,
    name: String,
    description: Option<String>,
    #[serde(deserialize_with = "deserialize_profile_count")]
    profiles: usize,
    created_at: DateTime<Utc>,
}

/// Custom deserializer that counts array elements without allocating them.
fn deserialize_profile_count<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct ProfileCountVisitor;

    impl<'de> serde::de::Visitor<'de> for ProfileCountVisitor {
        type Value = usize;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an array")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let mut count = 0;
            while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                count += 1;
            }
            Ok(count)
        }
    }

    deserializer.deserialize_seq(ProfileCountVisitor)
}

fn snapshot_index_path(snapshots_dir: &std::path::Path) -> PathBuf {
    snapshots_dir.join(SNAPSHOT_INDEX_FILE)
}

fn snapshot_transaction_path(snapshots_dir: &std::path::Path, id: &str) -> PathBuf {
    snapshots_dir
        .join(SNAPSHOT_TRANSACTION_DIR)
        .join(format!("{id}.json"))
}

fn write_snapshot_index(
    snapshots_dir: &std::path::Path,
    snapshots: &[SnapshotMeta],
) -> Result<(), MhostError> {
    let index = SnapshotIndex {
        version: SNAPSHOT_INDEX_VERSION,
        snapshots: snapshots.to_vec(),
    };
    let json = serde_json::to_vec(&index)
        .map_err(|e| MhostError::InvalidInput(format!("serialize snapshot index failed: {}", e)))?;
    Ok(write_atomic_0600(
        &snapshot_index_path(snapshots_dir),
        &json,
    )?)
}

fn read_snapshot_meta(path: &Path) -> Option<SnapshotMeta> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) => {
            eprintln!(
                "[mHost] Skipping unreadable snapshot file {:?}: {}",
                path, error
            );
            return None;
        }
    };
    let meta: SnapshotFileMeta = serde_json::from_str(&content)
        .map_err(|error| {
            eprintln!(
                "[mHost] Skipping corrupted snapshot file {:?}: {}",
                path, error
            );
            error
        })
        .ok()?;
    Some(SnapshotMeta {
        id: meta.id,
        name: meta.name,
        description: meta.description,
        profile_count: meta.profiles,
        created_at: meta.created_at,
    })
}

/// Rebuild the index from legacy snapshot files.  This is deliberately only
/// used when the index is missing or incompatible; normal list operations do
/// not inspect the full files.
fn rebuild_snapshot_index(
    snapshots_dir: &std::path::Path,
) -> Result<Vec<SnapshotMeta>, MhostError> {
    let mut metas = Vec::new();
    for entry in std::fs::read_dir(snapshots_dir)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) == Some(SNAPSHOT_INDEX_FILE)
            || path.extension().and_then(|extension| extension.to_str()) != Some("json")
        {
            continue;
        }

        let Some(meta) = read_snapshot_meta(&path) else {
            continue;
        };
        metas.push(meta);
    }

    metas.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));
    write_snapshot_index(snapshots_dir, &metas)?;
    Ok(metas)
}

/// Reconcile transaction markers and, when requested, legacy snapshot files
/// not represented in the index.  Startup passes `scan_orphans = true`; the
/// normal list path passes `false` so it never regresses #179's fast path.
pub fn reconcile_snapshot_index(
    storage: &(dyn Storage + Send + Sync),
    scan_orphans: bool,
) -> Result<(), MhostError> {
    let snapshots_dir = storage.root().join("snapshots");
    if !snapshots_dir.exists() {
        return Ok(());
    }

    let index_path = snapshot_index_path(&snapshots_dir);
    let mut snapshots = if index_path.exists() {
        let content = std::fs::read_to_string(&index_path)?;
        match serde_json::from_str::<SnapshotIndex>(&content) {
            Ok(index) if index.version == SNAPSHOT_INDEX_VERSION => index.snapshots,
            Ok(index) => {
                eprintln!(
                    "[mHost] Rebuilding snapshot index for unsupported version {}",
                    index.version
                );
                rebuild_snapshot_index(&snapshots_dir)?
            }
            Err(error) => {
                eprintln!("[mHost] Rebuilding corrupted snapshot index: {}", error);
                rebuild_snapshot_index(&snapshots_dir)?
            }
        }
    } else if snapshots_dir.exists() {
        // A missing index still needs pending markers to be resolved.  This
        // is also the legacy migration path used on first startup.
        rebuild_snapshot_index(&snapshots_dir)?
    } else {
        Vec::new()
    };
    let mut changed = false;

    // Resolve each marker independently.  This also makes a retry after a
    // failed save idempotent: a marker without its full file is discarded,
    // while a marker with a valid file is merged into the index.
    let transaction_dir = snapshots_dir.join(SNAPSHOT_TRANSACTION_DIR);
    if transaction_dir.exists() {
        for entry in std::fs::read_dir(&transaction_dir)? {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }

            let content = match std::fs::read_to_string(&path) {
                Ok(content) => content,
                Err(error) => {
                    eprintln!(
                        "[mHost] Ignoring unreadable snapshot transaction {:?}: {}",
                        path, error
                    );
                    if let Err(remove_error) = std::fs::remove_file(&path) {
                        eprintln!(
                            "[mHost] Failed to remove unreadable snapshot transaction {:?}: {}",
                            path, remove_error
                        );
                    }
                    continue;
                }
            };
            let transaction: Result<SnapshotTransaction, _> = serde_json::from_str(&content);
            let transaction = match transaction {
                Ok(transaction) if transaction.version == SNAPSHOT_TRANSACTION_VERSION => {
                    transaction
                }
                Ok(transaction) => {
                    eprintln!(
                        "[mHost] Ignoring snapshot transaction with unsupported version {}",
                        transaction.version
                    );
                    if let Err(remove_error) = std::fs::remove_file(&path) {
                        eprintln!(
                            "[mHost] Failed to remove unsupported snapshot transaction {:?}: {}",
                            path, remove_error
                        );
                    }
                    continue;
                }
                Err(error) => {
                    eprintln!(
                        "[mHost] Ignoring corrupted snapshot transaction {:?}: {}",
                        path, error
                    );
                    if let Err(remove_error) = std::fs::remove_file(&path) {
                        eprintln!(
                            "[mHost] Failed to remove corrupted snapshot transaction {:?}: {}",
                            path, remove_error
                        );
                    }
                    continue;
                }
            };
            if validate_snapshot_id(&transaction.meta.id).is_err() {
                eprintln!(
                    "[mHost] Ignoring snapshot transaction with invalid id {:?}",
                    path
                );
            } else {
                let snapshot_path = snapshots_dir.join(format!("{}.json", transaction.meta.id));
                if snapshot_path.exists() {
                    if read_snapshot_meta(&snapshot_path).is_none() {
                        eprintln!(
                            "[mHost] Ignoring snapshot transaction with invalid full file {:?}",
                            snapshot_path
                        );
                    } else if let Some(existing) = snapshots
                        .iter_mut()
                        .find(|entry| entry.id == transaction.meta.id)
                    {
                        if *existing != transaction.meta {
                            *existing = transaction.meta;
                            changed = true;
                        }
                    } else {
                        snapshots.push(transaction.meta);
                        changed = true;
                    }
                } else {
                    eprintln!(
                        "[mHost] Removing snapshot transaction without full file {:?}",
                        path
                    );
                }
            }

            if let Err(error) = std::fs::remove_file(&path) {
                eprintln!(
                    "[mHost] Failed to remove snapshot transaction {:?}: {}",
                    path, error
                );
            }
        }
    }

    // Index entries must point to a complete snapshot.  This is the second
    // half of the marker protocol and cleans up a phantom entry left by an
    // interrupted index-first protocol from an older version.
    let original_len = snapshots.len();
    snapshots.retain(|entry| snapshots_dir.join(format!("{}.json", entry.id)).exists());
    changed |= snapshots.len() != original_len;

    if scan_orphans {
        // This is deliberately startup-only.  It repairs pre-#188 data left
        // behind before the transaction marker was introduced, without
        // making every list_snapshots call O(N × file_size) again.
        for entry in std::fs::read_dir(&snapshots_dir)? {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.file_name().and_then(|name| name.to_str()) == Some(SNAPSHOT_INDEX_FILE)
                || path.extension().and_then(|extension| extension.to_str()) != Some("json")
            {
                continue;
            }
            let Some(meta) = read_snapshot_meta(&path) else {
                continue;
            };
            if !snapshots.iter().any(|entry| entry.id == meta.id) {
                snapshots.push(meta);
                changed = true;
            }
        }
    }

    snapshots.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));
    if changed {
        write_snapshot_index(&snapshots_dir, &snapshots)?;
    }
    Ok(())
}

/// Load or lazily rebuild the metadata index.  Once present, this reads only
/// one small file and never touches `<id>.json` files.
fn ensure_snapshot_index(
    storage: &(dyn Storage + Send + Sync),
) -> Result<Vec<SnapshotMeta>, MhostError> {
    reconcile_snapshot_index(storage, false)?;
    let snapshots_dir = storage.root().join("snapshots");
    let index_path = snapshot_index_path(&snapshots_dir);
    if !snapshots_dir.exists() {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(&index_path)?;
    match serde_json::from_str::<SnapshotIndex>(&content) {
        Ok(index) if index.version == SNAPSHOT_INDEX_VERSION => {
            let mut snapshots = index.snapshots;
            snapshots.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));
            Ok(snapshots)
        }
        Ok(index) => {
            eprintln!(
                "[mHost] Rebuilding snapshot index for unsupported version {}",
                index.version
            );
            rebuild_snapshot_index(&snapshots_dir)
        }
        Err(error) => {
            eprintln!("[mHost] Rebuilding corrupted snapshot index: {}", error);
            rebuild_snapshot_index(&snapshots_dir)
        }
    }
}

pub fn list_snapshots_logic(
    storage: &(dyn Storage + Send + Sync),
) -> Result<Vec<SnapshotMeta>, MhostError> {
    ensure_snapshot_index(storage)
}

pub fn load_snapshot_logic(
    storage: &(dyn Storage + Send + Sync),
    writer: &HostsWriter,
    id: &str,
) -> Result<(), MhostError> {
    validate_snapshot_id(id)?;

    // Load participates in the same pending-transaction recovery protocol.
    // Without this, a crash marker could be loaded before its index entry was
    // committed.  Reconcile is also idempotent and does not scan full files
    // when the index is healthy.
    reconcile_snapshot_index(storage, false)?;

    let snapshot_path = storage
        .root()
        .join("snapshots")
        .join(format!("{}.json", id));
    if !snapshot_path.exists() {
        return Err(MhostError::InvalidInput(format!(
            "snapshot not found: {}",
            id
        )));
    }

    let content = std::fs::read_to_string(&snapshot_path)?;
    let snapshot: Snapshot = serde_json::from_str(&content)
        .map_err(|e| MhostError::InvalidInput(format!("parse snapshot failed: {}", e)))?;

    // Fix (B2): Atomic recovery — save all snapshot profiles first, then delete extras.
    // If save_profile fails partway through, we only have extra profiles (no data loss).
    let current_profiles = storage.list_all_profiles()?;
    let snapshot_ids: std::collections::HashSet<_> =
        snapshot.profiles.iter().map(|p| p.id.clone()).collect();

    // Save all snapshot profiles (overwrites any with matching ids)
    for profile in snapshot.profiles {
        storage.save_profile(&profile)?;
    }

    // Delete current profiles that are not in the snapshot
    for p in current_profiles {
        if !snapshot_ids.contains(&p.id) {
            storage.delete_profile(&p.id)?;
        }
    }

    // Apply current plan
    // `apply_current_plan_logic` now returns `Option<PathBuf>` (backup path);
    // snapshot loading doesn't surface it, so discard.
    let _ = crate::commands::apply::apply_current_plan_logic(storage, writer)?;

    Ok(())
}

pub fn delete_snapshot_logic(
    storage: &(dyn Storage + Send + Sync),
    id: &str,
) -> Result<(), MhostError> {
    validate_snapshot_id(id)?;

    let snapshots_dir = storage.root().join("snapshots");
    let snapshot_path = snapshots_dir.join(format!("{}.json", id));
    let mut all = if snapshot_path.exists() || snapshot_index_path(&snapshots_dir).exists() {
        ensure_snapshot_index(storage)?
    } else {
        Vec::new()
    };

    if snapshot_path.exists() {
        std::fs::remove_file(&snapshot_path)?;
    }
    let previous_len = all.len();
    all.retain(|entry| entry.id != id);
    if all.len() != previous_len {
        write_snapshot_index(&snapshots_dir, &all)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn save_snapshot(
    name: String,
    description: Option<String>,
    state: State<'_, AppState>,
) -> Result<SnapshotMeta, MhostError> {
    // N2: Serialize snapshot operations to prevent races during save+prune
    let _guard = state.snapshot_lock.lock().await;
    let storage = state.storage.clone();
    tauri::async_runtime::spawn_blocking(move || {
        save_snapshot_logic(storage.as_ref(), name, description)
    })
    .await
    .map_err(|e| MhostError::InvalidInput(e.to_string()))?
}

#[tauri::command]
pub async fn list_snapshots(state: State<'_, AppState>) -> Result<Vec<SnapshotMeta>, MhostError> {
    let _guard = state.snapshot_lock.lock().await;
    let storage = state.storage.clone();
    tauri::async_runtime::spawn_blocking(move || list_snapshots_logic(storage.as_ref()))
        .await
        .map_err(|e| MhostError::InvalidInput(e.to_string()))?
}

#[tauri::command]
pub async fn load_snapshot(
    id: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), MhostError> {
    // Keep load on the same snapshot lock as save/delete.  load_snapshot_logic
    // also reconciles pending markers, so it must not race a save or delete.
    let _apply_guard = state.apply_lock.lock().await;
    let _snapshot_guard = state.snapshot_lock.lock().await;
    let storage = state.storage.clone();
    let writer = state.writer.clone();
    tauri::async_runtime::spawn_blocking(move || {
        load_snapshot_logic(storage.as_ref(), &writer, &id)
    })
    .await
    .map_err(|e| MhostError::InvalidInput(e.to_string()))??;
    // P-R12 (issue #181): load_snapshot_logic 写入 N 条 profile (snapshot.profiles)
    // + 删除不在 snapshot 的当前 profiles, cache 必须 invalidate.
    state.invalidate_profile_cache();

    // 快照恢复后，若 DNS 模式处于启用状态，同步重载 DNS 规则表
    if state.dns_enabled.load(std::sync::atomic::Ordering::Relaxed) {
        let profiles = state
            .storage
            .list_profiles_by_mode(ProfileMode::Dns)
            .map_err(MhostError::from)?;
        let enabled_profiles: Vec<_> = profiles.into_iter().filter(|p| p.enabled).collect();

        if let Some(server) = lock_or_recover(&state.dns_server).as_ref() {
            server.reload_rules(&enabled_profiles);
        }
    }

    #[cfg(target_os = "macos")]
    crate::tray::update_tray_menu(&app_handle);

    Ok(())
}

#[tauri::command]
pub async fn delete_snapshot(id: String, state: State<'_, AppState>) -> Result<(), MhostError> {
    // N2: Serialize snapshot operations
    let _guard = state.snapshot_lock.lock().await;
    let storage = state.storage.clone();
    tauri::async_runtime::spawn_blocking(move || delete_snapshot_logic(storage.as_ref(), &id))
        .await
        .map_err(|e| MhostError::InvalidInput(e.to_string()))?
}

// ---------------------------------------------------------------------------
// Auto snapshot
// ---------------------------------------------------------------------------

const AUTO_SNAPSHOT_INTERVAL_DAYS: i64 = 3;

/// Automatically create a snapshot after apply if conditions are met:
/// - If no snapshots exist, create one.
/// - If the latest snapshot is older than 3 days, create a new one.
/// - Otherwise, do nothing.
pub fn auto_snapshot_logic(
    storage: &(dyn Storage + Send + Sync),
    snapshot_lock: &ApplyLock,
) -> Result<Option<SnapshotMeta>, MhostError> {
    let _snapshot_guard = snapshot_lock.blocking_lock();
    // Blocking lock is required because this pure helper runs inside
    // spawn_blocking after the caller has acquired apply_lock.
    let snapshots = list_snapshots_logic(storage)?;

    let should_create = if snapshots.is_empty() {
        true
    } else {
        let latest = &snapshots[0]; // list_snapshots_logic returns descending order
        let now = Utc::now();
        let diff = now.signed_duration_since(latest.created_at);
        diff.num_days() >= AUTO_SNAPSHOT_INTERVAL_DAYS
    };

    if should_create {
        let name = format!("Auto-snapshot {}", Utc::now().format("%Y-%m-%d %H:%M"));
        let meta = save_snapshot_logic(
            storage,
            name,
            Some("Automatically created on apply".to_string()),
        )?;
        Ok(Some(meta))
    } else {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mhost_core::{HostRule, Profile};
    use mhost_storage::storage::{FileStorage, Storage};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn create_test_storage_and_writer() -> (TempDir, Arc<dyn Storage + Send + Sync>, HostsWriter) {
        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(FileStorage::new(temp_dir.path())) as Arc<dyn Storage + Send + Sync>;

        let hosts_path = temp_dir.path().join("hosts");
        let backup_dir = temp_dir.path().join("backups");
        std::fs::write(&hosts_path, "# original hosts\n").unwrap();

        let writer = HostsWriter::with_paths(&hosts_path, &backup_dir);
        (temp_dir, storage, writer)
    }

    fn create_profile_with_rules(
        storage: &Arc<dyn Storage + Send + Sync>,
        name: &str,
        rules: Vec<(&str, &str)>,
    ) -> Profile {
        let mut profile = Profile::new(name);
        for (ip, domain) in rules {
            profile
                .rules
                .push(HostRule::new(ip.parse().unwrap(), vec![domain.to_string()]));
        }
        storage.save_profile(&profile).unwrap();
        profile
    }

    #[test]
    fn test_save_snapshot_creates_file() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);

        let meta = save_snapshot_logic(storage.as_ref(), "test-snap".to_string(), None).unwrap();
        assert_eq!(meta.name, "test-snap");
        assert_eq!(meta.profile_count, 1);
        assert!(meta.description.is_none());

        let snapshot_path = storage
            .root()
            .join("snapshots")
            .join(format!("{}.json", meta.id));
        assert!(snapshot_path.exists());
    }

    /// 回归测试 P-R18（issue #181）：snapshot 文件必须 owner-only (0o600)，
    /// 不能依赖 umask 默认值。snapshot 内容是完整 profile 规则，
    /// 可能暴露内部主机名 / staging 域名，必须保护。
    #[test]
    fn test_save_snapshot_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let (_temp, storage, _writer) = create_test_storage_and_writer();
        create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);

        let meta = save_snapshot_logic(storage.as_ref(), "test-snap".to_string(), None).unwrap();
        let snapshot_path = storage
            .root()
            .join("snapshots")
            .join(format!("{}.json", meta.id));

        let mode = std::fs::metadata(&snapshot_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "snapshot 文件 mode 必须 0o600, 实际 {:#o}",
            mode
        );
    }

    #[test]
    fn test_snapshot_index_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let (_temp, storage, _writer) = create_test_storage_and_writer();
        save_snapshot_logic(storage.as_ref(), "index-permissions".to_string(), None).unwrap();

        let index_path = storage.root().join("snapshots").join("index.json");
        let mode = std::fs::metadata(index_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn test_list_rebuilds_unsupported_snapshot_index_version() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        let meta = save_snapshot_logic(storage.as_ref(), "versioned".to_string(), None).unwrap();
        let index_path = storage.root().join("snapshots").join("index.json");
        std::fs::write(&index_path, br#"{"version":999,"snapshots":[]}"#).unwrap();

        let metas = list_snapshots_logic(storage.as_ref()).unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, meta.id);
        let rebuilt: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(index_path).unwrap()).unwrap();
        assert_eq!(rebuilt["version"], 1);
    }

    #[test]
    fn test_list_rebuilds_corrupted_snapshot_index() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        let meta =
            save_snapshot_logic(storage.as_ref(), "corrupted-index".to_string(), None).unwrap();
        let index_path = storage.root().join("snapshots").join("index.json");
        std::fs::write(index_path, b"not valid index json").unwrap();

        let metas = list_snapshots_logic(storage.as_ref()).unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, meta.id);
    }

    #[test]
    fn test_save_snapshot_prunes_old() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();

        // Create MAX_SNAPSHOTS + 5 snapshots
        for i in 0..MAX_SNAPSHOTS + 5 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            let name = format!("snap-{}", i);
            let _meta = save_snapshot_logic(storage.as_ref(), name, None).unwrap();
        }

        let metas = list_snapshots_logic(storage.as_ref()).unwrap();
        assert_eq!(metas.len(), MAX_SNAPSHOTS, "should prune to MAX_SNAPSHOTS");
    }

    #[test]
    fn test_list_snapshots_returns_meta_only() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);

        let meta = save_snapshot_logic(storage.as_ref(), "test-snap".to_string(), None).unwrap();
        let metas = list_snapshots_logic(storage.as_ref()).unwrap();

        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, meta.id);
        assert_eq!(metas[0].name, "test-snap");
        assert_eq!(metas[0].profile_count, 1);
    }

    #[test]
    fn test_save_snapshot_creates_metadata_index() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        let meta = save_snapshot_logic(storage.as_ref(), "indexed".to_string(), None).unwrap();

        let index_path = storage.root().join("snapshots").join("index.json");
        let index_json = std::fs::read_to_string(index_path).unwrap();
        let index: serde_json::Value = serde_json::from_str(&index_json).unwrap();
        assert_eq!(index["version"], 1);
        assert_eq!(index["snapshots"].as_array().unwrap().len(), 1);
        assert_eq!(index["snapshots"][0]["id"], meta.id);
    }

    #[test]
    fn test_delete_snapshot_updates_metadata_index() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        let meta = save_snapshot_logic(storage.as_ref(), "delete-me".to_string(), None).unwrap();

        delete_snapshot_logic(storage.as_ref(), &meta.id).unwrap();

        let index_path = storage.root().join("snapshots").join("index.json");
        let index_json = std::fs::read_to_string(index_path).unwrap();
        let index: serde_json::Value = serde_json::from_str(&index_json).unwrap();
        assert!(index["snapshots"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_reconcile_recovers_orphan_full_snapshot_file() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        let snapshot = Snapshot {
            id: uuid::Uuid::new_v4().to_string(),
            name: "orphan".to_string(),
            description: None,
            profiles: Vec::new(),
            created_at: Utc::now(),
        };
        let snapshots_dir = storage.root().join("snapshots");
        std::fs::create_dir_all(&snapshots_dir).unwrap();
        std::fs::write(
            snapshots_dir.join(format!("{}.json", snapshot.id)),
            serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();

        // Simulate the old crash window: the complete file exists, but no
        // index entry was ever written.  Startup reconciliation recovers it.
        reconcile_snapshot_index(storage.as_ref(), true).unwrap();

        let metas = list_snapshots_logic(storage.as_ref()).unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, snapshot.id);
    }

    #[test]
    fn test_reconcile_removes_phantom_index_entry() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        let id = uuid::Uuid::new_v4();
        let meta = SnapshotMeta {
            id: id.to_string(),
            name: "phantom".to_string(),
            description: None,
            profile_count: 0,
            created_at: Utc::now(),
        };
        let index = SnapshotIndex {
            version: SNAPSHOT_INDEX_VERSION,
            snapshots: vec![meta],
        };
        let snapshots_dir = storage.root().join("snapshots");
        std::fs::create_dir_all(&snapshots_dir).unwrap();
        write_snapshot_index(&snapshots_dir, &index.snapshots).unwrap();

        // Simulate an interrupted write that published the index entry but
        // never produced the full snapshot file.
        reconcile_snapshot_index(storage.as_ref(), true).unwrap();

        assert!(list_snapshots_logic(storage.as_ref()).unwrap().is_empty());
    }

    #[test]
    fn test_reconcile_commits_pending_transaction_with_full_file() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        let id = uuid::Uuid::new_v4();
        let meta = SnapshotMeta {
            id: id.to_string(),
            name: "pending".to_string(),
            description: None,
            profile_count: 0,
            created_at: Utc::now(),
        };
        let snapshot = Snapshot {
            id: meta.id.clone(),
            name: meta.name.clone(),
            description: None,
            profiles: Vec::new(),
            created_at: meta.created_at,
        };
        let snapshots_dir = storage.root().join("snapshots");
        let transaction_dir = snapshots_dir.join(SNAPSHOT_TRANSACTION_DIR);
        std::fs::create_dir_all(&transaction_dir).unwrap();
        std::fs::write(
            snapshots_dir.join(format!("{}.json", meta.id)),
            serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();
        let transaction = SnapshotTransaction {
            version: SNAPSHOT_TRANSACTION_VERSION,
            meta: meta.clone(),
        };
        write_atomic_0600(
            &snapshot_transaction_path(&snapshots_dir, &meta.id),
            &serde_json::to_vec(&transaction).unwrap(),
        )
        .unwrap();

        // A marker with its full file is committed without scanning the
        // complete snapshot directory.  Running reconciliation repeatedly
        // must leave the same final state.
        reconcile_snapshot_index(storage.as_ref(), false).unwrap();
        reconcile_snapshot_index(storage.as_ref(), false).unwrap();

        let metas = list_snapshots_logic(storage.as_ref()).unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, meta.id);
        assert!(!snapshot_transaction_path(&snapshots_dir, &meta.id).exists());
    }

    #[test]
    fn test_first_index_migration_resolves_pending_transaction() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        let id = uuid::Uuid::new_v4();
        let meta = SnapshotMeta {
            id: id.to_string(),
            name: "first-index-pending".to_string(),
            description: None,
            profile_count: 0,
            created_at: Utc::now(),
        };
        let snapshot = Snapshot {
            id: meta.id.clone(),
            name: meta.name.clone(),
            description: None,
            profiles: Vec::new(),
            created_at: meta.created_at,
        };
        let snapshots_dir = storage.root().join("snapshots");
        let transaction_dir = snapshots_dir.join(SNAPSHOT_TRANSACTION_DIR);
        std::fs::create_dir_all(&transaction_dir).unwrap();
        std::fs::write(
            snapshots_dir.join(format!("{}.json", meta.id)),
            serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();
        let transaction = SnapshotTransaction {
            version: SNAPSHOT_TRANSACTION_VERSION,
            meta: meta.clone(),
        };
        write_atomic_0600(
            &snapshot_transaction_path(&snapshots_dir, &meta.id),
            &serde_json::to_vec(&transaction).unwrap(),
        )
        .unwrap();

        // This is a crash before index.json is first created.  A normal list
        // must recover the marker as part of the migration, not leave it behind.
        let metas = list_snapshots_logic(storage.as_ref()).unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, meta.id);
        assert!(!snapshot_transaction_path(&snapshots_dir, &meta.id).exists());
    }

    #[test]
    fn test_reconcile_drops_pending_transaction_without_full_file() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        let meta = SnapshotMeta {
            id: uuid::Uuid::new_v4().to_string(),
            name: "incomplete".to_string(),
            description: None,
            profile_count: 0,
            created_at: Utc::now(),
        };
        let snapshots_dir = storage.root().join("snapshots");
        let transaction_dir = snapshots_dir.join(SNAPSHOT_TRANSACTION_DIR);
        std::fs::create_dir_all(&transaction_dir).unwrap();
        let transaction = SnapshotTransaction {
            version: SNAPSHOT_TRANSACTION_VERSION,
            meta: meta.clone(),
        };
        write_atomic_0600(
            &snapshot_transaction_path(&snapshots_dir, &meta.id),
            &serde_json::to_vec(&transaction).unwrap(),
        )
        .unwrap();

        reconcile_snapshot_index(storage.as_ref(), false).unwrap();

        assert!(list_snapshots_logic(storage.as_ref()).unwrap().is_empty());
        assert!(!snapshot_transaction_path(&snapshots_dir, &meta.id).exists());
    }

    #[test]
    fn test_list_snapshots_migrates_legacy_data_once() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        let snapshot = Snapshot {
            id: uuid::Uuid::new_v4().to_string(),
            name: "legacy".to_string(),
            description: None,
            profiles: Vec::new(),
            created_at: Utc::now(),
        };
        let snapshots_dir = storage.root().join("snapshots");
        std::fs::create_dir_all(&snapshots_dir).unwrap();
        std::fs::write(
            snapshots_dir.join(format!("{}.json", snapshot.id)),
            serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();
        std::fs::write(
            snapshots_dir.join("corrupt.json"),
            b"not valid snapshot json",
        )
        .unwrap();

        // The first call performs the one-time migration and creates index.json.
        // The corrupt legacy file is skipped while the valid snapshot is indexed.
        assert_eq!(list_snapshots_logic(storage.as_ref()).unwrap().len(), 1);
        assert!(snapshots_dir.join("index.json").exists());

        // A later call only reads the index, so a legacy full snapshot file may
        // be corrupted without affecting the metadata list.
        std::fs::write(
            snapshots_dir.join(format!("{}.json", snapshot.id)),
            b"not valid snapshot json",
        )
        .unwrap();
        let metas = list_snapshots_logic(storage.as_ref()).unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].name, "legacy");
    }

    #[test]
    fn test_list_snapshots_sorted_by_date() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();

        for i in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            let _ = save_snapshot_logic(storage.as_ref(), format!("snap-{}", i), None).unwrap();
        }

        let metas = list_snapshots_logic(storage.as_ref()).unwrap();
        assert_eq!(metas.len(), 3);
        // Should be sorted descending (newest first)
        assert!(metas[0].created_at >= metas[1].created_at);
        assert!(metas[1].created_at >= metas[2].created_at);
    }

    #[test]
    fn test_load_snapshot_restores_profiles() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Create original profiles
        let _p1 = create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);
        let _p2 = create_profile_with_rules(&storage, "test", vec![("192.168.1.1", "test.local")]);

        // Save snapshot
        let meta = save_snapshot_logic(storage.as_ref(), "backup".to_string(), None).unwrap();

        // Delete all profiles
        for p in storage.list_profiles().unwrap() {
            storage.delete_profile(&p.id).unwrap();
        }
        assert!(storage.list_profiles().unwrap().is_empty());

        // Load snapshot
        load_snapshot_logic(storage.as_ref(), &writer, &meta.id).unwrap();

        let restored = storage.list_profiles().unwrap();
        assert_eq!(restored.len(), 2);
        assert!(restored.iter().any(|p| p.name == "dev"));
        assert!(restored.iter().any(|p| p.name == "test"));
    }

    #[test]
    fn test_load_snapshot_reconciles_pending_transaction_first() {
        let (_temp, storage, writer) = create_test_storage_and_writer();
        let profile = create_profile_with_rules(
            &storage,
            "pending-load",
            vec![("127.0.0.1", "pending.local")],
        );
        let meta = SnapshotMeta {
            id: uuid::Uuid::new_v4().to_string(),
            name: "pending-load".to_string(),
            description: None,
            profile_count: 1,
            created_at: Utc::now(),
        };
        let snapshot = Snapshot {
            id: meta.id.clone(),
            name: meta.name.clone(),
            description: None,
            profiles: vec![profile],
            created_at: meta.created_at,
        };
        let snapshots_dir = storage.root().join("snapshots");
        let transaction_dir = snapshots_dir.join(SNAPSHOT_TRANSACTION_DIR);
        std::fs::create_dir_all(&transaction_dir).unwrap();
        std::fs::write(
            snapshots_dir.join(format!("{}.json", meta.id)),
            serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();
        let transaction = SnapshotTransaction {
            version: SNAPSHOT_TRANSACTION_VERSION,
            meta: meta.clone(),
        };
        write_atomic_0600(
            &snapshot_transaction_path(&snapshots_dir, &meta.id),
            &serde_json::to_vec(&transaction).unwrap(),
        )
        .unwrap();

        load_snapshot_logic(storage.as_ref(), &writer, &meta.id).unwrap();

        assert!(storage
            .list_profiles()
            .unwrap()
            .iter()
            .any(|profile| profile.name == "pending-load"));
        assert!(!snapshot_transaction_path(&snapshots_dir, &meta.id).exists());
    }

    #[test]
    fn test_load_snapshot_applies_hosts() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        let mut profile =
            create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);
        profile.enabled = true;
        storage.save_profile(&profile).unwrap();

        // Apply first to set up hosts
        crate::commands::apply::apply_current_plan_logic(storage.as_ref(), &writer).unwrap();

        let meta = save_snapshot_logic(storage.as_ref(), "backup".to_string(), None).unwrap();

        // Clear profiles
        for p in storage.list_profiles().unwrap() {
            storage.delete_profile(&p.id).unwrap();
        }

        // Load and apply
        load_snapshot_logic(storage.as_ref(), &writer, &meta.id).unwrap();

        let hosts_content = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(hosts_content.contains("127.0.0.1 example.com"));
    }

    #[test]
    fn test_delete_snapshot_removes_file() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);

        let meta = save_snapshot_logic(storage.as_ref(), "to-delete".to_string(), None).unwrap();
        let snapshot_path = storage
            .root()
            .join("snapshots")
            .join(format!("{}.json", meta.id));
        assert!(snapshot_path.exists());

        delete_snapshot_logic(storage.as_ref(), &meta.id).unwrap();
        assert!(!snapshot_path.exists());
    }

    #[test]
    fn test_load_snapshot_validates_id_format() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // B1: Rejects path traversal attempts
        let result = load_snapshot_logic(storage.as_ref(), &writer, "../etc/passwd");
        assert!(result.is_err(), "should reject invalid snapshot id");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("invalid snapshot id"),
            "error should mention invalid id: {}",
            msg
        );

        let result = delete_snapshot_logic(storage.as_ref(), "../../secret");
        assert!(
            result.is_err(),
            "should reject invalid snapshot id for delete"
        );
    }

    #[test]
    fn test_load_snapshot_recovery_on_partial_failure() {
        // B2: Verify that if save_profile fails partway, no data is lost.
        // In practice, FileStorage::save_profile is atomic, so this test
        // verifies the ordering (save first, delete after).
        let (_temp, storage, writer) = create_test_storage_and_writer();
        let p1 = create_profile_with_rules(&storage, "keep", vec![("127.0.0.1", "keep.local")]);
        let p2 =
            create_profile_with_rules(&storage, "remove", vec![("192.168.1.1", "remove.local")]);

        // Save a snapshot that only contains "keep" by building it manually
        let snapshots_dir = storage.root().join("snapshots");
        std::fs::create_dir_all(&snapshots_dir).unwrap();
        let snapshot = mhost_core::Snapshot {
            id: uuid::Uuid::new_v4().to_string(),
            name: "partial".to_string(),
            description: None,
            profiles: vec![p1],
            created_at: Utc::now(),
        };
        let path = snapshots_dir.join(format!("{}.json", snapshot.id));
        let json = serde_json::to_string_pretty(&snapshot).unwrap();
        std::fs::write(&path, json).unwrap();

        // Delete original profiles
        for p in storage.list_profiles().unwrap() {
            storage.delete_profile(&p.id).unwrap();
        }
        assert!(storage.list_profiles().unwrap().is_empty());

        // Load snapshot (only "keep" should exist after)
        load_snapshot_logic(storage.as_ref(), &writer, &snapshot.id).unwrap();

        let restored = storage.list_profiles().unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].name, "keep");

        // Verify "remove" profile id is gone
        assert!(!restored.iter().any(|p| p.id == p2.id));
    }

    // -----------------------------------------------------------------------
    // Auto snapshot tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_auto_snapshot_creates_when_empty() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);

        let result = auto_snapshot_logic(storage.as_ref(), &ApplyLock::new()).unwrap();
        assert!(
            result.is_some(),
            "should create snapshot when list is empty"
        );

        let snapshots = list_snapshots_logic(storage.as_ref()).unwrap();
        assert_eq!(snapshots.len(), 1);
        assert!(snapshots[0].name.starts_with("Auto-snapshot"));
    }

    #[test]
    fn test_auto_snapshot_skips_when_recent() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);

        // Create a snapshot with current time
        save_snapshot_logic(storage.as_ref(), "recent".to_string(), None).unwrap();

        let result = auto_snapshot_logic(storage.as_ref(), &ApplyLock::new()).unwrap();
        assert!(
            result.is_none(),
            "should NOT create snapshot when recent one exists"
        );

        let snapshots = list_snapshots_logic(storage.as_ref()).unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].name, "recent");
    }

    #[test]
    fn test_auto_snapshot_creates_when_old() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);

        // Create an old snapshot by writing file directly with backdated created_at
        let old_snapshot = mhost_core::Snapshot {
            id: uuid::Uuid::new_v4().to_string(),
            name: "old".to_string(),
            description: None,
            profiles: storage.list_profiles().unwrap(),
            created_at: Utc::now() - chrono::Duration::days(4),
        };
        let snapshots_dir = storage.root().join("snapshots");
        std::fs::create_dir_all(&snapshots_dir).unwrap();
        let path = snapshots_dir.join(format!("{}.json", old_snapshot.id));
        let json = serde_json::to_string_pretty(&old_snapshot).unwrap();
        std::fs::write(&path, json).unwrap();

        let result = auto_snapshot_logic(storage.as_ref(), &ApplyLock::new()).unwrap();
        assert!(
            result.is_some(),
            "should create snapshot when latest is older than 3 days"
        );

        let snapshots = list_snapshots_logic(storage.as_ref()).unwrap();
        assert_eq!(snapshots.len(), 2);
        assert!(snapshots[0].name.starts_with("Auto-snapshot")); // newest first
    }

    #[test]
    fn test_save_snapshot_rejects_long_name() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        let long_name = "a".repeat(MAX_SNAPSHOT_NAME_LENGTH + 1);
        let result = save_snapshot_logic(storage.as_ref(), long_name, None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("exceeds maximum length"));
    }

    #[test]
    fn test_save_snapshot_rejects_long_description() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        let long_desc = "a".repeat(MAX_SNAPSHOT_DESC_LENGTH + 1);
        let result = save_snapshot_logic(storage.as_ref(), "name".to_string(), Some(long_desc));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("exceeds maximum length"));
    }

    #[test]
    fn test_snapshot_saves_and_restores_cross_mode_profiles() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Create hosts profile
        let mut hosts_profile = Profile::new("hosts_dev");
        hosts_profile.rules.push(mhost_core::HostRule::new(
            "127.0.0.1".parse().unwrap(),
            vec!["hosts.local".to_string()],
        ));
        hosts_profile.enabled = true;
        storage.save_profile(&hosts_profile).unwrap();

        // Create DNS profile (set mode before first save to avoid duplicate in hosts dir)
        let mut dns_profile = Profile::new("dns_dev");
        dns_profile.mode = ProfileMode::Dns;
        dns_profile.rules.push(mhost_core::HostRule::new(
            "192.168.1.1".parse().unwrap(),
            vec!["dns.local".to_string()],
        ));
        dns_profile.enabled = true;
        storage.save_profile(&dns_profile).unwrap();

        // Save snapshot — should include both hosts and DNS profiles
        let snapshot_meta =
            save_snapshot_logic(storage.as_ref(), "cross_mode".to_string(), None).unwrap();
        let snapshot_id = snapshot_meta.id;
        assert!(!snapshot_id.is_empty());

        // Verify snapshot contains both profiles by reading the file directly
        let snapshots_dir = storage.root().join("snapshots");
        let snapshot_path = snapshots_dir.join(format!("{}.json", snapshot_id));
        let snapshot_json = std::fs::read_to_string(&snapshot_path).unwrap();
        let snapshot: Snapshot = serde_json::from_str(&snapshot_json).unwrap();
        assert_eq!(
            snapshot.profiles.len(),
            2,
            "snapshot should contain both hosts and dns profiles"
        );
        assert!(snapshot
            .profiles
            .iter()
            .any(|p| p.mode == ProfileMode::Hosts));
        assert!(snapshot.profiles.iter().any(|p| p.mode == ProfileMode::Dns));

        // Delete all profiles
        for p in storage.list_all_profiles().unwrap() {
            storage.delete_profile(&p.id).unwrap();
        }
        assert!(storage.list_all_profiles().unwrap().is_empty());

        // Load snapshot — both profiles should be restored
        load_snapshot_logic(storage.as_ref(), &writer, &snapshot_id).unwrap();

        let restored = storage.list_all_profiles().unwrap();
        assert_eq!(restored.len(), 2, "both profiles should be restored");
        assert!(restored
            .iter()
            .any(|p| p.mode == ProfileMode::Hosts && p.name == "hosts_dev"));
        assert!(restored
            .iter()
            .any(|p| p.mode == ProfileMode::Dns && p.name == "dns_dev"));
    }
}
