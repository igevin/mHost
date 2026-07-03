use std::str::FromStr;

use mhost_apply::{ApplyPlan, generate_plan};
use mhost_apply::writer::HostsWriter;
use mhost_core::{MhostError, ProfileId, ProfileMode};
use mhost_storage::storage::Storage;
use tauri::{AppHandle, State};

use crate::commands::profile::disable_other_profiles;
use crate::state::AppState;

#[tauri::command]
pub async fn generate_apply_plan(state: State<'_, AppState>) -> Result<ApplyPlan, MhostError> {
    let storage = state.storage.clone();
    let writer = state.writer.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let profiles = storage.list_profiles_by_mode(ProfileMode::Hosts)?;
        let current_hosts = match std::fs::read_to_string(writer.hosts_path()) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                return Err(e.into());
            }
        };
        generate_plan(&profiles, &current_hosts).map_err(Into::into)
    }).await.map_err(|e| MhostError::InvalidInput(e.to_string()))?
}

/// Generate a preview plan for enabling/disabling a profile without modifying storage.
///
/// Testable without Tauri `State`.
pub fn generate_preview_plan_logic(
    id: &ProfileId,
    enabled: bool,
    storage: &(dyn Storage + Send + Sync),
    writer: &HostsWriter,
) -> Result<ApplyPlan, MhostError> {
    // DNS 模式 Profile 不写入 hosts，预览返回空 plan
    let target_profile = storage.load_profile(id)?;
    if target_profile.mode == ProfileMode::Dns {
        return Ok(ApplyPlan {
            rules: vec![],
            conflicts: vec![],
            diff: mhost_core::HostsDiff {
                added: vec![],
                removed: vec![],
                unchanged: vec![],
            },
            backup_required: false,
        });
    }

    let mut profiles = storage.list_profiles_by_mode(ProfileMode::Hosts)?;

    let mut found = false;
    for profile in &mut profiles {
        if profile.id == *id {
            profile.enabled = enabled;
            found = true;
        } else if enabled {
            // Single-profile exclusive mode: disable others when enabling target
            profile.enabled = false;
        }
    }

    if !found {
        return Err(MhostError::InvalidInput(format!("profile not found: {}", id)));
    }

    let current_hosts = match std::fs::read_to_string(writer.hosts_path()) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(e.into());
        }
    };

    generate_plan(&profiles, &current_hosts).map_err(Into::into)
}

/// Preview the apply plan for enabling/disabling a profile without modifying storage.
///
/// This is a pure query command: no storage state is mutated and no hosts file is written.
#[tauri::command]
pub async fn generate_preview_plan(
    id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<ApplyPlan, MhostError> {
    let storage = state.storage.clone();
    let writer = state.writer.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let profile_id = std::str::FromStr::from_str(&id)?;
        generate_preview_plan_logic(&profile_id, enabled, storage.as_ref(), &writer)
    }).await.map_err(|e| MhostError::InvalidInput(e.to_string()))?
}

/// Reject an empty plan (no enabled profiles with rules).
///
/// Extracted as a pure function so it can be tested without Tauri `State`.
/// Enhancement (#2-N1): Avoids writing empty managed block.
pub fn reject_empty_plan(plan: &ApplyPlan) -> Result<(), MhostError> {
    if plan.rules.is_empty() {
        return Err(MhostError::InvalidInput(
            "No enabled profiles with rules to apply".to_string(),
        ));
    }
    Ok(())
}

/// Apply hosts using server-side generated plan.
///
/// Security fix (#14): No longer accepts `ApplyPlan` from the frontend.
/// The plan is generated server-side from storage to prevent arbitrary hosts injection.
/// Security fix (#16): Uses apply_lock to prevent concurrent writes.
/// Perf fix (#26): Async with spawn_blocking to avoid blocking executor.
/// Enhancement (#2-N1): Rejects empty plan to avoid writing empty managed block.
#[tauri::command]
pub async fn apply_hosts(state: State<'_, AppState>) -> Result<(), MhostError> {
    let _guard = state.apply_lock.lock().await;
    let writer = state.writer.clone();
    let storage = state.storage.clone();
    tauri::async_runtime::spawn_blocking(move || {
        eprintln!("[mHost] Waiting for user authorization (if needed)...");
        let profiles = storage.list_profiles_by_mode(ProfileMode::Hosts)?;
        let current_hosts = match std::fs::read_to_string(writer.hosts_path()) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                return Err(e.into());
            }
        };
        let plan = generate_plan(&profiles, &current_hosts)?;

        // N1: Reject empty plan to avoid writing empty managed block
        reject_empty_plan(&plan)?;

        writer.apply(&plan)?;

        // Write last_applied timestamp only on success
        write_last_applied(&storage.root())?;

        // Auto-snapshot after successful apply
        if let Err(e) = crate::commands::snapshot::auto_snapshot_logic(storage.as_ref()) {
            eprintln!("[mHost] Auto-snapshot failed: {}", e);
        }

        Ok(())
    }).await.map_err(|e| MhostError::InvalidInput(e.to_string()))?
}

/// Core logic: regenerate plan from all enabled profiles and apply to system hosts.
///
/// Testable without Tauri `State`.
/// Fix (#44): Extracted as a reusable function so `update_profile` can re-apply
/// when saving an enabled profile.
pub fn apply_current_plan_logic(
    storage: &(dyn Storage + Send + Sync),
    writer: &HostsWriter,
) -> Result<(), MhostError> {
    let profiles = storage.list_profiles_by_mode(ProfileMode::Hosts)?;
    let current_hosts = match std::fs::read_to_string(writer.hosts_path()) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(e.into());
        }
    };
    let plan = generate_plan(&profiles, &current_hosts)?;

    // Note: When no profiles are enabled, empty plan is expected (clears managed block).
    // When a profile is enabled but has no rules, empty plan is still valid
    // (managed block will be empty until rules are added).
    // This is intentionally NOT rejected here; rejection of empty plans is the
    // responsibility of the `apply_hosts` command (issue #2-N1) to avoid writing
    // an empty managed block when the user explicitly clicks "Apply".
    writer.apply(&plan)?;

    // Record timestamp (only after successful apply)
    write_last_applied(&storage.root())?;

    Ok(())
}

/// Core logic: enable a hosts-mode profile and immediately apply its rules to the system hosts file.
///
/// Testable without Tauri `State`.
/// Only for hosts mode profiles. DNS mode profiles should use DNS reload instead.
pub fn enable_and_apply_logic(
    id: &ProfileId,
    enabled: bool,
    storage: &(dyn Storage + Send + Sync),
    writer: &HostsWriter,
) -> Result<(), MhostError> {
    // 1. Toggle enabled state in storage (same logic as set_profile_enabled)
    if enabled {
        disable_other_profiles(storage, id)?;
    }
    let mut profile = storage.load_profile(id)?;
    profile.enabled = enabled;
    profile.updated_at = chrono::Utc::now();
    storage.save_profile(&profile)?;

    // 2. Apply current plan (hosts mode only)
    apply_current_plan_logic(storage, writer)?;

    Ok(())
}

/// Enable a profile and immediately apply its rules.
///
/// For hosts mode: toggles the profile and applies to /etc/hosts.
/// For dns mode: toggles the profile and reloads DNS rules if DNS mode is enabled.
/// Security fix (#16): Uses apply_lock to prevent concurrent writes.
/// Perf fix (#26): Async with spawn_blocking to avoid blocking executor.
#[tauri::command]
pub async fn enable_and_apply(
    id: String,
    enabled: bool,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), MhostError> {
    let profile_id = ProfileId::from_str(&id)?;
    let profile = state.storage.load_profile(&profile_id)?;

    if profile.mode == ProfileMode::Hosts {
        let _guard = state.apply_lock.lock().await;
        let writer = state.writer.clone();
        let storage = state.storage.clone();
        tauri::async_runtime::spawn_blocking(move || {
            enable_and_apply_logic(&profile_id, enabled, storage.as_ref(), &writer)?;

            // Auto-snapshot after successful apply
            if let Err(e) = crate::commands::snapshot::auto_snapshot_logic(storage.as_ref()) {
                eprintln!("[mHost] Auto-snapshot failed: {}", e);
            }

            Ok::<(), MhostError>(())
        })
        .await
        .map_err(|e| MhostError::InvalidInput(e.to_string()))??;
    } else {
        // DNS 模式：直接启用/禁用，然后热重载规则
        let mut profile = profile;
        profile.enabled = enabled;
        profile.updated_at = chrono::Utc::now();
        state.storage.save_profile(&profile)?;

        if enabled && state.dns_enabled.load(std::sync::atomic::Ordering::Relaxed) {
            let profiles = state
                .storage
                .list_profiles_by_mode(ProfileMode::Dns)?;
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
    }

    #[cfg(target_os = "macos")]
    crate::tray::update_tray_menu(&app_handle);
    Ok(())
}

#[tauri::command]
pub async fn rollback_hosts(state: State<'_, AppState>) -> Result<(), MhostError> {
    let _guard = state.apply_lock.lock().await;
    let writer = state.writer.clone();
    tauri::async_runtime::spawn_blocking(move || {
        writer.rollback().map_err(Into::into)
    }).await.map_err(|e| MhostError::InvalidInput(e.to_string()))?
}

#[tauri::command]
pub async fn read_system_hosts() -> Result<String, MhostError> {
    tauri::async_runtime::spawn_blocking(|| {
        std::fs::read_to_string("/etc/hosts").map_err(Into::into)
    }).await.map_err(|e| MhostError::InvalidInput(e.to_string()))?
}

#[tauri::command]
pub async fn get_managed_block_content(
    state: State<'_, AppState>,
) -> Result<Option<String>, MhostError> {
    let writer = state.writer.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let hosts_text = std::fs::read_to_string(writer.hosts_path())?;
        Ok(mhost_hosts::parser::Parser::extract_managed_block_content(
            &hosts_text,
        ))
    }).await.map_err(|e| MhostError::InvalidInput(e.to_string()))?
}

/// Strong-typed struct for last_applied.json to prevent recursive deserialization attacks.
/// Security fix (#20): Replaces bare `serde_json::Value` deserialization.
#[derive(serde::Deserialize)]
struct LastApplied {
    timestamp: String,
    #[allow(dead_code)]
    profile_id: Option<String>,
}

#[tauri::command]
pub async fn get_last_applied(state: State<'_, AppState>) -> Result<Option<String>, MhostError> {
    let root = state.storage.root().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let path = root.join("last_applied.json");
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)?;
        let data: LastApplied = serde_json::from_str(&content).map_err(|e| MhostError::InvalidInput(
            format!("failed to parse last_applied.json: {}", e),
        ))?;
        Ok(Some(data.timestamp))
    }).await.map_err(|e| MhostError::InvalidInput(e.to_string()))?
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
            &profile_b.id,
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
            &profile.id,
            true,
            storage.as_ref(),
            &writer,
        );
        assert!(result.is_ok());

        let hosts_before = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(hosts_before.contains("# ---- mHost start ----"));

        // Now disable the profile
        let result = enable_and_apply_logic(
            &profile.id,
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

    #[test]
    fn test_apply_hosts_rejects_empty_plan() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Create a profile but do NOT enable it
        let profile = create_profile_with_rules(
            &storage,
            "dev",
            vec![("127.0.0.1", "example.com")],
        );
        // profile.enabled defaults to false
        storage.save_profile(&profile).unwrap();

        // Generate plan through the actual business logic
        let profiles = storage.list_profiles().unwrap();
        let current_hosts = std::fs::read_to_string(writer.hosts_path()).unwrap();
        let plan = generate_plan(&profiles, &current_hosts).unwrap();

        // Verify plan is empty
        assert!(plan.rules.is_empty(), "plan should be empty when no profiles are enabled");

        // Verify reject_empty_plan correctly rejects the empty plan
        let result = reject_empty_plan(&plan);
        assert!(result.is_err(), "should reject empty plan");
        let err = result.unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("No enabled profiles"),
            "error message should mention 'No enabled profiles': {}",
            err_str
        );
    }

    #[test]
    fn test_reject_empty_plan_accepts_non_empty() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Create and enable a profile
        let mut profile = create_profile_with_rules(
            &storage,
            "dev",
            vec![("127.0.0.1", "example.com")],
        );
        profile.enabled = true;
        storage.save_profile(&profile).unwrap();

        // Generate plan through the actual business logic
        let profiles = storage.list_profiles().unwrap();
        let current_hosts = std::fs::read_to_string(writer.hosts_path()).unwrap();
        let plan = generate_plan(&profiles, &current_hosts).unwrap();

        // Verify plan is NOT empty
        assert!(!plan.rules.is_empty(), "plan should have rules when a profile is enabled");

        // Verify reject_empty_plan accepts the non-empty plan
        let result = reject_empty_plan(&plan);
        assert!(result.is_ok(), "should accept non-empty plan: {:?}", result.err());
    }

    // Fix (#44): Test that apply_current_plan_logic re-applies enabled profiles
    // after their rules are updated (simulating update_profile on an enabled profile).
    #[test]
    fn test_apply_current_plan_logic_reapplies_after_rule_update() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Create and enable a profile with initial rules
        let mut profile = create_profile_with_rules(
            &storage,
            "dev",
            vec![("127.0.0.1", "example.com")],
        );
        profile.enabled = true;
        storage.save_profile(&profile).unwrap();

        // Initial apply
        let result = apply_current_plan_logic(storage.as_ref(), &writer);
        assert!(result.is_ok(), "initial apply should succeed: {:?}", result.err());

        let hosts_before = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(
            hosts_before.contains("127.0.0.1 example.com"),
            "hosts should contain initial rule: {}",
            hosts_before
        );
        assert!(
            !hosts_before.contains("192.168.1.1 new.local"),
            "hosts should NOT contain new rule yet: {}",
            hosts_before
        );

        // Simulate update_profile: update rules and save
        profile.rules.push(HostRule::new(
            "192.168.1.1".parse().unwrap(),
            vec!["new.local".to_string()],
        ));
        profile.updated_at = chrono::Utc::now();
        storage.save_profile(&profile).unwrap();

        // Re-apply (this is what update_profile does when profile.enabled == true)
        let result = apply_current_plan_logic(storage.as_ref(), &writer);
        assert!(result.is_ok(), "re-apply after rule update should succeed: {:?}", result.err());

        // Verify new rules are now in hosts file
        let hosts_after = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(
            hosts_after.contains("127.0.0.1 example.com"),
            "hosts should still contain original rule: {}",
            hosts_after
        );
        assert!(
            hosts_after.contains("192.168.1.1 new.local"),
            "hosts should now contain new rule: {}",
            hosts_after
        );
    }

    // Fix (#44): Test that apply_current_plan_logic clears rules when profile is disabled.
    #[test]
    fn test_apply_current_plan_logic_clears_when_no_enabled_profiles() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Create and enable a profile
        let mut profile = create_profile_with_rules(
            &storage,
            "dev",
            vec![("127.0.0.1", "example.com")],
        );
        profile.enabled = true;
        storage.save_profile(&profile).unwrap();

        // Apply so managed block exists
        let result = apply_current_plan_logic(storage.as_ref(), &writer);
        assert!(result.is_ok());

        let hosts_before = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(hosts_before.contains("# ---- mHost start ----"));

        // Disable the profile (simulating set_profile_enabled -> false)
        profile.enabled = false;
        storage.save_profile(&profile).unwrap();

        // Re-apply
        let result = apply_current_plan_logic(storage.as_ref(), &writer);
        assert!(result.is_ok(), "re-apply after disable should succeed: {:?}", result.err());

        // Verify managed block is removed
        let hosts_after = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(
            !hosts_after.contains("# ---- mHost start ----"),
            "managed block should be removed when no profiles enabled: {}",
            hosts_after
        );
        assert!(
            hosts_after.contains("# original hosts"),
            "original content should be preserved: {}",
            hosts_after
        );
    }

    #[test]
    fn test_generate_preview_plan_enable_shows_target_rules() {
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

        // Preview enabling profile_b (should disable profile_a)
        let plan = generate_preview_plan_logic(
            &profile_b.id,
            true,
            storage.as_ref(),
            &writer,
        ).unwrap();

        // Verify plan contains profile_b's rules
        assert!(
            plan.rules.iter().any(|r| r.ip.to_string() == "192.168.1.1" && r.domain == "test.local"),
            "plan should contain profile_b rules: {:?}",
            plan.rules
        );
        // Verify plan does NOT contain profile_a's rules
        assert!(
            !plan.rules.iter().any(|r| r.ip.to_string() == "127.0.0.1" && r.domain == "example.com"),
            "plan should NOT contain profile_a rules: {:?}",
            plan.rules
        );

        // Verify storage state was NOT modified
        let loaded_a = storage.load_profile(&profile_a.id).unwrap();
        assert!(loaded_a.enabled, "profile_a should still be enabled in storage");
        let loaded_b = storage.load_profile(&profile_b.id).unwrap();
        assert!(!loaded_b.enabled, "profile_b should still be disabled in storage");
    }

    #[test]
    fn test_generate_preview_plan_disable_shows_empty_plan() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Create and enable a profile
        let mut profile = create_profile_with_rules(
            &storage,
            "dev",
            vec![("127.0.0.1", "example.com")],
        );
        profile.enabled = true;
        storage.save_profile(&profile).unwrap();

        // Preview disabling the profile
        let plan = generate_preview_plan_logic(
            &profile.id,
            false,
            storage.as_ref(),
            &writer,
        ).unwrap();

        // Verify plan is empty
        assert!(plan.rules.is_empty(), "plan should be empty when disabling the only enabled profile: {:?}", plan.rules);

        // Verify storage state was NOT modified
        let loaded = storage.load_profile(&profile.id).unwrap();
        assert!(loaded.enabled, "profile should still be enabled in storage");
    }

    #[test]
    fn test_apply_current_plan_logic_ignores_dns_profiles() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Create a hosts profile with rules and enable it
        let mut hosts_profile = create_profile_with_rules(
            &storage,
            "hosts_dev",
            vec![("127.0.0.1", "hosts.example.com")],
        );
        hosts_profile.enabled = true;
        storage.save_profile(&hosts_profile).unwrap();

        // Create a DNS profile with rules and enable it
        let mut dns_profile = create_profile_with_rules(
            &storage,
            "dns_dev",
            vec![("192.168.1.1", "dns.example.com")],
        );
        dns_profile.mode = ProfileMode::Dns;
        dns_profile.enabled = true;
        storage.save_profile(&dns_profile).unwrap();

        // Apply current plan
        let result = apply_current_plan_logic(storage.as_ref(), &writer);
        assert!(result.is_ok(), "apply should succeed: {:?}", result.err());

        // Verify hosts file only contains hosts profile rules
        let hosts_content = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(
            hosts_content.contains("127.0.0.1 hosts.example.com"),
            "hosts should contain hosts profile rules: {}",
            hosts_content
        );
        assert!(
            !hosts_content.contains("192.168.1.1 dns.example.com"),
            "hosts should NOT contain dns profile rules: {}",
            hosts_content
        );
    }

    #[test]
    fn test_generate_preview_plan_ignores_dns_profiles() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Create and enable a DNS profile (set mode before first save)
        let mut dns_profile = Profile::new("dns_dev");
        dns_profile.mode = ProfileMode::Dns;
        dns_profile.rules.push(HostRule::new(
            "192.168.1.1".parse().unwrap(),
            vec!["dns.example.com".to_string()],
        ));
        dns_profile.enabled = true;
        storage.save_profile(&dns_profile).unwrap();

        // Preview plan should be empty (DNS profile does not affect hosts)
        let plan = generate_preview_plan_logic(
            &dns_profile.id,
            true,
            storage.as_ref(),
            &writer,
        ).unwrap();

        assert!(plan.rules.is_empty(), "plan should be empty when target is dns profile: {:?}", plan.rules);
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
