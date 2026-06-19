use mhost_core::{MhostError, Profile};
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub fn list_profiles(_state: State<'_, AppState>) -> Result<Vec<Profile>, MhostError> {
    Ok(vec![])
}

#[tauri::command]
pub fn get_profile(_id: String, _state: State<'_, AppState>) -> Result<Profile, MhostError> {
    Err(MhostError::InvalidInput("not implemented".to_string()))
}

#[tauri::command]
pub fn create_profile(_name: String, _state: State<'_, AppState>) -> Result<Profile, MhostError> {
    Err(MhostError::InvalidInput("not implemented".to_string()))
}

#[tauri::command]
pub fn update_profile(
    _profile: Profile,
    _state: State<'_, AppState>,
) -> Result<Profile, MhostError> {
    Err(MhostError::InvalidInput("not implemented".to_string()))
}

#[tauri::command]
pub fn delete_profile(_id: String, _state: State<'_, AppState>) -> Result<(), MhostError> {
    Err(MhostError::InvalidInput("not implemented".to_string()))
}

#[tauri::command]
pub fn set_profile_enabled(
    _id: String,
    _enabled: bool,
    _state: State<'_, AppState>,
) -> Result<Profile, MhostError> {
    Err(MhostError::InvalidInput("not implemented".to_string()))
}
