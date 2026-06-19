use mhost_apply::ApplyPlan;
use mhost_core::MhostError;
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub fn generate_apply_plan(_state: State<'_, AppState>) -> Result<ApplyPlan, MhostError> {
    Err(MhostError::InvalidInput("not implemented".to_string()))
}

#[tauri::command]
pub fn apply_hosts(_state: State<'_, AppState>) -> Result<(), MhostError> {
    Err(MhostError::InvalidInput("not implemented".to_string()))
}

#[tauri::command]
pub fn rollback_hosts(_state: State<'_, AppState>) -> Result<(), MhostError> {
    Err(MhostError::InvalidInput("not implemented".to_string()))
}

#[tauri::command]
pub fn read_system_hosts() -> Result<String, MhostError> {
    Err(MhostError::InvalidInput("not implemented".to_string()))
}
