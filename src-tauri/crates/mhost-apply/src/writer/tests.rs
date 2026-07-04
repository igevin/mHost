use super::*;
use mhost_core::{HostsDiff, ProfileId, ResolvedRule};
use std::fs;
use tempfile::TempDir;
use uuid::Uuid;

/// Helper: create a minimal `ApplyPlan` with the given rules.
fn plan_with_rules(rules: Vec<ResolvedRule>) -> ApplyPlan {
    ApplyPlan {
        rules,
        conflicts: vec![],
        diff: HostsDiff {
            added: vec![],
            removed: vec![],
            unchanged: vec![],
        },
        backup_required: true,
    }
}

/// Helper: create a single resolved rule.
fn resolved_rule(ip: &str, domain: &str, profile_name: &str) -> ResolvedRule {
    ResolvedRule {
        ip: ip.parse().unwrap(),
        domain: domain.to_string(),
        source_profile_id: ProfileId(Uuid::new_v4()),
        source_profile_name: profile_name.to_string(),
    }
}

// -----------------------------------------------------------------------
// build_hosts_content tests
// -----------------------------------------------------------------------

#[test]
fn test_build_content_first_write_appends_block() {
    let current = "# original content\n";
    let plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "example.com", "p1")]);
    let content = content::build_hosts_content(current, &plan);

    assert!(content.contains("# original content"));
    assert!(content.contains(MANAGED_START));
    assert!(content.contains(MANAGED_END));
    assert!(content.contains("127.0.0.1 example.com"));
}

#[test]
fn test_build_content_update_replaces_block() {
    let current =
        "# before\n# ---- mHost start ----\n127.0.0.1 old.com\n# ---- mHost end ----\n# after\n";
    let plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "new.com", "p1")]);
    let content = content::build_hosts_content(current, &plan);

    assert!(content.contains("# before"));
    assert!(content.contains("# after"));
    assert!(!content.contains("old.com"));
    assert!(content.contains("127.0.0.1 new.com"));
}

#[test]
fn test_build_content_empty_rules_no_block() {
    let current = "# original\n";
    let plan = plan_with_rules(vec![]);
    let content = content::build_hosts_content(current, &plan);

    assert!(content.contains("# original"));
    assert!(!content.contains(MANAGED_START));
}

#[test]
fn test_build_hosts_content_empty_file() {
    let current = "";
    let plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "example.com", "p1")]);
    let content = content::build_hosts_content(current, &plan);

    assert!(content.contains(MANAGED_START));
    assert!(content.contains(MANAGED_END));
    assert!(content.contains("127.0.0.1 example.com"));
}

// -----------------------------------------------------------------------
// apply tests (integration-style with temp dirs)
// -----------------------------------------------------------------------

#[test]
fn test_first_write_creates_managed_block() {
    let temp_dir = TempDir::new().unwrap();
    let hosts_path = temp_dir.path().join("hosts");
    fs::write(&hosts_path, "# original content\n").unwrap();

    let writer = HostsWriter::with_paths(&hosts_path, temp_dir.path().join("backups"));
    let plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "example.com", "p1")]);
    writer.apply(&plan).unwrap();

    let content = fs::read_to_string(&hosts_path).unwrap();
    assert!(content.contains("# original content"));
    assert!(content.contains("# ---- mHost start ----"));
    assert!(content.contains("# ---- mHost end ----"));
}

#[test]
fn test_update_replaces_managed_block() {
    let temp_dir = TempDir::new().unwrap();
    let hosts_path = temp_dir.path().join("hosts");
    fs::write(
        &hosts_path,
        "# before\n# ---- mHost start ----\n127.0.0.1 old.com\n# ---- mHost end ----\n# after\n",
    )
    .unwrap();

    let writer = HostsWriter::with_paths(&hosts_path, temp_dir.path().join("backups"));
    let plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "new.com", "p1")]);
    writer.apply(&plan).unwrap();

    let content = fs::read_to_string(&hosts_path).unwrap();
    assert!(content.contains("# before"));
    assert!(content.contains("# after"));
    assert!(!content.contains("old.com"));
    assert!(content.contains("127.0.0.1 new.com"));
}

#[test]
fn test_backup_created() {
    let temp_dir = TempDir::new().unwrap();
    let hosts_path = temp_dir.path().join("hosts");
    let backup_dir = temp_dir.path().join("backups");
    fs::write(&hosts_path, "original").unwrap();

    let writer = HostsWriter::with_paths(&hosts_path, &backup_dir);
    let plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "example.com", "p1")]);
    writer.apply(&plan).unwrap();

    let backups: Vec<_> = fs::read_dir(&backup_dir).unwrap().collect();
    assert_eq!(backups.len(), 1);
}

#[test]
fn test_rollback_restores_backup() {
    let temp_dir = TempDir::new().unwrap();
    let hosts_path = temp_dir.path().join("hosts");
    fs::write(&hosts_path, "original").unwrap();

    let writer = HostsWriter::with_paths(&hosts_path, temp_dir.path().join("backups"));
    let plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "example.com", "p1")]);
    writer.apply(&plan).unwrap();

    // Verify the file was modified
    let modified = fs::read_to_string(&hosts_path).unwrap();
    assert!(modified.contains("example.com"));

    // Rollback
    writer.rollback().unwrap();

    let content = fs::read_to_string(&hosts_path).unwrap();
    assert_eq!(content, "original");
}

#[test]
fn test_write_failure_preserves_original() {
    let temp_dir = TempDir::new().unwrap();
    let hosts_path = temp_dir.path().join("hosts");
    fs::write(&hosts_path, "original").unwrap();

    // Make the directory read-only so atomic_write (temp file creation) fails
    let mut dir_perms = fs::metadata(temp_dir.path()).unwrap().permissions();
    let original_dir_perms = dir_perms.clone();
    dir_perms.set_readonly(true);
    fs::set_permissions(temp_dir.path(), dir_perms).unwrap();

    let writer = HostsWriter::with_paths(&hosts_path, temp_dir.path().join("backups"));
    let plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "example.com", "p1")]);
    let result = writer.apply(&plan);

    // Restore permissions before assertions so cleanup can succeed
    fs::set_permissions(temp_dir.path(), original_dir_perms).unwrap();

    assert!(result.is_err());
    let content = fs::read_to_string(&hosts_path).unwrap();
    assert_eq!(content, "original");
}

#[test]
fn test_multiple_backups_sorted_by_mtime() {
    let temp_dir = TempDir::new().unwrap();
    let backup_dir = temp_dir.path().join("backups");
    fs::create_dir_all(&backup_dir).unwrap();

    // Create two backups with different timestamps (simulate older/newer)
    let old_path = backup_dir.join("hosts-20240101_120000.bak");
    let new_path = backup_dir.join("hosts-20240102_120000.bak");
    fs::write(&old_path, "old").unwrap();
    // Ensure there's a detectable time difference
    std::thread::sleep(std::time::Duration::from_millis(50));
    fs::write(&new_path, "new").unwrap();

    let hosts_path = temp_dir.path().join("hosts");
    fs::write(&hosts_path, "current").unwrap();

    let writer = HostsWriter::with_paths(&hosts_path, &backup_dir);
    writer.rollback().unwrap();

    let content = fs::read_to_string(&hosts_path).unwrap();
    assert_eq!(content, "new");
}

#[test]
fn test_apply_preserves_unmanaged_rules() {
    let temp_dir = TempDir::new().unwrap();
    let hosts_path = temp_dir.path().join("hosts");
    fs::write(
        &hosts_path,
        "# unmanaged\n127.0.0.1 unmanaged.com\n\n# ---- mHost start ----\n127.0.0.1 managed.com\n# ---- mHost end ----\n",
    )
    .unwrap();

    let writer = HostsWriter::with_paths(&hosts_path, temp_dir.path().join("backups"));
    let plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "new-managed.com", "p1")]);
    writer.apply(&plan).unwrap();

    let content = fs::read_to_string(&hosts_path).unwrap();
    assert!(content.contains("127.0.0.1 unmanaged.com"));
    assert!(!content.contains("127.0.0.1 managed.com"));
    assert!(content.contains("127.0.0.1 new-managed.com"));
}

#[test]
fn test_apply_empty_rules_removes_managed_block() {
    let temp_dir = TempDir::new().unwrap();
    let hosts_path = temp_dir.path().join("hosts");
    fs::write(
        &hosts_path,
        "# before\n# ---- mHost start ----\n127.0.0.1 old.com\n# ---- mHost end ----\n# after\n",
    )
    .unwrap();

    let writer = HostsWriter::with_paths(&hosts_path, temp_dir.path().join("backups"));
    let plan = plan_with_rules(vec![]);
    writer.apply(&plan).unwrap();

    let content = fs::read_to_string(&hosts_path).unwrap();
    assert!(content.contains("# before"));
    assert!(content.contains("# after"));
    assert!(!content.contains(MANAGED_START));
    assert!(!content.contains(MANAGED_END));
    assert!(!content.contains("old.com"));
}

#[test]
fn test_rollback_no_backup_fails() {
    let temp_dir = TempDir::new().unwrap();
    let hosts_path = temp_dir.path().join("hosts");
    fs::write(&hosts_path, "original").unwrap();

    let writer = HostsWriter::with_paths(&hosts_path, temp_dir.path().join("backups"));
    let result = writer.rollback();

    assert!(result.is_err());
}

#[test]
fn test_build_content_preserves_trailing_newlines() {
    // File ends with two blank lines (trailing whitespace)
    let current =
        "# before\n\n# ---- mHost start ----\n127.0.0.1 old.com\n# ---- mHost end ----\n\n";
    let plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "new.com", "p1")]);
    let content = content::build_hosts_content(current, &plan);

    // Should preserve the trailing blank lines
    assert!(content.contains("# before"));
    assert!(!content.contains("old.com"));
    assert!(content.contains("127.0.0.1 new.com"));
    assert!(
        content.ends_with("\n\n"),
        "trailing newlines should be preserved, got: {:?}",
        content
    );
}

#[test]
fn test_build_content_preserves_trailing_blank_lines_no_block() {
    let current = "# original\n\n";
    let plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "example.com", "p1")]);
    let content = content::build_hosts_content(current, &plan);

    assert!(content.contains("# original"));
    assert!(content.contains("127.0.0.1 example.com"));
    // The managed block is appended after the trailing newlines
    assert!(content.contains("# original\n\n# ---- mHost start ----"));
}

#[test]
fn test_backup_required_false_skips_backup() {
    let temp_dir = TempDir::new().unwrap();
    let hosts_path = temp_dir.path().join("hosts");
    let backup_dir = temp_dir.path().join("backups");
    fs::write(&hosts_path, "original").unwrap();

    let writer = HostsWriter::with_paths(&hosts_path, &backup_dir);
    let mut plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "example.com", "p1")]);
    plan.backup_required = false;
    writer.apply(&plan).unwrap();

    // Backup dir may not exist at all when backup_required is false
    let backups: Vec<_> = fs::read_dir(&backup_dir)
        .map(|d| d.collect())
        .unwrap_or_default();
    assert_eq!(
        backups.len(),
        0,
        "backup should not be created when backup_required is false"
    );
}

#[test]
fn test_backup_limit_prunes_oldest() {
    let temp_dir = TempDir::new().unwrap();
    let backup_dir = temp_dir.path().join("backups");
    fs::create_dir_all(&backup_dir).unwrap();

    // Create 12 backups (exceeds MAX_BACKUPS = 10)
    for i in 0..12 {
        let path = backup_dir.join(format!("hosts-202401{:02}_120000.bak", i + 1));
        fs::write(&path, format!("backup-{}", i)).unwrap();
        // Small sleep to ensure distinct modification times
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let hosts_path = temp_dir.path().join("hosts");
    fs::write(&hosts_path, "current").unwrap();

    let writer = HostsWriter::with_paths(&hosts_path, &backup_dir);
    let plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "example.com", "p1")]);
    writer.apply(&plan).unwrap();

    // After apply, should have at most MAX_BACKUPS backups
    let backups: Vec<_> = fs::read_dir(&backup_dir).unwrap().collect();
    assert!(
        backups.len() <= 10,
        "expected at most 10 backups, got {}",
        backups.len()
    );
}
