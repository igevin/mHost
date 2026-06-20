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
        let from_escaped = crate::platform::macos::escape_applescript_path(&from.to_string_lossy())?;
        let to_escaped = crate::platform::macos::escape_applescript_path(&to.to_string_lossy())?;
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

    /// Return the path to the system hosts file.
    pub fn hosts_path(&self) -> &Path {
        &self.hosts_path
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
