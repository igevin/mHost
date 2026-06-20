use std::str::FromStr;

use mhost_core::{MhostError, Profile, ProfileId};
use mhost_storage::storage::Storage;
use tauri::{AppHandle, State};

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
pub fn create_profile(
    name: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<Profile, MhostError> {
    let profile = Profile::new(name);
    state.storage.save_profile(&profile)?;
    crate::tray::update_tray_menu(&app_handle);
    Ok(profile)
}

#[tauri::command]
pub fn update_profile(
    profile: Profile,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<Profile, MhostError> {
    state.storage.save_profile(&profile)?;
    crate::tray::update_tray_menu(&app_handle);
    Ok(profile)
}

#[tauri::command]
pub fn delete_profile(
    id: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), MhostError> {
    let profile_id = ProfileId::from_str(&id)?;
    state.storage.delete_profile(&profile_id)?;
    crate::tray::update_tray_menu(&app_handle);
    Ok(())
}

/// Disable all profiles except the given one.
///
/// Phase 0 constraint: only one profile can be enabled at a time.
pub fn disable_other_profiles(
    storage: &(dyn Storage + Send + Sync),
    except_id: &ProfileId,
) -> Result<(), MhostError> {
    let all_profiles = storage.list_profiles()?;
    for mut p in all_profiles {
        if p.enabled && p.id != *except_id {
            p.enabled = false;
            storage.save_profile(&p)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn set_profile_enabled(
    id: String,
    enabled: bool,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<Profile, MhostError> {
    let profile_id = ProfileId::from_str(&id)?;

    // 阶段 0：只允许一个 Profile 启用
    if enabled {
        disable_other_profiles(state.storage.as_ref(), &profile_id)?;
    }

    let mut profile = state.storage.load_profile(&profile_id)?;
    profile.enabled = enabled;
    profile.updated_at = chrono::Utc::now();
    state.storage.save_profile(&profile)?;
    crate::tray::update_tray_menu(&app_handle);
    Ok(profile)
}
