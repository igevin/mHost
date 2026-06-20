use std::str::FromStr;

use mhost_core::{ExportFormat, MhostError, Profile, ProfileId};
use mhost_hosts::formatter::format_rules;
use mhost_hosts::parser::Parser;
use mhost_storage::storage::Storage;
use tauri::State;

use crate::state::AppState;

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
    loop {
        let candidate = format!("{} ({})", name, counter);
        if !existing_names.contains(&candidate.as_str()) {
            return Ok(candidate);
        }
        counter += 1;
    }
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
            ("with_comments", "# header\n127.0.0.1 x.com # inline", 1),
            ("empty", "", 0),
        ];

        for (name, hosts_text, expected_rules) in cases {
            let profile = import_profile_logic(
                format!("import_{}", name),
                hosts_text,
                &storage,
            )
            .unwrap();
            assert_eq!(
                profile.rules.len(),
                expected_rules,
                "case: {} — expected {} rules",
                name,
                expected_rules
            );
            assert!(!profile.enabled, "case: {} — imported profile should be disabled", name);
        }
    }

    #[test]
    fn test_import_rejects_invalid() {
        let (_temp, storage) = create_test_storage();

        // Invalid IP
        let result = import_profile_logic("bad".to_string(), "999.999.999.999 x.com", &storage);
        assert!(result.is_err(), "invalid IP should be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("parse error"), "error should mention parse errors: {}", msg);

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
        create_profile_with_rules(
            &storage,
            "dev",
            vec![],
        );

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

        let profile = import_profile_logic("persisted".to_string(), "127.0.0.1 a.com", &storage).unwrap();

        // Verify it can be loaded back
        let loaded = storage.load_profile(&profile.id).unwrap();
        assert_eq!(loaded.name, "persisted");
        assert_eq!(loaded.rules.len(), 1);

        // Verify it appears in list
        let all = storage.list_profiles().unwrap();
        assert!(all.iter().any(|p| p.id == profile.id), "imported profile should be in list");
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

        let output = export_profile_logic(
            &profile.id.to_string(),
            ExportFormat::Hosts,
            &storage,
        )
        .unwrap();

        assert!(output.contains("127.0.0.1 a.com"), "hosts output should contain first rule");
        assert!(output.contains("::1 localhost"), "hosts output should contain second rule");
    }

    #[test]
    fn test_export_as_json() {
        let (_temp, storage) = create_test_storage();
        let profile = create_profile_with_rules(
            &storage,
            "json_test",
            vec![("127.0.0.1".parse().unwrap(), vec!["a.com".to_string()])],
        );

        let output = export_profile_logic(
            &profile.id.to_string(),
            ExportFormat::Json,
            &storage,
        )
        .unwrap();

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
        assert!(result.is_err(), "exporting non-existent profile should fail");
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
        let hosts_text = export_profile_logic(
            &original.id.to_string(),
            ExportFormat::Hosts,
            &storage,
        )
        .unwrap();

        // Import back
        let imported = import_profile_logic("roundtrip_import".to_string(), &hosts_text, &storage).unwrap();

        // Verify rules match (same count, same IPs and domains)
        assert_eq!(original.rules.len(), imported.rules.len(), "rule count should match after roundtrip");
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

        let dup = duplicate_profile_logic(
            &source.id.to_string(),
            "copy".to_string(),
            &storage,
        )
        .unwrap();

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
        let source = create_profile_with_rules(
            &storage,
            "source",
            vec![],
        );

        let result = duplicate_profile_logic(&source.id.to_string(), "".to_string(), &storage);
        assert!(result.is_err(), "empty name should be rejected");

        let result = duplicate_profile_logic(&source.id.to_string(), "  ".to_string(), &storage);
        assert!(result.is_err(), "whitespace-only name should be rejected");
    }

    #[test]
    fn test_duplicate_name_conflict() {
        let (_temp, storage) = create_test_storage();
        let source = create_profile_with_rules(
            &storage,
            "source",
            vec![],
        );

        // Create an existing profile named "existing"
        create_profile_with_rules(&storage, "existing", vec![]);

        // Duplicate with conflicting name
        let dup = duplicate_profile_logic(
            &source.id.to_string(),
            "existing".to_string(),
            &storage,
        )
        .unwrap();
        assert_eq!(dup.name, "existing (2)");
    }

    #[test]
    fn test_duplicate_not_found() {
        let (_temp, storage) = create_test_storage();
        let fake_id = uuid::Uuid::new_v4().to_string();

        let result = duplicate_profile_logic(&fake_id, "copy".to_string(), &storage);
        assert!(result.is_err(), "duplicating non-existent profile should fail");
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

        let dup = duplicate_profile_logic(
            &source.id.to_string(),
            "copy".to_string(),
            &storage,
        )
        .unwrap();

        assert_eq!(dup.description, Some("test desc".to_string()));
        assert_eq!(dup.tags, vec!["tag1".to_string(), "tag2".to_string()]);
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
