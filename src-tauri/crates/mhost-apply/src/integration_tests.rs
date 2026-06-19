//! End-to-end integration tests for the mHost apply workflow.
//!
//! These tests verify the complete chain:
//! storage -> merge -> diff -> writer -> rollback
//!
//! All tests use `tempfile::TempDir` and do not touch the real `/etc/hosts`.

use mhost_core::{HostRule, Profile};
use mhost_storage::storage::{FileStorage, Storage};
use std::fs;
use tempfile::TempDir;

use crate::{generate_plan, writer::HostsWriter};

/// Helper: create a `FileStorage` backed by a temp directory.
fn create_test_storage() -> (TempDir, FileStorage) {
    let temp_dir = TempDir::new().unwrap();
    let storage = FileStorage::new(temp_dir.path());
    (temp_dir, storage)
}

/// Helper: create a profile with the given name and a set of (ip, domain) rules.
fn profile_with_rules(name: &str, rules: Vec<(&str, &str)>) -> Profile {
    let mut profile = Profile::new(name);
    profile.enabled = true;
    for (ip, domain) in rules {
        profile
            .rules
            .push(HostRule::new(ip.parse().unwrap(), vec![domain.to_string()]));
    }
    profile
}

// ---------------------------------------------------------------------------
// End-to-end workflow
// ---------------------------------------------------------------------------

#[test]
fn test_end_to_end_workflow() {
    // 1. Create FileStorage (using TempDir)
    let (_temp, storage) = create_test_storage();

    // 2. Create Profile and add rules
    let profile = profile_with_rules(
        "dev",
        vec![
            ("127.0.0.1", "example.com"),
            ("127.0.0.1", "test.local"),
            ("::1", "localhost"),
        ],
    );

    // 3. Save Profile
    storage.save_profile(&profile).unwrap();
    let loaded = storage.load_profile(&profile.id).unwrap();
    assert_eq!(profile, loaded);

    // 4. Enable Profile (set enabled = true, already done in helper)
    //    Verify it is enabled
    assert!(profile.enabled);

    // 5. Generate ApplyPlan (using mock hosts content)
    let mock_hosts_original = "# Original system hosts content\n127.0.0.1 existing.local\n";
    let plan = generate_plan(&[profile], mock_hosts_original).unwrap();

    // Verify diff is correct
    assert_eq!(plan.rules.len(), 3);
    assert!(plan.conflicts.is_empty());
    assert_eq!(plan.diff.added.len(), 3);
    assert!(plan
        .diff
        .added
        .contains(&"127.0.0.1 example.com".to_string()));
    assert!(plan
        .diff
        .added
        .contains(&"127.0.0.1 test.local".to_string()));
    assert!(plan.diff.added.contains(&"::1 localhost".to_string()));
    assert!(plan.backup_required);

    // 6. Use HostsWriter to apply (using TempDir mock hosts file)
    let temp_dir = TempDir::new().unwrap();
    let mock_hosts_path = temp_dir.path().join("hosts");
    let backup_dir = temp_dir.path().join("backups");
    fs::write(&mock_hosts_path, mock_hosts_original).unwrap();

    let writer = HostsWriter::with_paths(&mock_hosts_path, &backup_dir);
    writer.apply(&plan).unwrap();

    // 7. Verify hosts file contains managed block and rules
    let content = fs::read_to_string(&mock_hosts_path).unwrap();
    assert!(
        content.contains("# Original system hosts content"),
        "original unmanaged content should be preserved"
    );
    assert!(
        content.contains("# ---- mHost start ----"),
        "managed block start marker should exist"
    );
    assert!(
        content.contains("# ---- mHost end ----"),
        "managed block end marker should exist"
    );
    assert!(
        content.contains("127.0.0.1 example.com"),
        "rule for example.com should exist"
    );
    assert!(
        content.contains("127.0.0.1 test.local"),
        "rule for test.local should exist"
    );
    assert!(
        content.contains("::1 localhost"),
        "rule for localhost should exist"
    );

    // 8. Verify backup directory has a backup file
    let backups: Vec<_> = fs::read_dir(&backup_dir).unwrap().collect();
    assert_eq!(
        backups.len(),
        1,
        "exactly one backup file should be created"
    );
    let backup_content = fs::read_to_string(backups[0].as_ref().unwrap().path()).unwrap();
    assert_eq!(
        backup_content, mock_hosts_original,
        "backup should contain the original hosts content"
    );

    // 9. Execute rollback
    writer.rollback().unwrap();

    // 10. Verify hosts file restored to original content
    let restored = fs::read_to_string(&mock_hosts_path).unwrap();
    assert_eq!(
        restored, mock_hosts_original,
        "hosts file should be restored to original content after rollback"
    );
}

// ---------------------------------------------------------------------------
// Additional integration scenarios
// ---------------------------------------------------------------------------

#[test]
fn test_end_to_end_update_existing_managed_block() {
    // Setup: existing hosts already has a managed block
    let mock_hosts_original = r#"# Original content
# ---- mHost start ----
127.0.0.1 old.com
# ---- mHost end ----
# Footer
"#;

    let profile = profile_with_rules("dev", vec![("127.0.0.1", "new.com")]);

    let temp_dir = TempDir::new().unwrap();
    let mock_hosts_path = temp_dir.path().join("hosts");
    let backup_dir = temp_dir.path().join("backups");
    fs::write(&mock_hosts_path, mock_hosts_original).unwrap();

    let plan = generate_plan(&[profile], mock_hosts_original).unwrap();
    assert_eq!(plan.rules.len(), 1);
    assert_eq!(plan.diff.added.len(), 1);
    assert_eq!(plan.diff.removed.len(), 1);
    assert_eq!(plan.diff.unchanged.len(), 0);

    let writer = HostsWriter::with_paths(&mock_hosts_path, &backup_dir);
    writer.apply(&plan).unwrap();

    let content = fs::read_to_string(&mock_hosts_path).unwrap();
    assert!(content.contains("# Original content"));
    assert!(content.contains("# Footer"));
    assert!(!content.contains("old.com"), "old rule should be removed");
    assert!(
        content.contains("127.0.0.1 new.com"),
        "new rule should be added"
    );

    // Rollback restores the original content with the old managed block
    writer.rollback().unwrap();
    let restored = fs::read_to_string(&mock_hosts_path).unwrap();
    assert_eq!(restored, mock_hosts_original);
}

#[test]
fn test_end_to_end_empty_rules_removes_managed_block() {
    // Setup: existing hosts has a managed block, but profile has no rules
    let mock_hosts_original = r#"# Original content
# ---- mHost start ----
127.0.0.1 old.com
# ---- mHost end ----
# Footer
"#;

    let profile = profile_with_rules("empty", vec![]);

    let temp_dir = TempDir::new().unwrap();
    let mock_hosts_path = temp_dir.path().join("hosts");
    let backup_dir = temp_dir.path().join("backups");
    fs::write(&mock_hosts_path, mock_hosts_original).unwrap();

    let plan = generate_plan(&[profile], mock_hosts_original).unwrap();
    assert!(plan.rules.is_empty());
    assert_eq!(plan.diff.removed.len(), 1);

    let writer = HostsWriter::with_paths(&mock_hosts_path, &backup_dir);
    writer.apply(&plan).unwrap();

    let content = fs::read_to_string(&mock_hosts_path).unwrap();
    assert!(content.contains("# Original content"));
    assert!(content.contains("# Footer"));
    assert!(
        !content.contains("# ---- mHost start ----"),
        "managed block should be removed when no rules"
    );
    assert!(!content.contains("old.com"));

    writer.rollback().unwrap();
    let restored = fs::read_to_string(&mock_hosts_path).unwrap();
    assert_eq!(restored, mock_hosts_original);
}

#[test]
fn test_end_to_end_with_storage_persistence() {
    // Verify that the storage -> plan -> writer chain works with persisted data
    let (temp_dir, storage) = create_test_storage();

    let profile = profile_with_rules(
        "persisted",
        vec![("192.168.1.1", "my.local"), ("192.168.1.2", "api.local")],
    );
    storage.save_profile(&profile).unwrap();

    // Load from storage and generate plan
    let loaded = storage.list_profiles().unwrap();
    assert_eq!(loaded.len(), 1);

    let mock_hosts = "# system hosts\n";
    let plan = generate_plan(&loaded, mock_hosts).unwrap();
    assert_eq!(plan.rules.len(), 2);

    let hosts_path = temp_dir.path().join("hosts");
    let backup_dir = temp_dir.path().join("backups");
    fs::write(&hosts_path, mock_hosts).unwrap();

    let writer = HostsWriter::with_paths(&hosts_path, &backup_dir);
    writer.apply(&plan).unwrap();

    let content = fs::read_to_string(&hosts_path).unwrap();
    assert!(content.contains("192.168.1.1 my.local"));
    assert!(content.contains("192.168.1.2 api.local"));

    let backups: Vec<_> = fs::read_dir(&backup_dir).unwrap().collect();
    assert_eq!(backups.len(), 1);
}

#[test]
fn test_end_to_end_multiple_applies_and_rollback() {
    // Apply multiple times and verify rollback goes to the most recent backup
    let mock_hosts_v1 = "# version 1\n";

    let mut profile = profile_with_rules("dev", vec![("127.0.0.1", "site1.com")]);

    let temp_dir = TempDir::new().unwrap();
    let hosts_path = temp_dir.path().join("hosts");
    let backup_dir = temp_dir.path().join("backups");
    fs::write(&hosts_path, mock_hosts_v1).unwrap();

    // First apply
    let plan1 = generate_plan(&[profile.clone()], mock_hosts_v1).unwrap();
    let writer = HostsWriter::with_paths(&hosts_path, &backup_dir);
    writer.apply(&plan1).unwrap();

    let content_after_first = fs::read_to_string(&hosts_path).unwrap();
    assert!(content_after_first.contains("site1.com"));

    // Second apply with different rules
    profile.rules.clear();
    profile.rules.push(HostRule::new(
        "127.0.0.1".parse().unwrap(),
        vec!["site2.com".to_string()],
    ));
    let plan2 = generate_plan(&[profile], &content_after_first).unwrap();
    writer.apply(&plan2).unwrap();

    let content_after_second = fs::read_to_string(&hosts_path).unwrap();
    assert!(!content_after_second.contains("site1.com"));
    assert!(content_after_second.contains("site2.com"));

    // Rollback should restore to content_after_first (the most recent backup)
    writer.rollback().unwrap();
    let after_rollback = fs::read_to_string(&hosts_path).unwrap();
    assert_eq!(
        after_rollback, content_after_first,
        "rollback should restore to the most recent backup"
    );
}
