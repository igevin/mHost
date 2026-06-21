use std::str::FromStr;

use mhost_core::{MhostError, Profile, ProfileId};
use mhost_hosts::parser::Parser;
use mhost_storage::storage::Storage;
use tauri::{AppHandle, State};

use crate::state::AppState;

const MAX_NAME_LENGTH: usize = 200;
const MAX_DESCRIPTION_LENGTH: usize = 2000;

/// Validate profile fields before saving.
/// Security fix (#18): Prevents injection of control characters, excessive length, and invalid rules.
fn validate_profile(profile: &Profile) -> Result<(), MhostError> {
    // 1. Length limits
    if profile.name.len() > MAX_NAME_LENGTH {
        return Err(MhostError::InvalidInput(format!(
            "Profile name exceeds maximum length of {} characters",
            MAX_NAME_LENGTH
        )));
    }
    if profile
        .description
        .as_ref()
        .map_or(0, |s| s.len())
        > MAX_DESCRIPTION_LENGTH
    {
        return Err(MhostError::InvalidInput(format!(
            "Profile description exceeds maximum length of {} characters",
            MAX_DESCRIPTION_LENGTH
        )));
    }

    // 2. Reject control characters in name
    if profile.name.chars().any(|c| c.is_control()) {
        return Err(MhostError::InvalidInput(
            "Profile name contains control characters".to_string(),
        ));
    }

    // 3. Re-validate all rules through the parser
    for rule in &profile.rules {
        let line = format!("{} {}", rule.ip, rule.domains.join(" "));
        let result = Parser::parse(&line);
        if !result.errors.is_empty() {
            return Err(MhostError::InvalidInput(format!(
                "Invalid rule in profile: {} {}",
                rule.ip,
                rule.domains.join(" ")
            )));
        }
        // Reject control characters in comments (they would be written to /etc/hosts)
        if let Some(c) = &rule.comment {
            if c.chars().any(|ch| ch.is_control()) {
                return Err(MhostError::InvalidInput(format!(
                    "Rule comment contains control characters: {:?}",
                    c
                )));
            }
        }
    }

    // 4. Validate tags (reject control characters and excessive length)
    for tag in &profile.tags {
        if tag.chars().any(|c| c.is_control()) || tag.len() > 50 {
            return Err(MhostError::InvalidInput(format!(
                "Invalid tag: {:?}",
                tag
            )));
        }
    }

    Ok(())
}

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
    // Security fix (#18): Validate profile before saving
    validate_profile(&profile)?;
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
    // Security fix (#18): Validate profile before saving
    validate_profile(&profile)?;
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
