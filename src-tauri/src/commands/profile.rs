use std::str::FromStr;

use mhost_core::{MhostError, Profile, ProfileId};
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub fn list_profiles(state: State<'_, AppState>) -> Result<Vec<Profile>, MhostError> {
    state.storage.list_profiles().map_err(Into::into)
}

#[tauri::command]
pub fn get_profile(id: String, state: State<'_, AppState>) -> Result<Profile, MhostError> {
    let profile_id = ProfileId::from_str(&id)?;
    state.storage.load_profile(&profile_id).map_err(Into::into)
}

#[tauri::command]
pub fn create_profile(name: String, state: State<'_, AppState>) -> Result<Profile, MhostError> {
    let profile = Profile::new(name);
    state.storage.save_profile(&profile)?;
    Ok(profile)
}

#[tauri::command]
pub fn update_profile(profile: Profile, state: State<'_, AppState>) -> Result<Profile, MhostError> {
    state.storage.save_profile(&profile)?;
    Ok(profile)
}

#[tauri::command]
pub fn delete_profile(id: String, state: State<'_, AppState>) -> Result<(), MhostError> {
    let profile_id = ProfileId::from_str(&id)?;
    state.storage.delete_profile(&profile_id).map_err(Into::into)
}

#[tauri::command]
pub fn set_profile_enabled(
    id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<Profile, MhostError> {
    let profile_id = ProfileId::from_str(&id)?;

    // 阶段 0：只允许一个 Profile 启用
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
    Ok(profile)
}
