use std::str::FromStr;

use mhost_core::{MhostError, Profile, ProfileId, ProfileMode};
use mhost_storage::storage::Storage;
use tauri::{AppHandle, State};

use crate::state::AppState;

// Security fix (#18): Prevent malicious profile data from being written to /etc/hosts.
// Validate profile names, description, and all rules.
pub fn validate_profile(profile: &Profile) -> Result<(), MhostError> {
    // Name: non-empty, max 255 chars, no newlines or nulls
    if profile.name.is_empty() {
        return Err(MhostError::InvalidInput(
            "Profile name cannot be empty".to_string(),
        ));
    }
    if profile.name.len() > 255 {
        return Err(MhostError::InvalidInput(
            "Profile name exceeds 255 characters".to_string(),
        ));
    }
    if profile.name.contains('\n') || profile.name.contains('\0') {
        return Err(MhostError::InvalidInput(
            "Profile name contains invalid characters".to_string(),
        ));
    }

    // Description: optional, max 4096 chars, no nulls
    if let Some(desc) = &profile.description {
        if desc.len() > 4096 {
            return Err(MhostError::InvalidInput(
                "Profile description exceeds 4096 characters".to_string(),
            ));
        }
        if desc.contains('\0') {
            return Err(MhostError::InvalidInput(
                "Profile description contains invalid characters".to_string(),
            ));
        }
    }

    // Validate each rule entry
    for rule in &profile.rules {
        // IP must be present and valid (already IpAddr by type)
        if rule.ip.is_none() {
            return Err(MhostError::InvalidInput(format!(
                "Missing IP address in profile '{}'",
                profile.name
            )));
        }

        // Domains must not be empty
        if rule.domains.is_empty() {
            return Err(MhostError::InvalidInput(format!(
                "Empty domains list in profile '{}'",
                profile.name
            )));
        }
        for domain in &rule.domains {
            if domain.is_empty() {
                return Err(MhostError::InvalidInput(format!(
                    "Empty domain in profile '{}'",
                    profile.name
                )));
            }
            if domain
                .chars()
                .any(|c| c.is_whitespace() || c == '\n' || c == '\0')
            {
                return Err(MhostError::InvalidInput(format!(
                    "Invalid domain '{}' in profile '{}'",
                    domain, profile.name
                )));
            }
        }
    }

    Ok(())
}

/// Create a new profile with auto-generated ID.
///
/// Calls `profile.save()` which writes:
/// - `profiles/{id}.json`
/// - Updates `manifest.json`
///
/// Security fix (#18): Validates profile before saving.
#[tauri::command]
pub fn create_profile(
    name: String,
    mode: Option<ProfileMode>,
    state: State<'_, AppState>,
    #[allow(unused_variables)] app_handle: AppHandle,
) -> Result<Profile, MhostError> {
    let mut profile = Profile::new(name);
    if let Some(m) = mode {
        profile.mode = m;
    }
    // Security fix (#18): Validate profile before saving
    validate_profile(&profile)?;
    state.storage.save_profile(&profile)?;
    #[cfg(target_os = "macos")]
    crate::tray::update_tray_menu(&app_handle);
    Ok(profile)
}

/// Get a single profile by ID.
///
/// Returns `None` if not found.
#[tauri::command]
pub fn get_profile(id: String, state: State<'_, AppState>) -> Result<Option<Profile>, MhostError> {
    let profile_id = ProfileId::from_str(&id)?;
    match state.storage.load_profile(&profile_id) {
        Ok(profile) => Ok(Some(profile)),
        Err(mhost_core::StorageError::ProfileNotFound(_)) => Ok(None),
        Err(e) => Err(MhostError::from(e)),
    }
}

/// List all profiles in storage.
///
/// Reads `manifest.json` to get profile IDs, then loads each profile.
/// Perf fix (#30): Uses `list_profiles` instead of individual `load_profile` calls.
#[tauri::command]
pub fn list_profiles(
    mode: Option<ProfileMode>,
    state: State<'_, AppState>,
) -> Result<Vec<Profile>, MhostError> {
    match mode {
        Some(m) => state.storage.list_profiles_by_mode(m).map_err(Into::into),
        None => state.storage.list_profiles().map_err(Into::into),
    }
}

/// 列出 DNS 模式 Profile（快捷命令）。
#[tauri::command]
pub fn list_dns_profiles(state: State<'_, AppState>) -> Result<Vec<Profile>, MhostError> {
    state
        .storage
        .list_profiles_by_mode(ProfileMode::Dns)
        .map_err(Into::into)
}

/// Disable all hosts-mode profiles except the given one.
///
/// Phase 0 constraint: only one hosts profile can be enabled at a time.
/// DNS mode profiles are not affected (multi-activation is allowed).
/// Perf fix (#31): Collect profiles to disable first to avoid unnecessary iterations.
pub fn disable_other_profiles(
    storage: &(dyn Storage + Send + Sync),
    except_id: &ProfileId,
) -> Result<(), MhostError> {
    let all_profiles = storage.list_profiles()?;
    let to_disable: Vec<_> = all_profiles
        .into_iter()
        .filter(|p| p.enabled && p.id != *except_id)
        .collect();

    // TODO(#31): Each save_profile is an atomic write. Consider batching at the storage level
    // (e.g., a single manifest file) to reduce disk I/O when many profiles need disabling.
    for mut p in to_disable {
        p.enabled = false;
        storage.save_profile(&p)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_profile_enabled(
    id: String,
    enabled: bool,
    state: State<'_, AppState>,
    #[allow(unused_variables)] app_handle: AppHandle,
) -> Result<Profile, MhostError> {
    let profile_id = ProfileId::from_str(&id)?;

    let mut profile = state.storage.load_profile(&profile_id)?;

    // Hosts 模式保持互斥；DNS 模式允许多激活
    if profile.mode == ProfileMode::Hosts && enabled {
        disable_other_profiles(state.storage.as_ref(), &profile_id)?;
    }

    profile.enabled = enabled;
    profile.updated_at = chrono::Utc::now();
    state.storage.save_profile(&profile)?;

    // 如果是 DNS 模式 Profile 且 dns_enabled == true，热重载规则。
    // **fix issue #67 round 3 (Bug B)**: 不再要求 `enabled == true` —— 禁用
    // DNS profile 也要 reload，否则 in-memory RuleEngine 仍保留禁用 profile
    // 的规则，用户只能通过 DNS 模式 off→on 来真正清空（用户体验：禁用所
    // 有 profile 之后第一条 profile 的规则仍然生效）。Reload 是 in-memory
    // rebuild，廉价，无副作用（dns_enabled 关掉时也走这个 guard 不重载）。
    if profile.mode == ProfileMode::Dns
        && state.dns_enabled.load(std::sync::atomic::Ordering::Relaxed)
    {
        crate::commands::dns::reload_dns_rules(state).await?;
    }

    #[cfg(target_os = "macos")]
    crate::tray::update_tray_menu(&app_handle);
    Ok(profile)
}

/// Update an existing profile's name, description, and rules.
///
/// Replaces the entire profile file atomically.
/// Security fix (#18): Validates updated profile before saving.
///
/// **fix: DNS 规则热重载** —— 之前实现只 save_profile，不通知运行中的
/// DnsServer 重新加载规则集，导致用户编辑一个 enabled 的 DNS profile
/// 后新规则不生效。修复：保存后如果 profile 是 enabled DNS profile
/// 且 DNS 模式在跑，调 `reload_dns_rules`。
#[tauri::command]
pub async fn update_profile(
    id: String,
    name: String,
    description: Option<String>,
    rules: Vec<mhost_core::HostRule>,
    // **fix issue #67 bug 2**: 显式接受 mode。前端 create 时如果 Tauri
    // 反序列化 Option<ProfileMode> 漏掉（Hypothesis A），profile 会以
    // 默认 mode=Hosts 落盘到 profiles/hosts/，后续 set_profile_enabled
    // 的 reload 条件 `mode == Dns` 永远不满足 → DNS 规则永不生效。
    // 每次 update 显式 reassert mode，纠正任何磁盘上错误的 mode。
    mode: Option<ProfileMode>,
    state: State<'_, AppState>,
    #[allow(unused_variables)] app_handle: AppHandle,
) -> Result<Profile, MhostError> {
    let profile_id = ProfileId::from_str(&id)?;
    let mut profile = state.storage.load_profile(&profile_id)?;

    profile.name = name;
    profile.description = description;
    profile.rules = rules;
    if let Some(m) = mode {
        profile.mode = m;
    }
    profile.updated_at = chrono::Utc::now();

    // N4: Validate profile data before applying changes to system hosts.
    validate_profile(&profile)?;
    state.storage.save_profile(&profile)?;

    // 如果这是一个 enabled 的 DNS 模式 Profile 且 DNS 模式在跑，
    // 把新规则热加载到运行中的 DnsServer.RuleEngine。
    if profile.mode == ProfileMode::Dns
        && profile.enabled
        && state.dns_enabled.load(std::sync::atomic::Ordering::Relaxed)
    {
        if let Err(e) = crate::commands::dns::reload_dns_rules(state).await {
            // 规则已存盘，下次 set_profile_enabled(true) 也会触发 reload，
            // 所以这里不向用户抛 Err —— storage 是 source of truth，DNS
            // server reload 是 best-effort。
            eprintln!(
                "[mHost] DNS rule hot-reload failed after update_profile: {}",
                e
            );
        }
    }

    #[cfg(target_os = "macos")]
    crate::tray::update_tray_menu(&app_handle);
    Ok(profile)
}

/// Delete a profile and remove it from the manifest.
///
/// Calls `profile.delete()` which:
/// - Removes `profiles/{id}.json`
/// - Updates `manifest.json`
#[tauri::command]
pub fn delete_profile(
    id: String,
    state: State<'_, AppState>,
    #[allow(unused_variables)] app_handle: AppHandle,
) -> Result<(), MhostError> {
    let profile_id = ProfileId::from_str(&id)?;
    state.storage.delete_profile(&profile_id)?;
    #[cfg(target_os = "macos")]
    crate::tray::update_tray_menu(&app_handle);
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mhost_storage::storage::{FileStorage, Storage};
    use std::sync::Arc;

    fn create_test_storage() -> (tempfile::TempDir, Arc<dyn Storage + Send + Sync>) {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(FileStorage::new(temp_dir.path())) as Arc<dyn Storage + Send + Sync>;
        (temp_dir, storage)
    }

    fn create_profile(
        storage: &Arc<dyn Storage + Send + Sync>,
        name: &str,
        mode: ProfileMode,
    ) -> Profile {
        let mut profile = Profile::new(name.to_string());
        profile.mode = mode;
        storage.save_profile(&profile).unwrap();
        profile
    }

    #[test]
    fn test_disable_other_profiles_only_affects_hosts_mode() {
        let (_temp, storage) = create_test_storage();

        // Create two hosts profiles, enable both
        let mut hosts_a = create_profile(&storage, "hosts_a", ProfileMode::Hosts);
        hosts_a.enabled = true;
        storage.save_profile(&hosts_a).unwrap();

        let mut hosts_b = create_profile(&storage, "hosts_b", ProfileMode::Hosts);
        hosts_b.enabled = true;
        storage.save_profile(&hosts_b).unwrap();

        // Create a DNS profile, enable it
        let mut dns_a = create_profile(&storage, "dns_a", ProfileMode::Dns);
        dns_a.enabled = true;
        storage.save_profile(&dns_a).unwrap();

        // Disable others for hosts_a
        disable_other_profiles(storage.as_ref(), &hosts_a.id).unwrap();

        // hosts_a stays enabled, hosts_b disabled, dns_a stays enabled
        let hosts_a_loaded = storage.load_profile(&hosts_a.id).unwrap();
        let hosts_b_loaded = storage.load_profile(&hosts_b.id).unwrap();
        let dns_a_loaded = storage.load_profile(&dns_a.id).unwrap();

        assert!(hosts_a_loaded.enabled, "hosts_a should remain enabled");
        assert!(!hosts_b_loaded.enabled, "hosts_b should be disabled");
        assert!(
            dns_a_loaded.enabled,
            "dns_a should remain enabled (not affected by hosts-mode mutual exclusion)"
        );
    }

    #[test]
    fn test_list_profiles_by_mode() {
        let (_temp, storage) = create_test_storage();

        let hosts_profile = create_profile(&storage, "hosts_dev", ProfileMode::Hosts);
        let dns_profile = create_profile(&storage, "dns_dev", ProfileMode::Dns);

        // list_profiles (default hosts mode) should only return hosts profile
        let default_list = storage.list_profiles().unwrap();
        assert_eq!(default_list.len(), 1);
        assert_eq!(default_list[0].id, hosts_profile.id);

        // list_profiles_by_mode(Hosts) should return hosts profile
        let hosts_list = storage.list_profiles_by_mode(ProfileMode::Hosts).unwrap();
        assert_eq!(hosts_list.len(), 1);
        assert_eq!(hosts_list[0].id, hosts_profile.id);

        // list_profiles_by_mode(Dns) should return dns profile
        let dns_list = storage.list_profiles_by_mode(ProfileMode::Dns).unwrap();
        assert_eq!(dns_list.len(), 1);
        assert_eq!(dns_list[0].id, dns_profile.id);

        // list_all_profiles should return both
        let all_list = storage.list_all_profiles().unwrap();
        assert_eq!(all_list.len(), 2);
    }
}
