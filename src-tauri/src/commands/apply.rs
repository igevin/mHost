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
    let last_applied_path = state.storage.root().join("last_applied.json");
    let timestamp = chrono::Utc::now().to_rfc3339();
    let data = serde_json::json!({ "timestamp": timestamp });
    if let Err(e) = std::fs::write(
        &last_applied_path,
        serde_json::to_string_pretty(&data).unwrap(),
    ) {
        eprintln!(
            "[mHost] Warning: failed to write last_applied.json: {}",
            e
        );
    }

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
