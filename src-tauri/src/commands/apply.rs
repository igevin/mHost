use mhost_apply::{ApplyPlan, generate_plan};
use mhost_core::MhostError;
use tauri::State;

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
    // 1. Toggle enabled state in storage (same logic as set_profile_enabled)
    let profile_id = std::str::FromStr::from_str(&id)?;
    if enabled {
        let all_profiles = state.storage.list_profiles()?;
        for mut p in all_profiles {
            if p.enabled && p.id != profile_id {
                p.enabled = false;
                state.storage.save_profile(&p)?;
            }
        }
    }
    let mut profile = state.storage.load_profile(&profile_id)?;
    profile.enabled = enabled;
    profile.updated_at = chrono::Utc::now();
    state.storage.save_profile(&profile)?;

    // 2. Reload all profiles and generate plan
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
    let plan = generate_plan(&profiles, &current_hosts)?;

    // 3. Apply to system hosts
    state.writer.apply(&plan)?;

    // 4. Record timestamp
    write_last_applied(&state.storage.root())?;

    Ok(())
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
