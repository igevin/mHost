//! System hosts writer
//!
//! Safely writes hosts file changes to the system hosts file.
//! Supports backup creation, atomic writes, rollback, and DNS cache flushing.

pub mod backup;
pub mod content;
pub mod verification;

use mhost_core::{ApplyError, ApplyPlan, MhostError};
use mhost_hosts::Parser;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::platform::{create_platform_adapter, PlatformAdapter};

/// Managed block markers
#[allow(dead_code)]
pub const MANAGED_START: &str = "# ---- mHost start ----";
#[allow(dead_code)]
pub const MANAGED_END: &str = "# ---- mHost end ----";

/// Trait for moving a file to another path, potentially with elevated privileges.
#[deprecated(
    note = "Use PlatformAdapter instead. This trait is retained for backward compatibility."
)]
pub trait ElevatedMover: Send + Sync {
    /// Move `from` to `to`.
    fn elevated_move(&self, from: &Path, to: &Path) -> Result<(), MhostError>;
}

/// Production mover using osascript to request administrator privileges.
#[deprecated(note = "Use MacOsAdapter via PlatformAdapter instead.")]
pub struct OsascriptMover;

#[allow(deprecated)]
impl ElevatedMover for OsascriptMover {
    fn elevated_move(&self, from: &Path, to: &Path) -> Result<(), MhostError> {
        let from_escaped = crate::platform::macos::escape_applescript_path(&from.to_string_lossy());
        let to_escaped = crate::platform::macos::escape_applescript_path(&to.to_string_lossy());
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
            return Err(ApplyError::PermissionDenied(stderr.to_string()).into());
        }

        Ok(())
    }
}

/// Test mover using regular `fs::rename`.
pub struct TestMover;

#[allow(deprecated)]
impl ElevatedMover for TestMover {
    fn elevated_move(&self, from: &Path, to: &Path) -> Result<(), MhostError> {
        fs::rename(from, to).map_err(|e| e.into())
    }
}

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
    platform: Box<dyn PlatformAdapter>,
}

impl HostsWriter {
    /// Create a new `HostsWriter` for production use.
    ///
    /// Uses the platform-specific hosts path and the standard
    /// application data directory for backups.
    pub fn new() -> Self {
        let platform = create_platform_adapter();
        Self {
            hosts_path: platform.hosts_path(),
            backup_dir: storage_root().join("backups"),
            platform,
        }
    }

    /// Create a new `HostsWriter` with custom paths (for testing).
    ///
    /// Uses `TestMover` internally wrapped as a `PlatformAdapter`.
    pub fn with_paths(hosts_path: impl Into<PathBuf>, backup_dir: impl Into<PathBuf>) -> Self {
        Self {
            hosts_path: hosts_path.into(),
            backup_dir: backup_dir.into(),
            platform: Box::new(MoverAdapter(Box::new(TestMover))),
        }
    }

    /// Create a new `HostsWriter` with a custom mover (for advanced testing).
    ///
    /// Retained for backward compatibility. The mover is wrapped into a
    /// `PlatformAdapter` internally.
    #[allow(dead_code)]
    #[allow(deprecated)]
    #[deprecated(note = "Prefer with_platform() or with_paths() for new code.")]
    pub fn with_mover(
        hosts_path: impl Into<PathBuf>,
        backup_dir: impl Into<PathBuf>,
        mover: Box<dyn ElevatedMover>,
    ) -> Self {
        Self {
            hosts_path: hosts_path.into(),
            backup_dir: backup_dir.into(),
            platform: Box::new(MoverAdapter(mover)),
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
            Some(backup::create_backup(&self.backup_dir, &current)?)
        } else {
            None
        };

        // 10-12. Write temp file, verify, replace
        let new_content = content::build_hosts_content(&current, plan);
        self.atomic_write(&new_content)?;

        // 13. Flush DNS cache (non-blocking: failure is logged but not fatal)
        if let Err(e) = self.platform.flush_dns_cache() {
            eprintln!("[mHost] Warning: DNS cache flush failed: {}", e);
        }

        // 14. Verify write result
        let written = fs::read_to_string(&self.hosts_path)?;
        if let Err(verify_err) = verification::verify(&written, plan) {
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
    /// Finds the latest backup file (by filesystem modification time) and
    /// restores it to the hosts path. After rollback, verifies the file
    /// content matches the backup.
    ///
    /// Also enforces a maximum number of backups (retains the most recent).
    pub fn rollback(&self) -> Result<(), MhostError> {
        let mut backups: Vec<_> = fs::read_dir(&self.backup_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                let file_name = e.file_name();
                let name = file_name.to_string_lossy();
                name.starts_with("hosts-") && name.ends_with(".bak")
            })
            .collect();

        if backups.is_empty() {
            return Err(ApplyError::BackupFailed("no backup found".to_string()).into());
        }

        // Sort by modification time descending to get the latest backup
        backups.sort_by(|a, b| {
            let time_a = a
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let time_b = b
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            time_b.cmp(&time_a)
        });

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
    // Atomic write
    // -----------------------------------------------------------------------

    /// Write content atomically via a temp file.
    ///
    /// Uses `tempfile::NamedTempFile` to generate a unique temporary file
    /// in the same directory as the target, then moves it into place.
    /// The temp file is automatically cleaned up on failure.
    fn atomic_write(&self, content: &str) -> Result<(), MhostError> {
        let parent = self
            .hosts_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut temp_file = tempfile::NamedTempFile::new_in(parent)?;
        std::io::Write::write_all(&mut temp_file, content.as_bytes())?;
        temp_file.flush()?;

        let temp_path = temp_file.into_temp_path();
        self.platform.elevated_move(&temp_path, &self.hosts_path)?;

        // On success, the temp file has been moved to hosts_path;
        // no explicit cleanup needed.
        Ok(())
    }
}

impl Default for HostsWriter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// MoverAdapter: wraps an ElevatedMover into a PlatformAdapter
// ---------------------------------------------------------------------------

/// Adapter that wraps a legacy `ElevatedMover` into the new `PlatformAdapter`
/// trait. Used for backward compatibility with existing tests.
#[allow(deprecated)]
struct MoverAdapter(Box<dyn ElevatedMover>);

#[allow(deprecated)]
impl PlatformAdapter for MoverAdapter {
    fn hosts_path(&self) -> PathBuf {
        PathBuf::from("/etc/hosts")
    }

    fn elevated_move(&self, from: &Path, to: &Path) -> Result<(), MhostError> {
        self.0.elevated_move(from, to)
    }

    fn flush_dns_cache(&self) -> Result<(), MhostError> {
        // Test movers should not flush DNS
        Ok(())
    }

    fn platform_name(&self) -> &'static str {
        "mover-adapter"
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
        let current = "# before\n# ---- mHost start ----\n127.0.0.1 old.com\n# ---- mHost end ----\n# after\n";
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
}
