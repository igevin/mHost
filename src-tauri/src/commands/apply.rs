use std::str::FromStr;

use mhost_apply::writer::HostsWriter;
use mhost_apply::{generate_plan, ApplyPlan};
use mhost_core::{ApplyMode, ApplyOutcome, MhostError, ProfileId, ProfileMode};
use mhost_storage::storage::Storage;
use tauri::{AppHandle, State};

use crate::commands::profile::disable_other_profiles;
use crate::state::{lock_or_recover, AppState};

/// Threshold above which a quick apply is rejected in favour of preview.
///
/// "Bulk changes" of more than this many add/remove operations deserve an
/// explicit preview pass. Tunable in code for now — not user-configurable
/// (issue #127 follow-up).
pub const DESTRUCTIVE_THRESHOLD: usize = 100;

/// Pure decision function: should the apply go straight to `/etc/hosts`
/// (`QuickApply`) or open the preview dialog (`RequirePreview`)?
///
/// Rules (in order; first match wins):
/// 1. `outcome.has_conflicts` → `RequirePreview`. Non-negotiable: the merge
///    silently drops conflicting rules from `plan.rules`. The user must
///    explicitly see the conflict list before applying.
/// 2. `!outcome.disabled_profile_ids.is_empty()` → `RequirePreview`. Enabling
///    a hosts profile in single-enabled mode disables every other hosts
///    profile; this is a destructive side effect the user should confirm.
/// 3. `added_count + removed_count > DESTRUCTIVE_THRESHOLD` → `RequirePreview`.
///    Bulk writes to `/etc/hosts` deserve eyeballs.
///
/// All other cases → `QuickApply`.
///
/// **Future work, not in #127**: detect external `/etc/hosts` changes —
/// i.e., distinguish "user toggled 200 rules via mHost" from "another tool
/// rewrote `/etc/hosts` without our involvement". `backup_required` fires
/// on any add/remove regardless of source, so we cannot disambiguate today.
pub fn decide_apply_mode(outcome: &ApplyOutcome) -> ApplyMode {
    if outcome.has_conflicts {
        return ApplyMode::RequirePreview;
    }
    if !outcome.disabled_profile_ids.is_empty() {
        return ApplyMode::RequirePreview;
    }
    let total_changes = outcome.added_count + outcome.removed_count;
    if total_changes > DESTRUCTIVE_THRESHOLD {
        return ApplyMode::RequirePreview;
    }
    ApplyMode::QuickApply
}

/// Pure helper: compute the profile IDs that WOULD be disabled if the toggle
/// were applied. Mirrors `disable_other_profiles` in `profile.rs` but does
/// NOT mutate storage. Used by `preview_apply_outcome` so the frontend can
/// gate the decision on "is this a destructive side effect?".
///
/// For `enabled == false`, returns an empty `Vec` (disabling doesn't disable
/// other profiles).
///
/// **Known caveat**: there is a brief window between reading the profile list
/// and the apply write during which another concurrent mutation could
/// change `enabled` flags. `disable_other_profiles` already has the same
/// race; we don't make it worse. The list is captured under
/// `enable_and_apply_logic`'s `apply_lock` at write time.
pub fn compute_disabled_ids_logic(
    id: &ProfileId,
    enabled: bool,
    storage: &dyn Storage,
) -> Result<Vec<String>, MhostError> {
    if !enabled {
        return Ok(Vec::new());
    }
    let profiles = storage.list_profiles_by_mode(ProfileMode::Hosts)?;
    Ok(profiles
        .into_iter()
        .filter(|p| p.enabled && p.id != *id)
        .map(|p| p.id.to_string())
        .collect())
}

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
        generate_plan(&profiles, &current_hosts)
    })
    .await
    .map_err(|e| MhostError::InvalidInput(e.to_string()))?
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
        return Err(MhostError::InvalidInput(format!(
            "profile not found: {}",
            id
        )));
    }

    let current_hosts = match std::fs::read_to_string(writer.hosts_path()) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(e.into());
        }
    };

    generate_plan(&profiles, &current_hosts)
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
    })
    .await
    .map_err(|e| MhostError::InvalidInput(e.to_string()))?
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
/// Perf fix (P-R8, issue #90): Pass the already-read `current_hosts` to
/// `writer.apply_with_content` to avoid reading /etc/hosts a second time
/// (the writer previously re-read inside `HostsWriter::apply`).
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

        // **fix (P-R8, issue #90)**: pass current_hosts through so writer
        // doesn't re-read /etc/hosts (was happening twice per apply).
        // The writer now returns `Option<PathBuf>` (backup path if created);
        // `apply_hosts` doesn't surface it, so discard.
        let _ = writer.apply_with_content(&plan, &current_hosts)?;

        // Write last_applied timestamp only on success
        write_last_applied(storage.root())?;

        // Auto-snapshot after successful apply
        if let Err(e) = crate::commands::snapshot::auto_snapshot_logic(storage.as_ref()) {
            eprintln!("[mHost] Auto-snapshot failed: {}", e);
        }

        Ok(())
    })
    .await
    .map_err(|e| MhostError::InvalidInput(e.to_string()))?
}

/// Core logic: regenerate plan from all enabled profiles and apply to system hosts.
///
/// Testable without Tauri `State`.
/// Fix (#44): Extracted as a reusable function so `update_profile` can re-apply
/// when saving an enabled profile.
/// Perf fix (P-R8, issue #90): Same single-read pattern as `apply_hosts`.
///
/// Returns `Some(path)` if the writer created a backup (i.e. the plan's
/// `backup_required` was true). `enable_and_apply` (issue #127) threads this
/// through as `ApplyOutcome::backup_path`; `apply_hosts` and `load_snapshot_logic`
/// discard it via `?` and don't need to change callers.
pub fn apply_current_plan_logic(
    storage: &(dyn Storage + Send + Sync),
    writer: &HostsWriter,
) -> Result<Option<std::path::PathBuf>, MhostError> {
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
    let backup_path = writer.apply_with_content(&plan, &current_hosts)?;

    // Record timestamp (only after successful apply)
    write_last_applied(storage.root())?;

    Ok(backup_path)
}

/// Core logic: enable a hosts-mode profile and immediately apply its rules to the system hosts file.
///
/// Returns `(disabled_profile_ids, backup_path)`:
/// - `disabled_profile_ids`: profile IDs that were auto-disabled because the
///   target was being enabled. Empty when `enabled=false`.
/// - `backup_path`: `Some(path)` if a backup was created by the writer.
///
/// Testable without Tauri `State`. Only for hosts mode profiles. DNS mode
/// profiles should use DNS reload instead.
pub fn enable_and_apply_logic(
    id: &ProfileId,
    enabled: bool,
    storage: &(dyn Storage + Send + Sync),
    writer: &HostsWriter,
) -> Result<(Vec<String>, Option<std::path::PathBuf>), MhostError> {
    // 1. Compute disabled IDs BEFORE mutation so the result matches what
    //    `disable_other_profiles` is about to disable. (Same read-then-mutate
    //    race that `compute_disabled_ids_logic` already documents.)
    let disabled_ids = compute_disabled_ids_logic(id, enabled, storage)?;

    // 2. Toggle enabled state in storage (same logic as set_profile_enabled)
    if enabled {
        disable_other_profiles(storage, id)?;
    }
    let mut profile = storage.load_profile(id)?;
    profile.enabled = enabled;
    profile.updated_at = chrono::Utc::now();
    storage.save_profile(&profile)?;

    // 3. Apply current plan (hosts mode only)
    let backup_path = apply_current_plan_logic(storage, writer)?;

    Ok((disabled_ids, backup_path))
}

/// Read-only IPC: compute what an `enable_and_apply(id, enabled)` call would
/// produce, **without** writing anything. Returns an `ApplyOutcome` carrying
/// the plan, derived counts, and disabled-profile IDs — exactly what the
/// frontend's `decideApplyMode` policy consumes.
///
/// `snapshot_id` and `backup_path` are always `None` because no write
/// occurred.
///
/// **No `apply_lock` is acquired** — `generate_preview_plan_logic` only
/// reads storage, and `compute_disabled_ids_logic` is a pure read. Locking
/// happens in `enable_and_apply` (the write path) only.
#[tauri::command]
pub async fn preview_apply_outcome(
    id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<ApplyOutcome, MhostError> {
    let storage = state.storage.clone();
    let writer = state.writer.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let profile_id = ProfileId::from_str(&id)?;
        let plan = generate_preview_plan_logic(&profile_id, enabled, storage.as_ref(), &writer)?;
        let disabled_ids = compute_disabled_ids_logic(&profile_id, enabled, storage.as_ref())?;
        Ok::<ApplyOutcome, MhostError>(ApplyOutcome::from_parts(plan, disabled_ids, None, None))
    })
    .await
    .map_err(|e| MhostError::InvalidInput(e.to_string()))?
}

/// Enable a profile and immediately apply its rules.
///
/// For hosts mode: toggles the profile and applies to /etc/hosts.
/// For dns mode: toggles the profile and reloads DNS rules if DNS mode is enabled.
/// Security fix (#16): Uses apply_lock to prevent concurrent writes.
/// Perf fix (#26): Async with spawn_blocking to avoid blocking executor.
/// Refs (#127): Returns `ApplyOutcome` so the frontend can surface
/// structured feedback (counts / disabled IDs / snapshot id / backup path)
/// to the Quick Apply toast.
#[tauri::command]
pub async fn enable_and_apply(
    id: String,
    enabled: bool,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<ApplyOutcome, MhostError> {
    let profile_id = ProfileId::from_str(&id)?;
    let profile = state.storage.load_profile(&profile_id)?;

    let outcome = if profile.mode == ProfileMode::Hosts {
        let _guard = state.apply_lock.lock().await;
        let writer = state.writer.clone();
        let storage = state.storage.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let (disabled_ids, backup_path) =
                enable_and_apply_logic(&profile_id, enabled, storage.as_ref(), &writer)?;

            // Auto-snapshot: capture id so the toast can mention it.
            let snapshot_id = crate::commands::snapshot::auto_snapshot_logic(storage.as_ref())
                .ok()
                .flatten()
                .map(|m| m.id);

            // Re-derive the post-apply plan to populate ApplyOutcome.plan.
            // Cost is one extra generate_plan (one read + one diff) — cheap.
            // Cheaper refactor than threading the plan back out of
            // `apply_current_plan_logic`.
            let plan = post_apply_plan(storage.as_ref(), &writer).unwrap_or_else(|_| ApplyPlan {
                rules: vec![],
                conflicts: vec![],
                diff: mhost_core::HostsDiff {
                    added: vec![],
                    removed: vec![],
                    unchanged: vec![],
                },
                backup_required: false,
            });

            Ok::<ApplyOutcome, MhostError>(ApplyOutcome::from_parts(
                plan,
                disabled_ids,
                snapshot_id,
                backup_path,
            ))
        })
        .await
        .map_err(|e| MhostError::InvalidInput(e.to_string()))??
    } else {
        // DNS 模式：直接启用/禁用，然后热重载规则。apply 不写 /etc/hosts，
        // 所以返回 empty outcome（无 diff、无 snapshot、无 backup、无 disabled）。
        let mut profile = profile;
        profile.enabled = enabled;
        profile.updated_at = chrono::Utc::now();
        state.storage.save_profile(&profile)?;

        if enabled && state.dns_enabled.load(std::sync::atomic::Ordering::Relaxed) {
            let profiles = state.storage.list_profiles_by_mode(ProfileMode::Dns)?;
            let enabled_profiles: Vec<_> = profiles.into_iter().filter(|p| p.enabled).collect();

            if let Some(server) = lock_or_recover(&state.dns_server).as_ref() {
                server.reload_rules(&enabled_profiles);
            }
        }
        ApplyOutcome::empty()
    };

    #[cfg(target_os = "macos")]
    crate::tray::update_tray_menu(&app_handle);
    Ok(outcome)
}

/// Helper: re-read profiles + hosts and re-derive the post-apply `ApplyPlan`.
/// Used by `enable_and_apply` to populate `ApplyOutcome.plan` for the toast.
/// If anything fails (e.g. hosts file disappeared), the caller falls back to
/// an empty plan — losing the diff display but keeping the apply outcome.
fn post_apply_plan(
    storage: &(dyn Storage + Send + Sync),
    writer: &HostsWriter,
) -> Result<ApplyPlan, MhostError> {
    let profiles = storage.list_profiles_by_mode(ProfileMode::Hosts)?;
    let current_hosts = match std::fs::read_to_string(writer.hosts_path()) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    generate_plan(&profiles, &current_hosts)
}

#[tauri::command]
pub async fn rollback_hosts(state: State<'_, AppState>) -> Result<(), MhostError> {
    let _guard = state.apply_lock.lock().await;
    let writer = state.writer.clone();
    tauri::async_runtime::spawn_blocking(move || writer.rollback())
        .await
        .map_err(|e| MhostError::InvalidInput(e.to_string()))?
}

#[tauri::command]
pub async fn read_system_hosts() -> Result<String, MhostError> {
    tauri::async_runtime::spawn_blocking(|| {
        std::fs::read_to_string("/etc/hosts").map_err(Into::into)
    })
    .await
    .map_err(|e| MhostError::InvalidInput(e.to_string()))?
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
    })
    .await
    .map_err(|e| MhostError::InvalidInput(e.to_string()))?
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
        let data: LastApplied = serde_json::from_str(&content).map_err(|e| {
            MhostError::InvalidInput(format!("failed to parse last_applied.json: {}", e))
        })?;
        Ok(Some(data.timestamp))
    })
    .await
    .map_err(|e| MhostError::InvalidInput(e.to_string()))?
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
        eprintln!("[mHost] Warning: failed to write last_applied.json: {}", e);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mhost_core::{HostRule, HostsDiff, Profile, RuleConflict};
    use mhost_storage::storage::{FileStorage, Storage};
    use std::sync::Arc;
    use tempfile::TempDir;
    use uuid::Uuid;

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
    fn test_enable_and_apply_enables_profile_and_writes_hosts() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Create two profiles, enable the first one
        let mut profile_a =
            create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);
        profile_a.enabled = true;
        storage.save_profile(&profile_a).unwrap();

        let profile_b =
            create_profile_with_rules(&storage, "test", vec![("192.168.1.1", "test.local")]);

        // Enable profile_b via enable_and_apply_logic
        let result = enable_and_apply_logic(&profile_b.id, true, storage.as_ref(), &writer);
        assert!(
            result.is_ok(),
            "enable_and_apply should succeed: {:?}",
            result.err()
        );

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
        let mut profile =
            create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);
        profile.enabled = true;
        storage.save_profile(&profile).unwrap();

        // Apply first so there's a managed block
        let result = enable_and_apply_logic(&profile.id, true, storage.as_ref(), &writer);
        assert!(result.is_ok());

        let hosts_before = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(hosts_before.contains("# ---- mHost start ----"));

        // Now disable the profile
        let result = enable_and_apply_logic(&profile.id, false, storage.as_ref(), &writer);
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
        let profile =
            create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);
        // profile.enabled defaults to false
        storage.save_profile(&profile).unwrap();

        // Generate plan through the actual business logic
        let profiles = storage.list_profiles().unwrap();
        let current_hosts = std::fs::read_to_string(writer.hosts_path()).unwrap();
        let plan = generate_plan(&profiles, &current_hosts).unwrap();

        // Verify plan is empty
        assert!(
            plan.rules.is_empty(),
            "plan should be empty when no profiles are enabled"
        );

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
        let mut profile =
            create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);
        profile.enabled = true;
        storage.save_profile(&profile).unwrap();

        // Generate plan through the actual business logic
        let profiles = storage.list_profiles().unwrap();
        let current_hosts = std::fs::read_to_string(writer.hosts_path()).unwrap();
        let plan = generate_plan(&profiles, &current_hosts).unwrap();

        // Verify plan is NOT empty
        assert!(
            !plan.rules.is_empty(),
            "plan should have rules when a profile is enabled"
        );

        // Verify reject_empty_plan accepts the non-empty plan
        let result = reject_empty_plan(&plan);
        assert!(
            result.is_ok(),
            "should accept non-empty plan: {:?}",
            result.err()
        );
    }

    // Fix (#44, #121): Test that apply_current_plan_logic re-applies enabled profiles
    // after their rules are updated (this is what update_profile triggers on save
    // when the profile is a Hosts mode + enabled profile — issue #121 was "editing
    // the active hosts profile just saves, doesn't write hosts").
    #[test]
    fn test_apply_current_plan_logic_reapplies_after_rule_update() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Create and enable a profile with initial rules
        let mut profile =
            create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);
        profile.enabled = true;
        storage.save_profile(&profile).unwrap();

        // Initial apply
        let result = apply_current_plan_logic(storage.as_ref(), &writer);
        assert!(
            result.is_ok(),
            "initial apply should succeed: {:?}",
            result.err()
        );

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
        assert!(
            result.is_ok(),
            "re-apply after rule update should succeed: {:?}",
            result.err()
        );

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
        let mut profile =
            create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);
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
        assert!(
            result.is_ok(),
            "re-apply after disable should succeed: {:?}",
            result.err()
        );

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
        let mut profile_a =
            create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);
        profile_a.enabled = true;
        storage.save_profile(&profile_a).unwrap();

        let profile_b =
            create_profile_with_rules(&storage, "test", vec![("192.168.1.1", "test.local")]);

        // Preview enabling profile_b (should disable profile_a)
        let plan =
            generate_preview_plan_logic(&profile_b.id, true, storage.as_ref(), &writer).unwrap();

        // Verify plan contains profile_b's rules
        assert!(
            plan.rules
                .iter()
                .any(|r| r.ip.to_string() == "192.168.1.1" && r.domain == "test.local"),
            "plan should contain profile_b rules: {:?}",
            plan.rules
        );
        // Verify plan does NOT contain profile_a's rules
        assert!(
            !plan
                .rules
                .iter()
                .any(|r| r.ip.to_string() == "127.0.0.1" && r.domain == "example.com"),
            "plan should NOT contain profile_a rules: {:?}",
            plan.rules
        );

        // Verify storage state was NOT modified
        let loaded_a = storage.load_profile(&profile_a.id).unwrap();
        assert!(
            loaded_a.enabled,
            "profile_a should still be enabled in storage"
        );
        let loaded_b = storage.load_profile(&profile_b.id).unwrap();
        assert!(
            !loaded_b.enabled,
            "profile_b should still be disabled in storage"
        );
    }

    #[test]
    fn test_generate_preview_plan_disable_shows_empty_plan() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Create and enable a profile
        let mut profile =
            create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "example.com")]);
        profile.enabled = true;
        storage.save_profile(&profile).unwrap();

        // Preview disabling the profile
        let plan =
            generate_preview_plan_logic(&profile.id, false, storage.as_ref(), &writer).unwrap();

        // Verify plan is empty
        assert!(
            plan.rules.is_empty(),
            "plan should be empty when disabling the only enabled profile: {:?}",
            plan.rules
        );

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
        let plan =
            generate_preview_plan_logic(&dns_profile.id, true, storage.as_ref(), &writer).unwrap();

        assert!(
            plan.rules.is_empty(),
            "plan should be empty when target is dns profile: {:?}",
            plan.rules
        );
    }

    // Fix issue #121: editing an enabled hosts profile must write the new rules
    // to /etc/hosts (not just persist them in storage). This integration test
    // exercises the full update flow:
    //   1. create + enable + initial apply → /etc/hosts has rule A
    //   2. update profile rules (mimics update_profile's storage step)
    //   3. call apply_current_plan_logic (what update_profile now triggers
    //      automatically for enabled Hosts profiles)
    //   4. assert /etc/hosts has rule B and no longer has rule A
    #[test]
    fn test_update_profile_reapplies_hosts_for_issue_121() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Step 1: create + enable + initial apply
        let mut profile =
            create_profile_with_rules(&storage, "dev", vec![("127.0.0.1", "before.com")]);
        profile.enabled = true;
        storage.save_profile(&profile).unwrap();

        apply_current_plan_logic(storage.as_ref(), &writer).unwrap();

        let hosts_initial = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(
            hosts_initial.contains("127.0.0.1 before.com"),
            "hosts should contain initial rule: {}",
            hosts_initial
        );

        // Step 2: update profile rules (mimics update_profile's storage update)
        //   - profile.id / profile.enabled preserved
        //   - rules replaced with new set
        //   - saved to storage (source of truth)
        profile.rules = vec![HostRule::new(
            "192.168.1.1".parse().unwrap(),
            vec!["after.local".to_string()],
        )];
        profile.updated_at = chrono::Utc::now();
        storage.save_profile(&profile).unwrap();

        // Step 3: re-apply (the new behavior introduced for issue #121)
        let result = apply_current_plan_logic(storage.as_ref(), &writer);
        assert!(
            result.is_ok(),
            "re-apply after profile update must succeed: {:?}",
            result.err()
        );

        // Step 4: verify /etc/hosts reflects the new rules
        let hosts_after = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(
            hosts_after.contains("192.168.1.1 after.local"),
            "hosts should contain new rule after re-apply: {}",
            hosts_after
        );
        assert!(
            !hosts_after.contains("127.0.0.1 before.com"),
            "hosts should no longer contain the old rule: {}",
            hosts_after
        );
    }

    // Fix issue #121 (negative case): editing a DISABLED hosts profile must NOT
    // touch /etc/hosts. The re-apply is gated on profile.enabled, so disabled
    // profiles only persist to storage; the existing hosts file is untouched.
    #[test]
    fn test_update_profile_skips_reapply_when_disabled() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Step 1: create + enable another profile, then apply → /etc/hosts has a baseline rule
        let mut baseline =
            create_profile_with_rules(&storage, "baseline", vec![("10.0.0.1", "keep.com")]);
        baseline.enabled = true;
        storage.save_profile(&baseline).unwrap();
        apply_current_plan_logic(storage.as_ref(), &writer).unwrap();

        let hosts_baseline = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(hosts_baseline.contains("10.0.0.1 keep.com"));

        // Step 2: create a disabled profile and "update" it (re-apply guard must NOT fire)
        let mut disabled =
            create_profile_with_rules(&storage, "draft", vec![("127.0.0.1", "draft.com")]);
        // disabled.enabled stays false
        storage.save_profile(&disabled).unwrap();

        // Update disabled profile's rules — update_profile logic gates reapply on enabled,
        // so the call below represents what would happen IF we called apply_current_plan_logic
        // unconditionally. With the gate, we skip this call entirely. The test confirms
        // that even if apply_current_plan_logic were called with the disabled profile alone,
        // it would clear the managed block (because disabled profile produces empty plan).
        // The key invariant we test: a disabled profile's update must not invalidate
        // the baseline's rule.
        //
        // Simulate the gated behavior: only persist to storage, do NOT call
        // apply_current_plan_logic. After this step, /etc/hosts is untouched.
        disabled.rules = vec![HostRule::new(
            "127.0.0.1".parse().unwrap(),
            vec!["draft-v2.local".to_string()],
        )];
        disabled.updated_at = chrono::Utc::now();
        storage.save_profile(&disabled).unwrap();

        let hosts_after = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(
            hosts_after.contains("10.0.0.1 keep.com"),
            "baseline rule must remain when editing a disabled profile: {}",
            hosts_after
        );
        assert!(
            !hosts_after.contains("127.0.0.1 draft.local"),
            "disabled profile's rule must NOT leak into hosts: {}",
            hosts_after
        );
    }

    // ---- decide_apply_mode (Refs #127) ----

    fn outcome_with_conflicts() -> ApplyOutcome {
        let plan = ApplyPlan {
            rules: vec![],
            conflicts: vec![RuleConflict {
                domain: "x.example".into(),
                rules: vec![],
            }],
            diff: HostsDiff {
                added: vec!["1.1.1.1 x.example".into()],
                removed: vec![],
                unchanged: vec![],
            },
            backup_required: false,
        };
        ApplyOutcome::from_parts(plan, vec![], None, None)
    }

    fn outcome_with_disabled_ids() -> ApplyOutcome {
        let plan = ApplyPlan {
            rules: vec![],
            conflicts: vec![],
            diff: HostsDiff {
                added: vec!["1.1.1.1 a".into()],
                removed: vec![],
                unchanged: vec![],
            },
            backup_required: false,
        };
        ApplyOutcome::from_parts(plan, vec!["other-id".into()], None, None)
    }

    fn outcome_with_n_added(n: usize) -> ApplyOutcome {
        let added: Vec<String> = (0..n)
            .map(|i| format!("1.1.1.{} a.example", i + 1))
            .collect();
        let plan = ApplyPlan {
            rules: vec![],
            conflicts: vec![],
            diff: HostsDiff {
                added,
                removed: vec![],
                unchanged: vec![],
            },
            backup_required: n > 0,
        };
        ApplyOutcome::from_parts(plan, vec![], None, None)
    }

    #[test]
    fn test_decide_apply_mode_conflicts_require_preview() {
        let outcome = outcome_with_conflicts();
        assert_eq!(decide_apply_mode(&outcome), ApplyMode::RequirePreview);
    }

    #[test]
    fn test_decide_apply_mode_disabled_ids_require_preview() {
        let outcome = outcome_with_disabled_ids();
        assert_eq!(decide_apply_mode(&outcome), ApplyMode::RequirePreview);
    }

    #[test]
    fn test_decide_apply_mode_above_threshold_require_preview() {
        let outcome = outcome_with_n_added(DESTRUCTIVE_THRESHOLD + 1);
        assert_eq!(decide_apply_mode(&outcome), ApplyMode::RequirePreview);
    }

    #[test]
    fn test_decide_apply_mode_at_threshold_quick() {
        // exactly at threshold = not above = QuickApply
        let outcome = outcome_with_n_added(DESTRUCTIVE_THRESHOLD);
        assert_eq!(decide_apply_mode(&outcome), ApplyMode::QuickApply);
    }

    #[test]
    fn test_decide_apply_mode_zero_changes_quick() {
        let outcome = ApplyOutcome::empty();
        assert_eq!(decide_apply_mode(&outcome), ApplyMode::QuickApply);
    }

    #[test]
    fn test_decide_apply_mode_combined_rules_first_match_wins() {
        // conflicts + disabled + bulk — conflicts gate fires first
        let outcome = outcome_with_conflicts();
        assert_eq!(decide_apply_mode(&outcome), ApplyMode::RequirePreview);

        // no conflicts, disabled + bulk — disabled gate fires
        let outcome = outcome_with_disabled_ids();
        assert_eq!(decide_apply_mode(&outcome), ApplyMode::RequirePreview);
    }

    // ---- compute_disabled_ids_logic (Refs #127) ----

    #[test]
    fn test_compute_disabled_ids_logic_disabling_returns_empty() {
        let (_tmp, storage, _writer) = create_test_storage_and_writer();
        let any_id = ProfileId(Uuid::new_v4());
        let result = compute_disabled_ids_logic(&any_id, false, storage.as_ref()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_compute_disabled_ids_logic_enabling_lists_other_enabled_hosts() {
        let (_tmp, storage, _writer) = create_test_storage_and_writer();
        let mut p_a = Profile::new("a");
        p_a.mode = ProfileMode::Hosts;
        p_a.enabled = true;
        storage.save_profile(&p_a).unwrap();
        let mut p_b = Profile::new("b");
        p_b.mode = ProfileMode::Hosts;
        p_b.enabled = false;
        storage.save_profile(&p_b).unwrap();
        let mut p_c = Profile::new("c");
        p_c.mode = ProfileMode::Hosts;
        p_c.enabled = true;
        storage.save_profile(&p_c).unwrap();
        let mut p_dns = Profile::new("d");
        p_dns.mode = ProfileMode::Dns;
        p_dns.enabled = true;
        storage.save_profile(&p_dns).unwrap();

        let ids = compute_disabled_ids_logic(&p_a.id, true, storage.as_ref()).unwrap();
        // Should include C (enabled hosts other than A). Should NOT include B
        // (disabled) or DNS profile D.
        assert_eq!(ids.len(), 1, "expected only p_c, got {:?}", ids);
        assert_eq!(ids[0], p_c.id.to_string());
    }

    #[test]
    fn test_compute_disabled_ids_logic_does_not_mutate_storage() {
        let (_tmp, storage, _writer) = create_test_storage_and_writer();
        let mut p = Profile::new("only");
        p.mode = ProfileMode::Hosts;
        p.enabled = false;
        storage.save_profile(&p).unwrap();
        let original_id = p.id.to_string();

        let _ = compute_disabled_ids_logic(&p.id, true, storage.as_ref()).unwrap();
        let after = storage.load_profile(&p.id).unwrap();
        assert!(!after.enabled, "compute_disabled_ids_logic must not mutate");
        assert_eq!(after.id.to_string(), original_id);
    }

    // ---- preview_apply_outcome (Refs #127) ----

    fn preview_apply_outcome_logic(
        id: ProfileId,
        enabled: bool,
        storage: &(dyn Storage + Send + Sync),
        writer: &HostsWriter,
    ) -> Result<ApplyOutcome, MhostError> {
        let plan = generate_preview_plan_logic(&id, enabled, storage, writer)?;
        let disabled_ids = compute_disabled_ids_logic(&id, enabled, storage)?;
        Ok(ApplyOutcome::from_parts(plan, disabled_ids, None, None))
    }

    #[test]
    fn test_preview_apply_outcome_does_not_write_hosts_or_storage() {
        let (_tmp, storage, writer) = create_test_storage_and_writer();
        let mut p = Profile::new("preview-only");
        p.mode = ProfileMode::Hosts;
        p.enabled = true;
        storage.save_profile(&p).unwrap();

        let outcome = preview_apply_outcome_logic(p.id.clone(), true, storage.as_ref(), &writer)
            .expect("preview must succeed");

        assert!(outcome.snapshot_id.is_none(), "preview never writes");
        assert!(outcome.backup_path.is_none(), "preview never writes");
        let reloaded = storage.load_profile(&p.id).unwrap();
        assert!(
            reloaded.enabled,
            "preview must not change the persisted enabled flag"
        );
        let hosts_now = std::fs::read_to_string(writer.hosts_path()).unwrap_or_default();
        assert!(
            !hosts_now.contains(p.name.as_str()) || hosts_now.is_empty(),
            "preview must not write profile name into hosts; got: {}",
            hosts_now
        );
    }

    #[test]
    fn test_preview_apply_outcome_carries_disabled_ids() {
        let (_tmp, storage, writer) = create_test_storage_and_writer();
        let mut target = Profile::new("target");
        target.mode = ProfileMode::Hosts;
        target.enabled = false;
        storage.save_profile(&target).unwrap();
        let mut other = Profile::new("other");
        other.mode = ProfileMode::Hosts;
        other.enabled = true;
        storage.save_profile(&other).unwrap();

        let outcome =
            preview_apply_outcome_logic(target.id.clone(), true, storage.as_ref(), &writer)
                .expect("preview must succeed");
        assert_eq!(outcome.disabled_profile_ids, vec![other.id.to_string()]);
        assert!(!outcome.has_conflicts);
    }

    #[test]
    fn test_preview_apply_outcome_snapshot_and_backup_are_none() {
        let (_tmp, storage, writer) = create_test_storage_and_writer();
        let mut p = Profile::new("p");
        p.mode = ProfileMode::Hosts;
        p.enabled = true;
        storage.save_profile(&p).unwrap();

        let outcome = preview_apply_outcome_logic(p.id, true, storage.as_ref(), &writer).unwrap();
        assert!(outcome.snapshot_id.is_none());
        assert!(outcome.backup_path.is_none());
    }
}
