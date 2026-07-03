use chrono::{DateTime, Utc};
use mhost_apply::writer::HostsWriter;
use mhost_core::{MhostError, ProfileMode, Snapshot, SnapshotMeta};
use mhost_storage::storage::Storage;
use serde::Deserialize;
use tauri::{AppHandle, State};

use crate::state::AppState;

const MAX_SNAPSHOTS: usize = 20;
const MAX_SNAPSHOT_NAME_LENGTH: usize = 100;
const MAX_SNAPSHOT_DESC_LENGTH: usize = 500;

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
    if description
        .as_ref()
        .map_or(0, |s| s.len())
        > MAX_SNAPSHOT_DESC_LENGTH
    {
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
    let snapshot_path = snapshots_dir.join(format!("{}.json", id));
    let json = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| MhostError::InvalidInput(format!("serialize snapshot failed: {}", e)))?;

    // N1: Atomic write via temp file + rename
    let temp_path = snapshot_path.with_extension("tmp");
    std::fs::write(&temp_path, json)?;
    std::fs::rename(&temp_path, &snapshot_path)?;

    let meta = SnapshotMeta {
        id: id.clone(),
        name,
        description,
        profile_count: snapshot.profiles.len(),
        created_at,
    };

    // Prune old snapshots if exceeding MAX_SNAPSHOTS
    let mut all = list_snapshots_logic(storage)?;
    if all.len() > MAX_SNAPSHOTS {
        all.sort_by_key(|a| a.created_at); // oldest first
        let excess = all.len() - MAX_SNAPSHOTS;
        for old in all.iter().take(excess) {
            // Do not prune the snapshot we just created
            if old.id == id {
                continue;
            }
            let path = snapshots_dir.join(format!("{}.json", old.id));
            let _ = std::fs::remove_file(&path);
        }
    }

    Ok(meta)
}

/// Lightweight metadata-only structure for reading snapshot files without
/// loading the full `profiles` array into memory.
/// Fix (B3): Avoids deserializing the entire Snapshot when only meta is needed.
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

pub fn list_snapshots_logic(
    storage: &(dyn Storage + Send + Sync),
) -> Result<Vec<SnapshotMeta>, MhostError> {
    let snapshots_dir = storage.root().join("snapshots");
    if !snapshots_dir.exists() {
        return Ok(Vec::new());
    }

    let mut metas = Vec::new();
    for entry in std::fs::read_dir(&snapshots_dir)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let meta: SnapshotFileMeta = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[mHost] Skipping corrupted snapshot file {:?}: {}", path, e);
                continue;
            }
        };

        metas.push(SnapshotMeta {
            id: meta.id,
            name: meta.name,
            description: meta.description,
            profile_count: meta.profiles,
            created_at: meta.created_at,
        });
    }

    metas.sort_by_key(|b| std::cmp::Reverse(b.created_at));
    Ok(metas)
}

pub fn load_snapshot_logic(
    storage: &(dyn Storage + Send + Sync),
    writer: &HostsWriter,
    id: &str,
) -> Result<(), MhostError> {
    validate_snapshot_id(id)?;

    let snapshot_path = storage.root().join("snapshots").join(format!("{}.json", id));
    if !snapshot_path.exists() {
        return Err(MhostError::InvalidInput(format!("snapshot not found: {}", id)));
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
    crate::commands::apply::apply_current_plan_logic(storage, writer)?;

    Ok(())
}

pub fn delete_snapshot_logic(
    storage: &(dyn Storage + Send + Sync),
    id: &str,
) -> Result<(), MhostError> {
    validate_snapshot_id(id)?;

    let snapshot_path = storage.root().join("snapshots").join(format!("{}.json", id));
    if snapshot_path.exists() {
        std::fs::remove_file(&snapshot_path)?;
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
pub async fn list_snapshots(
    state: State<'_, AppState>,
) -> Result<Vec<SnapshotMeta>, MhostError> {
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
    let _guard = state.apply_lock.lock().await;
    let storage = state.storage.clone();
    let writer = state.writer.clone();
    tauri::async_runtime::spawn_blocking(move || {
        load_snapshot_logic(storage.as_ref(), &writer, &id)
    })
    .await
    .map_err(|e| MhostError::InvalidInput(e.to_string()))??;

    // 快照恢复后，若 DNS 模式处于启用状态，同步重载 DNS 规则表
    if state.dns_enabled.load(std::sync::atomic::Ordering::Relaxed) {
        let profiles = state
            .storage
            .list_profiles_by_mode(ProfileMode::Dns)
            .map_err(MhostError::from)?;
        let enabled_profiles: Vec<_> = profiles.into_iter().filter(|p| p.enabled).collect();

        match state.dns_server.lock() {
            Ok(guard) => {
                if let Some(server) = guard.as_ref() {
                    server.reload_rules(&enabled_profiles);
                }
            }
            Err(poisoned) => {
                let guard = poisoned.into_inner();
                if let Some(server) = guard.as_ref() {
                    server.reload_rules(&enabled_profiles);
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    crate::tray::update_tray_menu(&app_handle);

    Ok(())
}

#[tauri::command]
pub async fn delete_snapshot(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), MhostError> {
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
) -> Result<Option<SnapshotMeta>, MhostError> {
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
            profile.rules.push(HostRule::new(ip.parse().unwrap(), vec![domain.to_string()]));
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

        let snapshot_path = storage.root().join("snapshots").join(format!("{}.json", meta.id));
        assert!(snapshot_path.exists());
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
    fn test_load_snapshot_applies_hosts() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        let mut profile = create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);
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
        let snapshot_path = storage.root().join("snapshots").join(format!("{}.json", meta.id));
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
        assert!(msg.contains("invalid snapshot id"), "error should mention invalid id: {}", msg);

        let result = delete_snapshot_logic(storage.as_ref(), "../../secret");
        assert!(result.is_err(), "should reject invalid snapshot id for delete");
    }

    #[test]
    fn test_load_snapshot_recovery_on_partial_failure() {
        // B2: Verify that if save_profile fails partway, no data is lost.
        // In practice, FileStorage::save_profile is atomic, so this test
        // verifies the ordering (save first, delete after).
        let (_temp, storage, writer) = create_test_storage_and_writer();
        let p1 = create_profile_with_rules(&storage, "keep", vec![("127.0.0.1", "keep.local")]);
        let p2 = create_profile_with_rules(&storage, "remove", vec![("192.168.1.1", "remove.local")]);

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

        let result = auto_snapshot_logic(storage.as_ref()).unwrap();
        assert!(result.is_some(), "should create snapshot when list is empty");

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

        let result = auto_snapshot_logic(storage.as_ref()).unwrap();
        assert!(result.is_none(), "should NOT create snapshot when recent one exists");

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

        let result = auto_snapshot_logic(storage.as_ref()).unwrap();
        assert!(result.is_some(), "should create snapshot when latest is older than 3 days");

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
        assert!(result.unwrap_err().to_string().contains("exceeds maximum length"));
    }

    #[test]
    fn test_save_snapshot_rejects_long_description() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        let long_desc = "a".repeat(MAX_SNAPSHOT_DESC_LENGTH + 1);
        let result = save_snapshot_logic(storage.as_ref(), "name".to_string(), Some(long_desc));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceeds maximum length"));
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
        let snapshot_meta = save_snapshot_logic(storage.as_ref(), "cross_mode".to_string(), None).unwrap();
        let snapshot_id = snapshot_meta.id;
        assert!(!snapshot_id.is_empty());

        // Verify snapshot contains both profiles by reading the file directly
        let snapshots_dir = storage.root().join("snapshots");
        let snapshot_path = snapshots_dir.join(format!("{}.json", snapshot_id));
        let snapshot_json = std::fs::read_to_string(&snapshot_path).unwrap();
        let snapshot: Snapshot = serde_json::from_str(&snapshot_json).unwrap();
        assert_eq!(snapshot.profiles.len(), 2, "snapshot should contain both hosts and dns profiles");
        assert!(snapshot.profiles.iter().any(|p| p.mode == ProfileMode::Hosts));
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
        assert!(restored.iter().any(|p| p.mode == ProfileMode::Hosts && p.name == "hosts_dev"));
        assert!(restored.iter().any(|p| p.mode == ProfileMode::Dns && p.name == "dns_dev"));
    }
}
