//! System hosts writer
//!
//! Safely writes hosts file changes to the system hosts file.
//! Supports backup creation, atomic writes, rollback, and DNS cache flushing.

pub mod backup;
pub mod content;
pub mod verification;

#[cfg(test)]
mod tests;

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

    /// Return the path to the system hosts file.
    pub fn hosts_path(&self) -> &Path {
        &self.hosts_path
    }

    /// Create a new `HostsWriter` with custom paths (for testing).
    pub fn with_paths(hosts_path: impl Into<PathBuf>, backup_dir: impl Into<PathBuf>) -> Self {
        let hosts_path = hosts_path.into();
        Self {
            hosts_path: hosts_path.clone(),
            backup_dir: backup_dir.into(),
            platform: Box::new(TestPlatformAdapter {
                hosts_path,
            }),
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
        // Security fix (#19): Verify hosts_path is a regular file, not a symlink
        ensure_regular_file(&self.hosts_path)?;

        // 1. Read current system hosts
        let current = fs::read_to_string(&self.hosts_path)?;

        // 2. Detect mHost managed block
        let _has_managed = Parser::extract_managed_block(&current).is_some();

        // 3. External modification detection (simplified for phase 0)
        //    -- skipped; will be enhanced in later phases.

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
            log::warn!("DNS cache flush failed: {}", e);
        }

        // 14. Verify write result (use in-memory new_content to avoid re-reading)
        if let Err(verify_err) = verification::verify(&new_content, plan) {
            // Rollback to backup on verification failure
            if let Some(ref backup) = backup_path {
                log::warn!("Verification failed, rolling back...");
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
        // Security fix (#19): Verify hosts_path is a regular file before rollback
        ensure_regular_file(&self.hosts_path)?;

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
        // Create the temp file in the system temp directory (writable by all users).
        // Using /etc as the parent would fail for non-root users.
        let mut temp_file = tempfile::NamedTempFile::new()?;
        std::io::Write::write_all(&mut temp_file, content.as_bytes())?;
        temp_file.flush()?;

        let temp_path = temp_file.into_temp_path();
        self.platform.elevated_move(&temp_path, &self.hosts_path)?;

        // On success, the temp file has been copied to hosts_path and removed;
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
// TestPlatformAdapter: simple adapter for testing without elevated privileges
// ---------------------------------------------------------------------------

/// A simple platform adapter for testing that uses `fs::copy` + `fs::remove_file`
/// instead of elevated moves.
struct TestPlatformAdapter {
    hosts_path: PathBuf,
}

impl PlatformAdapter for TestPlatformAdapter {
    fn hosts_path(&self) -> PathBuf {
        self.hosts_path.clone()
    }

    fn elevated_move(&self, from: &Path, to: &Path) -> Result<(), MhostError> {
        fs::copy(from, to).map_err(MhostError::from)?;
        let _ = fs::remove_file(from);
        Ok(())
    }

    fn flush_dns_cache(&self) -> Result<(), MhostError> {
        Ok(())
    }

    fn platform_name(&self) -> &'static str {
        "test-adapter"
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

/// Verify that a path is a regular file (not a symlink).
/// Security fix (#19): Prevents following symlinks when writing to /etc/hosts.
fn ensure_regular_file(path: &Path) -> Result<(), MhostError> {
    let metadata = fs::symlink_metadata(path).map_err(|e| MhostError::Io {
        kind: e.kind().to_string(),
        message: format!("Cannot stat '{}': {}", path.display(), e),
    })?;
    if !metadata.file_type().is_file() {
        return Err(MhostError::InvalidInput(format!(
            "'{}' is not a regular file (may be a symlink or special file). Refusing to write.",
            path.display()
        )));
    }
    Ok(())
}
