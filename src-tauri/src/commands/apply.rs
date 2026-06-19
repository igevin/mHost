use mhost_apply::{ApplyPlan, generate_plan};
use mhost_core::MhostError;
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub fn generate_apply_plan(state: State<'_, AppState>) -> Result<ApplyPlan, MhostError> {
    let profiles = state.storage.list_profiles()?;
    let current_hosts = std::fs::read_to_string("/etc/hosts")
        .unwrap_or_default();
    generate_plan(&profiles, &current_hosts).map_err(Into::into)
}

#[tauri::command]
pub fn apply_hosts(plan: ApplyPlan, state: State<'_, AppState>) -> Result<(), MhostError> {
    state.writer.apply(&plan).map_err(Into::into)
}

#[tauri::command]
pub fn rollback_hosts(state: State<'_, AppState>) -> Result<(), MhostError> {
    state.writer.rollback().map_err(Into::into)
}

#[tauri::command]
pub fn read_system_hosts() -> Result<String, MhostError> {
    std::fs::read_to_string("/etc/hosts")
        .map_err(|e| MhostError::Io {
            kind: e.kind().to_string(),
            message: e.to_string(),
        })
}
