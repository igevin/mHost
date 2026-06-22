//! Backup management for the hosts writer
//!
//! Handles creating timestamped backups of the hosts file and pruning
//! old backups when the count exceeds the configured maximum.

use chrono::Utc;
use mhost_core::MhostError;
use std::fs;
use std::path::{Path, PathBuf};

/// Maximum number of backup files to retain.
const MAX_BACKUPS: usize = 10;

/// Create a timestamped backup of the given content.
///
/// After creating the backup, enforces the maximum backup limit by
/// removing the oldest backups if the count exceeds `MAX_BACKUPS`.
pub fn create_backup(backup_dir: &Path, content: &str) -> Result<PathBuf, MhostError> {
    fs::create_dir_all(backup_dir)?;
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let path = backup_dir.join(format!("hosts-{}.bak", timestamp));
    fs::write(&path, content)?;

    // Enforce backup limit
    prune_old_backups(backup_dir)?;

    Ok(path)
}

/// Remove oldest backups if the total count exceeds `MAX_BACKUPS`.
///
/// Logs warnings for each deletion failure instead of silently ignoring.
pub fn prune_old_backups(backup_dir: &Path) -> Result<(), MhostError> {
    let mut backups: Vec<_> = fs::read_dir(backup_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let file_name = e.file_name();
            let name = file_name.to_string_lossy();
            name.starts_with("hosts-") && name.ends_with(".bak")
        })
        .collect();

    if backups.len() <= MAX_BACKUPS {
        return Ok(());
    }

    // Sort by modification time ascending (oldest first)
    backups.sort_by(|a, b| {
        let time_a = a
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let time_b = b
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        time_a.cmp(&time_b)
    });

    let to_remove = backups.len() - MAX_BACKUPS;
    let mut failed_count = 0;
    for entry in backups.iter().take(to_remove) {
        let path = entry.path();
        if let Err(e) = fs::remove_file(&path) {
            log::warn!("Failed to remove old backup '{}': {}", path.display(), e);
            failed_count += 1;
        }
    }

    if failed_count > 0 {
        log::warn!(
            "Pruned {} old backups, {} deletions failed",
            to_remove - failed_count,
            failed_count
        );
        // Return error if ALL deletions failed (nothing was pruned)
        if failed_count == to_remove {
            return Err(MhostError::Apply(mhost_core::ApplyError::BackupFailed(
                format!("failed to prune old backups: {} deletions failed", failed_count),
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_prune_old_backups_removes_oldest() {
        let dir = tempfile::tempdir().unwrap();

        // Create 15 backup files (exceeds MAX_BACKUPS=10)
        for i in 0..15 {
            let name = format!("hosts-202401{:02}_120000.bak", i);
            let path = dir.path().join(&name);
            fs::write(&path, format!("backup {}", i)).unwrap();
            // Ensure distinct modification times so sorting is deterministic
            // touch the file to slightly stagger mtime
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Verify we created 15 files
        let before: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with("hosts-") && n.ends_with(".bak")
            })
            .collect();
        assert_eq!(before.len(), 15, "should have 15 backups before pruning");

        prune_old_backups(dir.path()).unwrap();

        let after: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with("hosts-") && n.ends_with(".bak")
            })
            .collect();
        assert_eq!(
            after.len(),
            MAX_BACKUPS,
            "should retain exactly MAX_BACKUPS ({}) files after pruning",
            MAX_BACKUPS
        );
    }

    #[test]
    fn test_prune_old_backups_under_limit_is_noop() {
        let dir = tempfile::tempdir().unwrap();

        // Create only 5 backups (under MAX_BACKUPS=10)
        for i in 0..5 {
            let name = format!("hosts-202401{:02}_120000.bak", i);
            let path = dir.path().join(&name);
            fs::write(&path, format!("backup {}", i)).unwrap();
        }

        prune_old_backups(dir.path()).unwrap();

        let after: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with("hosts-") && n.ends_with(".bak")
            })
            .collect();
        assert_eq!(after.len(), 5, "should keep all 5 backups when under limit");
    }

    #[test]
    fn test_prune_old_backups_retains_10_latest() {
        let dir = tempfile::tempdir().unwrap();

        // Create 12 backup files with staggered modification times
        for i in 0..12 {
            let name = format!("hosts-202401{:02}_120000.bak", i);
            let path = dir.path().join(&name);
            fs::write(&path, format!("backup {}", i)).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        prune_old_backups(dir.path()).unwrap();

        let after: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with("hosts-") && n.ends_with(".bak")
            })
            .collect();
        assert_eq!(
            after.len(),
            MAX_BACKUPS,
            "should retain exactly MAX_BACKUPS ({}) files after pruning",
            MAX_BACKUPS
        );

        // Verify the oldest 2 were removed (hosts-20240100 and hosts-20240101)
        let names: Vec<String> = after
            .iter()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            !names.contains(&"hosts-20240100_120000.bak".to_string()),
            "oldest backup should be removed"
        );
        assert!(
            !names.contains(&"hosts-20240101_120000.bak".to_string()),
            "second oldest backup should be removed"
        );
        assert!(
            names.contains(&"hosts-20240111_120000.bak".to_string()),
            "newest backup should be retained"
        );
    }
}
