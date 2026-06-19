//! System hosts writer
//!
//! Safely writes hosts file changes to the system hosts file.
//! Supports backup creation, atomic writes, rollback, and DNS cache flushing.

use chrono::Utc;
use mhost_core::{ApplyError, ApplyPlan, MhostError};
use mhost_hosts::Parser;
use std::fs;
use std::path::{Path, PathBuf};

/// Managed block markers
#[allow(dead_code)]
const MANAGED_START: &str = "# ---- mHost start ----";
#[allow(dead_code)]
const MANAGED_END: &str = "# ---- mHost end ----";

/// Writes hosts changes to the system hosts file.
///
/// `HostsWriter` handles:
/// - Reading the current hosts file
/// - Building new content that preserves unmanaged content
/// - Creating backups before modification
/// - Atomic file writes (temp file + move)
/// - DNS cache flushing
/// - Verification after write
/// - Rollback to the most recent backup
pub struct HostsWriter {
    hosts_path: PathBuf,
    backup_dir: PathBuf,
}

impl HostsWriter {
    /// Create a new `HostsWriter` for production use.
    ///
    /// Uses `/etc/hosts` as the system hosts path and the standard
    /// application data directory for backups.
    pub fn new() -> Self {
        Self {
            hosts_path: PathBuf::from("/etc/hosts"),
            backup_dir: storage_root().join("backups"),
        }
    }

    /// Create a new `HostsWriter` with custom paths (for testing).
    pub fn with_paths(hosts_path: impl Into<PathBuf>, backup_dir: impl Into<PathBuf>) -> Self {
        Self {
            hosts_path: hosts_path.into(),
            backup_dir: backup_dir.into(),
        }
    }

    /// Apply an `ApplyPlan` to the system hosts file.
    ///
    /// The full flow:
    /// 1. Read current hosts content
    /// 2. Detect managed block presence
    /// 3. Create backup (if required)
    /// 4. Build new content (preserve unmanaged, replace managed block)
    /// 5. Atomic write via temp file + elevated move
    /// 6. Flush DNS cache (non-blocking)
    /// 7. Verify written content
    /// 8. Rollback on verification failure
    pub fn apply(&self, plan: &ApplyPlan) -> Result<(), MhostError> {
        // 1. Read current system hosts
        let current = fs::read_to_string(&self.hosts_path)?;

        // 2. Detect mHost managed block
        let _has_managed = Parser::extract_managed_block(&current).is_some();

        // 3. External modification detection (simplified for phase 0)
        //    — skipped; will be enhanced in later phases.

        // 4-7. Merging, validation, conflict detection, diff: done by ApplyPlan.

        // 8. User confirmation is handled at the UI layer.

        // 9. Create backup if required
        let backup_path = if plan.backup_required {
            Some(self.create_backup(&current)?)
        } else {
            None
        };

        // 10-12. Write temp file, verify, replace
        let new_content = self.build_hosts_content(&current, plan);
        self.atomic_write(&new_content)?;

        // 13. Flush DNS cache (non-blocking: failure is logged but not fatal)
        if let Err(e) = self.flush_dns_cache() {
            eprintln!("[mHost] Warning: DNS cache flush failed: {}", e);
        }

        // 14. Verify write result
        let written = fs::read_to_string(&self.hosts_path)?;
        if let Err(verify_err) = self.verify(&written, plan) {
            // Rollback to backup on verification failure
            if let Some(ref backup) = backup_path {
                eprintln!("[mHost] Verification failed, rolling back...");
                let backup_content = fs::read_to_string(backup)?;
                if let Err(rollback_err) = self.atomic_write(&backup_content) {
                    return Err(ApplyError::VerificationFailed(format!(
                        "verify failed ({}), rollback also failed ({})",
                        verify_err, rollback_err
                    ))
                    .into());
                }
                // Verify rollback succeeded
                let rolled_back = fs::read_to_string(&self.hosts_path)?;
                if rolled_back != backup_content {
                    return Err(ApplyError::VerificationFailed(
                        "verify failed and rollback content mismatch".to_string(),
                    )
                    .into());
                }
            }
            return Err(
                ApplyError::VerificationFailed(format!("verify failed: {}", verify_err)).into(),
            );
        }

        Ok(())
    }

    /// Rollback to the most recent backup.
    ///
    /// Finds the latest backup file (sorted by filename, which includes a
    /// timestamp) and restores it to the hosts path.
    /// After rollback, verifies the file content matches the backup.
    pub fn rollback(&self) -> Result<(), MhostError> {
        let mut backups: Vec<_> = fs::read_dir(&self.backup_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name().to_string_lossy().starts_with("hosts-")
                    && e.file_name().to_string_lossy().ends_with(".bak")
            })
            .collect();

        if backups.is_empty() {
            return Err(ApplyError::BackupFailed("no backup found".to_string()).into());
        }

        // Sort by filename descending to get the latest backup
        backups.sort_by_key(|b| std::cmp::Reverse(b.file_name()));
        let latest = &backups[0];
        let latest_path = latest.path();

        let backup_content = fs::read_to_string(&latest_path)?;
        self.atomic_write(&backup_content)?;

        // Verify rollback succeeded
        let rolled_back = fs::read_to_string(&self.hosts_path)?;
        if rolled_back != backup_content {
            return Err(ApplyError::BackupFailed("rollback content mismatch".to_string()).into());
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Content building
    // -----------------------------------------------------------------------

    /// Build the new hosts content.
    ///
    /// - If a managed block exists, remove it and replace with the new block.
    /// - If no managed block exists, append the new block at the end.
    /// - All unmanaged content is preserved exactly as-is, including trailing
    ///   whitespace.
    fn build_hosts_content(&self, current: &str, plan: &ApplyPlan) -> String {
        let managed_block = crate::format_as_hosts(&plan.rules);

        if let Some((start, end)) = Parser::extract_managed_block(current) {
            // Replace existing managed block using byte offsets to preserve
            // original formatting including trailing whitespace.
            let line_offsets: Vec<(usize, usize)> = current
                .lines()
                .scan(0, |pos, line| {
                    let line_start = *pos;
                    // lines() does not include the newline; find it manually
                    let after_line = line_start + line.len();
                    let nl_len = if current[after_line..].starts_with("\r\n") {
                        2
                    } else if current[after_line..].starts_with('\n') {
                        1
                    } else {
                        0
                    };
                    *pos = after_line + nl_len;
                    Some((line_start, *pos))
                })
                .collect();

            let block_start = line_offsets[start].0;
            let block_end = line_offsets[end].1;

            let mut output = String::new();
            output.push_str(&current[..block_start]);
            if !managed_block.is_empty() {
                output.push_str(&managed_block);
            }
            output.push_str(&current[block_end..]);
            output
        } else {
            // No managed block — append at the end
            let mut output = current.to_string();
            if !output.ends_with('\n') && !output.is_empty() {
                output.push('\n');
            }
            if !managed_block.is_empty() {
                output.push_str(&managed_block);
            }
            output
        }
    }

    // -----------------------------------------------------------------------
    // Backup
    // -----------------------------------------------------------------------

    /// Create a timestamped backup of the given content.
    fn create_backup(&self, content: &str) -> Result<PathBuf, MhostError> {
        fs::create_dir_all(&self.backup_dir)?;
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let path = self.backup_dir.join(format!("hosts-{}.bak", timestamp));
        fs::write(&path, content)?;
        Ok(path)
    }

    // -----------------------------------------------------------------------
    // Atomic write
    // -----------------------------------------------------------------------

    /// Write content atomically via a temp file.
    fn atomic_write(&self, content: &str) -> Result<(), MhostError> {
        let temp = self.hosts_path.with_extension("tmp");
        fs::write(&temp, content)?;
        self.elevated_move(&temp, &self.hosts_path)?;
        Ok(())
    }

    /// Escape a path for safe use inside an AppleScript string literal.
    ///
    /// AppleScript string literals use `\` as the escape character, so
    /// backslashes and double quotes must be escaped.
    fn escape_applescript_path(path: &str) -> String {
        path.replace('\\', "\\\\").replace('"', "\\\"")
    }

    /// Move `from` to `to` with elevated privileges (macOS).
    ///
    /// Uses `osascript` to prompt the user for administrator privileges.
    /// In test environments this falls back to a regular `fs::rename` when
    /// the temp file and target are in the same directory (mock setup).
    fn elevated_move(&self, from: &Path, to: &Path) -> Result<(), MhostError> {
        // In tests (same directory, not /etc/hosts) we can do a regular rename
        if self.hosts_path != Path::new("/etc/hosts") {
            return fs::rename(from, to).map_err(|e| e.into());
        }

        let from_escaped = Self::escape_applescript_path(&from.to_string_lossy());
        let to_escaped = Self::escape_applescript_path(&to.to_string_lossy());
        let script = format!(
            "do shell script \"mv {} {}\" with administrator privileges",
            from_escaped, to_escaped
        );
        let output = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Clean up temp file on failure
            let _ = fs::remove_file(from);
            return Err(ApplyError::PermissionDenied(stderr.to_string()).into());
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // DNS cache
    // -----------------------------------------------------------------------

    /// Flush the system DNS cache (macOS).
    fn flush_dns_cache(&self) -> Result<(), MhostError> {
        // Skip DNS flush in test environments
        if self.hosts_path != Path::new("/etc/hosts") {
            return Ok(());
        }

        std::process::Command::new("dscacheutil")
            .args(["-flushcache"])
            .output()?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Verification
    // -----------------------------------------------------------------------

    /// Verify that the written content matches the expected plan.
    fn verify(&self, written: &str, plan: &ApplyPlan) -> Result<(), MhostError> {
        // Basic verification: check that the managed block markers exist
        // if the plan has rules, and that all expected rules are present.
        if plan.rules.is_empty() {
            // If no rules, there should be no managed block
            if Parser::extract_managed_block(written).is_some() {
                return Err(ApplyError::VerificationFailed(
                    "expected no managed block but found one".to_string(),
                )
                .into());
            }
            return Ok(());
        }

        let block = Parser::extract_managed_block(written);
        if block.is_none() {
            return Err(ApplyError::VerificationFailed("managed block missing".to_string()).into());
        }

        // Verify each rule appears in the written content
        for rule in &plan.rules {
            let expected = format!("{} {}", rule.ip, rule.domain);
            if !written.contains(&expected) {
                return Err(ApplyError::VerificationFailed(format!(
                    "expected rule '{}' not found",
                    expected
                ))
                .into());
            }
        }

        Ok(())
    }
}

impl Default for HostsWriter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the application storage root directory.
fn storage_root() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mHost")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mhost_core::{HostsDiff, ProfileId, ResolvedRule};
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
        let temp_dir = TempDir::new().unwrap();
        let writer = HostsWriter::with_paths(
            temp_dir.path().join("hosts"),
            temp_dir.path().join("backups"),
        );

        let current = "# original content\n";
        let plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "example.com", "p1")]);
        let content = writer.build_hosts_content(current, &plan);

        assert!(content.contains("# original content"));
        assert!(content.contains(MANAGED_START));
        assert!(content.contains(MANAGED_END));
        assert!(content.contains("127.0.0.1 example.com"));
    }

    #[test]
    fn test_build_content_update_replaces_block() {
        let temp_dir = TempDir::new().unwrap();
        let writer = HostsWriter::with_paths(
            temp_dir.path().join("hosts"),
            temp_dir.path().join("backups"),
        );

        let current = "# before\n# ---- mHost start ----\n127.0.0.1 old.com\n# ---- mHost end ----\n# after\n";
        let plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "new.com", "p1")]);
        let content = writer.build_hosts_content(current, &plan);

        assert!(content.contains("# before"));
        assert!(content.contains("# after"));
        assert!(!content.contains("old.com"));
        assert!(content.contains("127.0.0.1 new.com"));
    }

    #[test]
    fn test_build_content_empty_rules_no_block() {
        let temp_dir = TempDir::new().unwrap();
        let writer = HostsWriter::with_paths(
            temp_dir.path().join("hosts"),
            temp_dir.path().join("backups"),
        );

        let current = "# original\n";
        let plan = plan_with_rules(vec![]);
        let content = writer.build_hosts_content(current, &plan);

        assert!(content.contains("# original"));
        assert!(!content.contains(MANAGED_START));
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
    fn test_multiple_backups_sorted_correctly() {
        let temp_dir = TempDir::new().unwrap();
        let backup_dir = temp_dir.path().join("backups");
        fs::create_dir_all(&backup_dir).unwrap();

        // Create two backups with different timestamps
        fs::write(backup_dir.join("hosts-20240101_120000.bak"), "old").unwrap();
        fs::write(backup_dir.join("hosts-20240102_120000.bak"), "new").unwrap();

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
        let temp_dir = TempDir::new().unwrap();
        let writer = HostsWriter::with_paths(
            temp_dir.path().join("hosts"),
            temp_dir.path().join("backups"),
        );

        // File ends with two blank lines (trailing whitespace)
        let current =
            "# before\n\n# ---- mHost start ----\n127.0.0.1 old.com\n# ---- mHost end ----\n\n";
        let plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "new.com", "p1")]);
        let content = writer.build_hosts_content(current, &plan);

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
        let temp_dir = TempDir::new().unwrap();
        let writer = HostsWriter::with_paths(
            temp_dir.path().join("hosts"),
            temp_dir.path().join("backups"),
        );

        let current = "# original\n\n";
        let plan = plan_with_rules(vec![resolved_rule("127.0.0.1", "example.com", "p1")]);
        let content = writer.build_hosts_content(current, &plan);

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
    fn test_escape_applescript_path() {
        assert_eq!(
            HostsWriter::escape_applescript_path("/path/to/file"),
            "/path/to/file"
        );
        assert_eq!(
            HostsWriter::escape_applescript_path("/path/with\"quote"),
            "/path/with\\\"quote"
        );
        assert_eq!(
            HostsWriter::escape_applescript_path("/path/with\\backslash"),
            "/path/with\\\\backslash"
        );
        assert_eq!(
            HostsWriter::escape_applescript_path("/path/with\\\"both"),
            "/path/with\\\\\\\"both"
        );
    }
}
