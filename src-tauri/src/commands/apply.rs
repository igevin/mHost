use mhost_apply::{ApplyPlan, generate_plan};
use mhost_apply::writer::HostsWriter;
use mhost_core::MhostError;
use mhost_storage::storage::Storage;
use tauri::State;

use crate::commands::profile::disable_other_profiles;
use crate::state::AppState;

#[tauri::command]
pub fn generate_apply_plan(state: State<'_, AppState>) -> Result<ApplyPlan, MhostError> {
    let profiles = state.storage.list_profiles()?;
    let current_hosts = match std::fs::read_to_string("/etc/hosts") {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(MhostError::Io {
                kind: e.kind().to_string(),
                message: e.to_string(),
            });
        }
    };
    generate_plan(&profiles, &current_hosts).map_err(Into::into)
}

#[tauri::command]
pub fn apply_hosts(plan: ApplyPlan, state: State<'_, AppState>) -> Result<(), MhostError> {
    state.writer.apply(&plan)?;

    // Write last_applied timestamp on success (non-fatal: must not break apply)
    write_last_applied(&state.storage.root())?;

    Ok(())
}

/// Core logic: enable a profile and immediately apply its rules to the system hosts file.
///
/// Testable without Tauri `State`.
pub fn enable_and_apply_logic(
    id: &str,
    enabled: bool,
    storage: &(dyn Storage + Send + Sync),
    writer: &HostsWriter,
) -> Result<(), MhostError> {
    // 1. Toggle enabled state in storage (same logic as set_profile_enabled)
    let profile_id = std::str::FromStr::from_str(id)?;
    if enabled {
        disable_other_profiles(storage, &profile_id)?;
    }
    let mut profile = storage.load_profile(&profile_id)?;
    profile.enabled = enabled;
    profile.updated_at = chrono::Utc::now();
    storage.save_profile(&profile)?;

    // 2. Reload all profiles and generate plan
    let profiles = storage.list_profiles()?;
    let current_hosts = match std::fs::read_to_string("/etc/hosts") {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(MhostError::Io {
                kind: e.kind().to_string(),
                message: e.to_string(),
            });
        }
    };
    let plan = generate_plan(&profiles, &current_hosts)?;

    // 3. Apply to system hosts
    writer.apply(&plan)?;

    // 4. Record timestamp
    write_last_applied(&storage.root())?;

    Ok(())
}

/// Enable a profile and immediately apply its rules to the system hosts file.
///
/// This is the primary user-facing command: toggling a profile on/off should
/// instantly update `/etc/hosts` (with a macOS authorization prompt).
///
/// Flow:
/// 1. Set the target profile as enabled (disable all others — phase 0 constraint).
/// 2. Reload all profiles from storage.
/// 3. Generate an apply plan from the newly-enabled profiles.
/// 4. Write the plan to `/etc/hosts` (triggers macOS auth prompt).
/// 5. Record last-applied timestamp.
///
/// If `enabled` is `false`, the managed block is removed from hosts (all profiles
/// disabled → empty plan → managed block cleared).
#[tauri::command]
pub fn enable_and_apply(
    id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), MhostError> {
    enable_and_apply_logic(&id, enabled, state.storage.as_ref(), &state.writer)
}

#[tauri::command]
pub fn rollback_hosts(state: State<'_, AppState>) -> Result<(), MhostError> {
    state.writer.rollback().map_err(Into::into)
}

#[tauri::command]
pub fn read_system_hosts() -> Result<String, MhostError> {
    std::fs::read_to_string("/etc/hosts").map_err(|e| MhostError::Io {
        kind: e.kind().to_string(),
        message: e.to_string(),
    })
}

#[tauri::command]
pub fn get_managed_block_content(
    state: State<'_, AppState>,
) -> Result<Option<String>, MhostError> {
    let hosts_text = std::fs::read_to_string(state.writer.hosts_path()).map_err(|e| {
        MhostError::Io {
            kind: e.kind().to_string(),
            message: e.to_string(),
        }
    })?;
    Ok(mhost_hosts::parser::Parser::extract_managed_block_content(
        &hosts_text,
    ))
}

#[tauri::command]
pub fn get_last_applied(state: State<'_, AppState>) -> Result<Option<String>, MhostError> {
    let path = state.storage.root().join("last_applied.json");
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path).map_err(|e| MhostError::Io {
        kind: e.kind().to_string(),
        message: e.to_string(),
    })?;
    let data: serde_json::Value = serde_json::from_str(&content).map_err(|e| MhostError::Io {
        kind: "serde_json".to_string(),
        message: e.to_string(),
    })?;
    Ok(data
        .get("timestamp")
        .and_then(|v| v.as_str())
        .map(String::from))
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
    fn test_enable_and_apply_enables_profile_and_writes_hosts() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Create two profiles, enable the first one
        let mut profile_a = create_profile_with_rules(
            &storage,
            "dev",
            vec![("127.0.0.1", "example.com")],
        );
        profile_a.enabled = true;
        storage.save_profile(&profile_a).unwrap();

        let profile_b = create_profile_with_rules(
            &storage,
            "test",
            vec![("192.168.1.1", "test.local")],
        );

        // Enable profile_b via enable_and_apply_logic
        let result = enable_and_apply_logic(
            &profile_b.id.to_string(),
            true,
            storage.as_ref(),
            &writer,
        );
        assert!(result.is_ok(), "enable_and_apply should succeed: {:?}", result.err());

        // Verify profile_b is enabled
        let loaded_b = storage.load_profile(&profile_b.id).unwrap();
        assert!(loaded_b.enabled, "profile_b should be enabled");

        // Verify profile_a is disabled
        let loaded_a = storage.load_profile(&profile_a.id).unwrap();
        assert!(!loaded_a.enabled, "profile_a should be disabled");

        // Verify hosts file contains profile_b's rules
        let hosts_content = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(
            hosts_content.contains("192.168.1.1 test.local"),
            "hosts should contain profile_b rules: {}",
            hosts_content
        );
        assert!(
            !hosts_content.contains("127.0.0.1 example.com"),
            "hosts should NOT contain profile_a rules: {}",
            hosts_content
        );
    }

    #[test]
    fn test_enable_and_apply_disable_removes_managed_block() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Create and enable a profile
        let mut profile = create_profile_with_rules(
            &storage,
            "dev",
            vec![("127.0.0.1", "example.com")],
        );
        profile.enabled = true;
        storage.save_profile(&profile).unwrap();

        // Apply first so there's a managed block
        let result = enable_and_apply_logic(
            &profile.id.to_string(),
            true,
            storage.as_ref(),
            &writer,
        );
        assert!(result.is_ok());

        let hosts_before = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(hosts_before.contains("# ---- mHost start ----"));

        // Now disable the profile
        let result = enable_and_apply_logic(
            &profile.id.to_string(),
            false,
            storage.as_ref(),
            &writer,
        );
        assert!(result.is_ok(), "disable should succeed: {:?}", result.err());

        // Verify profile is disabled
        let loaded = storage.load_profile(&profile.id).unwrap();
        assert!(!loaded.enabled);

        // Verify hosts file no longer has managed block
        let hosts_after = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(
            !hosts_after.contains("# ---- mHost start ----"),
            "managed block should be removed: {}",
            hosts_after
        );
        assert!(
            !hosts_after.contains("127.0.0.1 example.com"),
            "rule should be removed: {}",
            hosts_after
        );
        assert!(
            hosts_after.contains("# original hosts"),
            "original content should be preserved: {}",
            hosts_after
        );
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_last_applied(root: &std::path::Path) -> Result<(), MhostError> {
    let last_applied_path = root.join("last_applied.json");
    let timestamp = chrono::Utc::now().to_rfc3339();
    let data = serde_json::json!({ "timestamp": timestamp });
    let json = match serde_json::to_string_pretty(&data) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("Warning: failed to serialize last_applied: {}", e);
            return Ok(());
        }
    };
    if let Err(e) = std::fs::write(&last_applied_path, json) {
        eprintln!(
            "[mHost] Warning: failed to write last_applied.json: {}",
            e
        );
    }
    Ok(())
}
