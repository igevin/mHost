use std::str::FromStr;

use chrono::Utc;
use mhost_core::{ExportFormat, MhostError, Profile, ProfileId};
use mhost_hosts::formatter::format_rules;
use mhost_hosts::parser::Parser;
use mhost_hosts::validator;
use mhost_storage::storage::Storage;
use tauri::State;

use crate::state::AppState;

const MAX_FILE_READ_SIZE: usize = 1_048_576; // 1MB

/// Validate that a file path stays within the user's home directory.
/// For existing files, canonicalizes the full path. For new files (export),
/// validates the parent directory instead.
/// Security fix (#17): Prevents arbitrary file read/write via IPC.
fn validate_user_path(path: &str, must_exist: bool) -> Result<std::path::PathBuf, MhostError> {
    let p = std::path::Path::new(path);
    let home = dirs::home_dir()
        .ok_or_else(|| MhostError::InvalidInput("Cannot determine home directory".to_string()))?;

    if must_exist {
        // Import: file must exist, canonicalize full path
        let canonical = p
            .canonicalize()
            .map_err(|e| MhostError::InvalidInput(format!("Invalid path '{}': {}", path, e)))?;
        if !canonical.starts_with(&home) {
            return Err(MhostError::InvalidInput(format!(
                "Path '{}' is outside home directory",
                path
            )));
        }
        Ok(canonical)
    } else {
        // Export: file may not exist yet, validate parent directory.
        // Security note: canonicalize parent, then reconstruct the full path
        // to avoid TOCTOU where the parent is replaced by a symlink after check.
        let parent = p.parent().ok_or_else(|| {
            MhostError::InvalidInput(format!("Path '{}' has no parent directory", path))
        })?;
        let canonical_parent = parent.canonicalize().map_err(|e| {
            MhostError::InvalidInput(format!("Invalid parent path '{}': {}", path, e))
        })?;
        if !canonical_parent.starts_with(&home) {
            return Err(MhostError::InvalidInput(format!(
                "Path '{}' is outside home directory",
                path
            )));
        }
        // Reconstruct path using canonicalized parent + original filename
        let file_name = p
            .file_name()
            .ok_or_else(|| MhostError::InvalidInput(format!("Path '{}' has no file name", path)))?;
        Ok(canonical_parent.join(file_name))
    }
}

// ---------------------------------------------------------------------------
// Core logic (testable without Tauri State)
// ---------------------------------------------------------------------------

/// Import a profile from hosts text.
pub fn import_profile_logic(
    name: String,
    hosts_text: &str,
    storage: &dyn Storage,
) -> Result<Profile, MhostError> {
    if name.trim().is_empty() {
        return Err(MhostError::InvalidInput(
            "Profile name must not be empty".to_string(),
        ));
    }

    let parse_result = Parser::parse(hosts_text);
    if !parse_result.errors.is_empty() {
        return Err(MhostError::InvalidInput(format!(
            "Hosts text contains {} parse error(s)",
            parse_result.errors.len()
        )));
    }

    let final_name = resolve_name_conflict(&name, storage)?;

    let mut profile = Profile::new(&final_name);
    profile.rules = parse_result.rules;
    storage.save_profile(&profile)?;
    Ok(profile)
}

/// Export a profile as the specified format.
pub fn export_profile_logic(
    id: &str,
    format: ExportFormat,
    storage: &dyn Storage,
) -> Result<String, MhostError> {
    let profile_id = ProfileId::from_str(id)?;
    let profile = storage.load_profile(&profile_id)?;

    match format {
        ExportFormat::Hosts => Ok(format_rules(&profile.rules)),
        ExportFormat::Json => serde_json::to_string_pretty(&profile)
            .map_err(|e| MhostError::InvalidInput(format!("Failed to serialize profile: {}", e))),
    }
}

/// Duplicate a profile with a new name.
///
/// **fix issue #67 round 4 (review follow-up)**: 必须保留 source 的
/// `mode`。`Profile::new` 默认 `ProfileMode::Hosts`，原本会让所有
/// 复制的 profile 都变成 hosts 模式 —— 这在「DNS profile 列表加
/// Duplicate 按钮」（issue #67）后变成 user-visible bug：复制 DNS
/// profile 后丢到 hosts 目录，下次 `set_profile_enabled` 重载时
/// `mode == Dns` 条件 false，规则永远不生效。
pub fn duplicate_profile_logic(
    id: &str,
    new_name: String,
    storage: &dyn Storage,
) -> Result<Profile, MhostError> {
    if new_name.trim().is_empty() {
        return Err(MhostError::InvalidInput(
            "Profile name must not be empty".to_string(),
        ));
    }

    let profile_id = ProfileId::from_str(id)?;
    let source = storage.load_profile(&profile_id)?;

    let final_name = resolve_name_conflict(&new_name, storage)?;

    let mut dup = Profile::new(&final_name);
    dup.mode = source.mode; // fix: 保留 source 的 mode（Hosts 或 Dns）
    dup.rules = source.rules;
    dup.description = source.description.clone();
    dup.tags = source.tags.clone();
    dup.protected = false;
    dup.enabled = false;
    storage.save_profile(&dup)?;
    Ok(dup)
}

/// Resolve name conflicts by appending " (2)", " (3)", etc.
fn resolve_name_conflict(name: &str, storage: &dyn Storage) -> Result<String, MhostError> {
    let existing = storage.list_profiles()?;
    let existing_names: Vec<&str> = existing.iter().map(|p| p.name.as_str()).collect();

    if !existing_names.contains(&name) {
        return Ok(name.to_string());
    }

    let mut counter = 2;
    const MAX_ITERATIONS: usize = 100;
    while counter <= MAX_ITERATIONS {
        let candidate = format!("{} ({})", name, counter);
        if !existing_names.contains(&candidate.as_str()) {
            return Ok(candidate);
        }
        counter += 1;
    }

    Err(MhostError::InvalidInput(format!(
        "Could not resolve name conflict for '{}' after {} attempts",
        name, MAX_ITERATIONS
    )))
}

// ---------------------------------------------------------------------------
// Tauri commands (thin wrappers)
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn import_profile(
    name: String,
    hosts_text: String,
    state: State<'_, AppState>,
) -> Result<Profile, MhostError> {
    import_profile_logic(name, &hosts_text, state.storage.as_ref())
}

#[tauri::command]
pub fn export_profile(
    id: String,
    format: ExportFormat,
    state: State<'_, AppState>,
) -> Result<String, MhostError> {
    export_profile_logic(&id, format, state.storage.as_ref())
}

#[tauri::command]
pub fn duplicate_profile(
    id: String,
    new_name: String,
    state: State<'_, AppState>,
) -> Result<Profile, MhostError> {
    duplicate_profile_logic(&id, new_name, state.storage.as_ref())
}

/// Export a profile to a file.
///
/// Security fix (#17): Path is validated to stay within home directory.
/// Perf fix (#26): Async with spawn_blocking for file I/O.
#[tauri::command]
pub async fn export_profile_to_file(
    id: String,
    format: ExportFormat,
    path: String,
    state: State<'_, AppState>,
) -> Result<(), MhostError> {
    let validated = validate_user_path(&path, false)?;
    let content = export_profile_logic(&id, format, state.storage.as_ref())?;
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::write(&validated, &content).map_err(Into::into)
    })
    .await
    .map_err(|e| MhostError::InvalidInput(e.to_string()))?
}

/// Import a profile from a file.
///
/// Supports two formats:
/// - `.json`: Deserializes the file as a Profile JSON, then re-saves with the given name.
/// - `.hosts` / `.txt` / other: Parses the file content as hosts text.
///
/// Security fix (#17): Path is validated to stay within home directory.
/// Reads file content (limited to 1MB) and imports it as a new profile.
/// Perf fix (#26): Async with spawn_blocking for file I/O.
#[tauri::command]
pub async fn import_profile_from_file(
    name: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<Profile, MhostError> {
    let canonical = validate_user_path(&path, true)?;
    let storage = state.storage.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let metadata = std::fs::metadata(&canonical)?;
        if metadata.len() > MAX_FILE_READ_SIZE as u64 {
            return Err(MhostError::InvalidInput(format!(
                "File too large (max {} bytes)",
                MAX_FILE_READ_SIZE
            )));
        }
        let content = std::fs::read_to_string(&canonical)?;

        // Detect format by file extension (case-insensitive)
        let is_json = canonical
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
        if is_json {
            import_profile_from_json(name, &content, storage.as_ref())
        } else {
            import_profile_logic(name, &content, storage.as_ref())
        }
    })
    .await
    .map_err(|e| MhostError::InvalidInput(e.to_string()))?
}

/// Import a profile from JSON content.
/// Deserializes the JSON as a Profile, validates rules, then re-saves with the given name.
fn import_profile_from_json(
    name: String,
    json_str: &str,
    storage: &dyn Storage,
) -> Result<Profile, MhostError> {
    if name.trim().is_empty() {
        return Err(MhostError::InvalidInput(
            "Profile name must not be empty".to_string(),
        ));
    }

    let mut profile: Profile = serde_json::from_str(json_str)
        .map_err(|e| MhostError::InvalidInput(format!("Invalid JSON profile: {}", e)))?;

    // Validate rules: check IP and domain formats
    for rule in &profile.rules {
        if let Some(ip) = &rule.ip {
            if ip.to_string().is_empty() {
                return Err(MhostError::InvalidInput(format!(
                    "Rule {} has an empty IP address",
                    rule.id
                )));
            }
        }
        for domain in &rule.domains {
            if !validator::is_valid_domain(domain) {
                return Err(MhostError::InvalidInput(format!(
                    "Rule {} has an invalid domain: {}",
                    rule.id, domain
                )));
            }
        }
    }

    let now = Utc::now();
    let final_name = resolve_name_conflict(&name, storage)?;
    profile.name = final_name;
    profile.id = ProfileId(uuid::Uuid::new_v4()); // Assign a new ID to avoid collisions
    profile.enabled = false;
    profile.protected = false;
    profile.created_at = now;
    profile.updated_at = now;
    storage.save_profile(&profile)?;
    Ok(profile)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mhost_core::HostRule;
    use mhost_storage::storage::FileStorage;
    use std::net::IpAddr;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // Helper
    // -----------------------------------------------------------------------

    fn create_test_storage() -> (TempDir, FileStorage) {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());
        (temp_dir, storage)
    }

    fn create_profile_with_rules(
        storage: &dyn Storage,
        name: &str,
        rules: Vec<(IpAddr, Vec<String>)>,
    ) -> Profile {
        let mut profile = Profile::new(name);
        for (ip, domains) in rules {
            profile.rules.push(HostRule::new(ip, domains));
        }
        storage.save_profile(&profile).unwrap();
        profile
    }

    // -----------------------------------------------------------------------
    // import_profile tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_import_valid_hosts() {
        let (_temp, storage) = create_test_storage();
        let cases = vec![
            ("simple", "127.0.0.1 example.com", 1),
            ("multiple", "127.0.0.1 a.com\n192.168.1.1 b.com", 2),
            ("with_comments", "# header\n127.0.0.1 x.com # inline", 2),
            ("empty", "", 0),
        ];

        for (name, hosts_text, expected_rules) in cases {
            let profile =
                import_profile_logic(format!("import_{}", name), hosts_text, &storage).unwrap();
            assert_eq!(
                profile.rules.len(),
                expected_rules,
                "case: {} — expected {} rules",
                name,
                expected_rules
            );
            assert!(
                !profile.enabled,
                "case: {} — imported profile should be disabled",
                name
            );
        }
    }

    #[test]
    fn test_import_rejects_invalid() {
        let (_temp, storage) = create_test_storage();

        // Invalid IP
        let result = import_profile_logic("bad".to_string(), "999.999.999.999 x.com", &storage);
        assert!(result.is_err(), "invalid IP should be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("parse error"),
            "error should mention parse errors: {}",
            msg
        );

        // Invalid domain
        let result = import_profile_logic("bad2".to_string(), "127.0.0.1 -bad.com", &storage);
        assert!(result.is_err(), "invalid domain should be rejected");
    }

    #[test]
    fn test_import_empty_name() {
        let (_temp, storage) = create_test_storage();

        let result = import_profile_logic("".to_string(), "127.0.0.1 a.com", &storage);
        assert!(result.is_err(), "empty name should be rejected");

        let result = import_profile_logic("   ".to_string(), "127.0.0.1 a.com", &storage);
        assert!(result.is_err(), "whitespace-only name should be rejected");
    }

    #[test]
    fn test_import_name_conflict() {
        let (_temp, storage) = create_test_storage();

        // Create an existing profile named "dev"
        create_profile_with_rules(&storage, "dev", vec![]);

        // Import with same name → should auto-append suffix
        let profile = import_profile_logic("dev".to_string(), "127.0.0.1 a.com", &storage).unwrap();
        assert_eq!(profile.name, "dev (2)");
        assert_eq!(profile.rules.len(), 1);

        // Import again → should get "dev (3)"
        let profile2 = import_profile_logic("dev".to_string(), "::1 localhost", &storage).unwrap();
        assert_eq!(profile2.name, "dev (3)");
    }

    #[test]
    fn test_import_persisted() {
        let (_temp, storage) = create_test_storage();

        let profile =
            import_profile_logic("persisted".to_string(), "127.0.0.1 a.com", &storage).unwrap();

        // Verify it can be loaded back
        let loaded = storage.load_profile(&profile.id).unwrap();
        assert_eq!(loaded.name, "persisted");
        assert_eq!(loaded.rules.len(), 1);

        // Verify it appears in list
        let all = storage.list_profiles().unwrap();
        assert!(
            all.iter().any(|p| p.id == profile.id),
            "imported profile should be in list"
        );
    }

    // -----------------------------------------------------------------------
    // export_profile tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_export_as_hosts() {
        let (_temp, storage) = create_test_storage();
        let profile = create_profile_with_rules(
            &storage,
            "export_test",
            vec![
                ("127.0.0.1".parse().unwrap(), vec!["a.com".to_string()]),
                ("::1".parse().unwrap(), vec!["localhost".to_string()]),
            ],
        );

        let output =
            export_profile_logic(&profile.id.to_string(), ExportFormat::Hosts, &storage).unwrap();

        assert!(
            output.contains("127.0.0.1 a.com"),
            "hosts output should contain first rule"
        );
        assert!(
            output.contains("::1 localhost"),
            "hosts output should contain second rule"
        );
    }

    #[test]
    fn test_export_as_json() {
        let (_temp, storage) = create_test_storage();
        let profile = create_profile_with_rules(
            &storage,
            "json_test",
            vec![("127.0.0.1".parse().unwrap(), vec!["a.com".to_string()])],
        );

        let output =
            export_profile_logic(&profile.id.to_string(), ExportFormat::Json, &storage).unwrap();

        // Verify it's valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["name"], "json_test");
        assert_eq!(parsed["rules"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_export_not_found() {
        let (_temp, storage) = create_test_storage();
        let fake_id = uuid::Uuid::new_v4().to_string();

        let result = export_profile_logic(&fake_id, ExportFormat::Hosts, &storage);
        assert!(
            result.is_err(),
            "exporting non-existent profile should fail"
        );
    }

    #[test]
    fn test_export_roundtrip() {
        let (_temp, storage) = create_test_storage();

        // Create a profile with rules
        let original = import_profile_logic(
            "roundtrip".to_string(),
            "127.0.0.1 a.com\n::1 localhost\n# comment\n192.168.1.1 b.com",
            &storage,
        )
        .unwrap();

        // Export as hosts
        let hosts_text =
            export_profile_logic(&original.id.to_string(), ExportFormat::Hosts, &storage).unwrap();

        // Import back
        let imported =
            import_profile_logic("roundtrip_import".to_string(), &hosts_text, &storage).unwrap();

        // Verify rules match (same count, same IPs and domains)
        assert_eq!(
            original.rules.len(),
            imported.rules.len(),
            "rule count should match after roundtrip"
        );
        for (orig, imp) in original.rules.iter().zip(imported.rules.iter()) {
            assert_eq!(orig.ip, imp.ip, "IP should match");
            assert_eq!(orig.domains, imp.domains, "domains should match");
        }
    }

    // -----------------------------------------------------------------------
    // duplicate_profile tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_duplicate_profile() {
        let (_temp, storage) = create_test_storage();
        let source = create_profile_with_rules(
            &storage,
            "source",
            vec![("127.0.0.1".parse().unwrap(), vec!["a.com".to_string()])],
        );

        let dup =
            duplicate_profile_logic(&source.id.to_string(), "copy".to_string(), &storage).unwrap();

        // New ID
        assert_ne!(dup.id, source.id, "duplicate should have a different ID");
        // New name
        assert_eq!(dup.name, "copy");
        // Disabled
        assert!(!dup.enabled, "duplicate should be disabled");
        // Same rules
        assert_eq!(dup.rules.len(), source.rules.len());
        assert_eq!(dup.rules[0].ip, source.rules[0].ip);
        assert_eq!(dup.rules[0].domains, source.rules[0].domains);
        // Not protected
        assert!(!dup.protected, "duplicate should not be protected");
    }

    #[test]
    fn test_duplicate_empty_name() {
        let (_temp, storage) = create_test_storage();
        let source = create_profile_with_rules(&storage, "source", vec![]);

        let result = duplicate_profile_logic(&source.id.to_string(), "".to_string(), &storage);
        assert!(result.is_err(), "empty name should be rejected");

        let result = duplicate_profile_logic(&source.id.to_string(), "  ".to_string(), &storage);
        assert!(result.is_err(), "whitespace-only name should be rejected");
    }

    #[test]
    fn test_duplicate_name_conflict() {
        let (_temp, storage) = create_test_storage();
        let source = create_profile_with_rules(&storage, "source", vec![]);

        // Create an existing profile named "existing"
        create_profile_with_rules(&storage, "existing", vec![]);

        // Duplicate with conflicting name
        let dup = duplicate_profile_logic(&source.id.to_string(), "existing".to_string(), &storage)
            .unwrap();
        assert_eq!(dup.name, "existing (2)");
    }

    #[test]
    fn test_duplicate_not_found() {
        let (_temp, storage) = create_test_storage();
        let fake_id = uuid::Uuid::new_v4().to_string();

        let result = duplicate_profile_logic(&fake_id, "copy".to_string(), &storage);
        assert!(
            result.is_err(),
            "duplicating non-existent profile should fail"
        );
    }

    #[test]
    fn test_duplicate_preserves_description_and_tags() {
        let (_temp, storage) = create_test_storage();
        let mut source = Profile::new("source");
        source.description = Some("test desc".to_string());
        source.tags = vec!["tag1".to_string(), "tag2".to_string()];
        source.rules.push(HostRule::new(
            "127.0.0.1".parse().unwrap(),
            vec!["a.com".to_string()],
        ));
        storage.save_profile(&source).unwrap();

        let dup =
            duplicate_profile_logic(&source.id.to_string(), "copy".to_string(), &storage).unwrap();

        assert_eq!(dup.description, Some("test desc".to_string()));
        assert_eq!(dup.tags, vec!["tag1".to_string(), "tag2".to_string()]);
    }

    /// **fix issue #67 round 4 (review follow-up)**: 复制的 profile 必须
    /// 保留 source 的 mode。DNS 模式 profile 在新 UI 里有 Duplicate 按钮，
    /// 这个 bug 是 user-visible 的。
    #[test]
    fn test_duplicate_preserves_mode() {
        let (_temp, storage) = create_test_storage();

        // Source: hosts mode (default)
        let hosts_source = create_profile_with_rules(
            &storage,
            "hosts-source",
            vec![("127.0.0.1".parse().unwrap(), vec!["a.com".to_string()])],
        );
        let hosts_dup = duplicate_profile_logic(
            &hosts_source.id.to_string(),
            "hosts-copy".to_string(),
            &storage,
        )
        .unwrap();
        assert_eq!(
            hosts_dup.mode,
            mhost_core::ProfileMode::Hosts,
            "FIX: hosts source → hosts copy"
        );

        // Source: dns mode
        let mut dns_source = Profile::new("dns-source");
        dns_source.mode = mhost_core::ProfileMode::Dns;
        dns_source.rules.push(HostRule::new(
            "127.0.0.1".parse().unwrap(),
            vec!["test.local".to_string()],
        ));
        storage.save_profile(&dns_source).unwrap();

        let dns_dup =
            duplicate_profile_logic(&dns_source.id.to_string(), "dns-copy".to_string(), &storage)
                .unwrap();
        assert_eq!(
            dns_dup.mode,
            mhost_core::ProfileMode::Dns,
            "FIX: dns source must produce dns copy (was always Hosts before fix)"
        );

        // 落盘验证：dns-copy 必须在 profiles/dns/ 而不是 profiles/hosts/
        let dns_path = storage
            .root()
            .join("profiles")
            .join("dns")
            .join(format!("{}.json", dns_dup.id));
        let hosts_path = storage
            .root()
            .join("profiles")
            .join("hosts")
            .join(format!("{}.json", dns_dup.id));
        assert!(
            dns_path.exists(),
            "FIX: dns copy must be saved in profiles/dns/"
        );
        assert!(
            !hosts_path.exists(),
            "FIX: dns copy must NOT be saved in profiles/hosts/"
        );
    }

    // -----------------------------------------------------------------------
    // resolve_name_conflict tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_name_no_conflict() {
        let (_temp, storage) = create_test_storage();
        let name = resolve_name_conflict("new_name", &storage).unwrap();
        assert_eq!(name, "new_name");
    }

    #[test]
    fn test_resolve_name_with_conflict() {
        let (_temp, storage) = create_test_storage();
        create_profile_with_rules(&storage, "taken", vec![]);

        let name = resolve_name_conflict("taken", &storage).unwrap();
        assert_eq!(name, "taken (2)");
    }

    #[test]
    fn test_resolve_name_multiple_conflicts() {
        let (_temp, storage) = create_test_storage();
        create_profile_with_rules(&storage, "dup", vec![]);
        create_profile_with_rules(&storage, "dup (2)", vec![]);

        let name = resolve_name_conflict("dup", &storage).unwrap();
        assert_eq!(name, "dup (3)");
    }
}
