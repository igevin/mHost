//! Backup management for the hosts writer
//!
//! Handles creating timestamped backups of the hosts file and pruning
//! old backups when the count exceeds the configured maximum.

use chrono::Utc;
use mhost_core::{BackupInfo, MhostError};
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

/// List all backup files in the given directory.
///
/// Returns backup metadata sorted by timestamp descending (newest first).
/// Filters files matching `hosts-*.bak`.
pub fn list_backups(backup_dir: &Path) -> Result<Vec<BackupInfo>, MhostError> {
    let mut backups: Vec<BackupInfo> = Vec::new();

    let entries = match fs::read_dir(backup_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(backups),
        Err(e) => return Err(e.into()),
    };

    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("hosts-") || !name.ends_with(".bak") {
            continue;
        }

        let metadata = entry.metadata()?;
        let modified = metadata.modified()?;
        let timestamp = chrono::DateTime::<chrono::Utc>::from(modified).to_rfc3339();

        backups.push(BackupInfo {
            id: name.clone(),
            filename: name,
            timestamp,
            size: metadata.len(),
            path: entry.path().to_string_lossy().to_string(),
        });
    }

    // Sort by timestamp descending (newest first)
    backups.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    Ok(backups)
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
            // Set distinct modification times so sorting is deterministic
            let mtime = filetime::FileTime::from_unix_time(i as i64, 0);
            filetime::set_file_mtime(&path, mtime).unwrap();
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

        // Create 12 backup files with distinct modification times
        for i in 0..12 {
            let name = format!("hosts-202401{:02}_120000.bak", i);
            let path = dir.path().join(&name);
            fs::write(&path, format!("backup {}", i)).unwrap();
            let mtime = filetime::FileTime::from_unix_time(i as i64, 0);
            filetime::set_file_mtime(&path, mtime).unwrap();
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

    // -----------------------------------------------------------------------
    // list_backups tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_list_backups_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let backups = list_backups(dir.path()).unwrap();
        assert!(backups.is_empty(), "should return empty vec for empty dir");
    }

    #[test]
    fn test_list_backups_filters_non_backup_files() {
        let dir = tempfile::tempdir().unwrap();

        // Create backup files
        fs::write(dir.path().join("hosts-20240101_120000.bak"), "backup1").unwrap();
        fs::write(dir.path().join("hosts-20240102_120000.bak"), "backup2").unwrap();

        // Create non-backup files
        fs::write(dir.path().join("random.txt"), "random").unwrap();
        fs::write(dir.path().join("hosts-20240103.txt"), "not a backup").unwrap();
        fs::write(dir.path().join("other-20240102_120000.bak"), "other").unwrap();

        let backups = list_backups(dir.path()).unwrap();
        assert_eq!(backups.len(), 2, "should only include hosts-*.bak files");
    }

    #[test]
    fn test_list_backups_sorted_descending() {
        let dir = tempfile::tempdir().unwrap();

        // Create backups with distinct modification times
        let path1 = dir.path().join("hosts-20240101_120000.bak");
        let path2 = dir.path().join("hosts-20240102_120000.bak");
        let path3 = dir.path().join("hosts-20240103_120000.bak");

        fs::write(&path1, "backup1").unwrap();
        let mtime1 = filetime::FileTime::from_unix_time(1000, 0);
        filetime::set_file_mtime(&path1, mtime1).unwrap();

        fs::write(&path2, "backup2").unwrap();
        let mtime2 = filetime::FileTime::from_unix_time(2000, 0);
        filetime::set_file_mtime(&path2, mtime2).unwrap();

        fs::write(&path3, "backup3").unwrap();
        let mtime3 = filetime::FileTime::from_unix_time(3000, 0);
        filetime::set_file_mtime(&path3, mtime3).unwrap();

        let backups = list_backups(dir.path()).unwrap();
        assert_eq!(backups.len(), 3);
        assert_eq!(
            backups[0].filename, "hosts-20240103_120000.bak",
            "newest backup should be first"
        );
        assert_eq!(
            backups[1].filename, "hosts-20240102_120000.bak",
            "middle backup should be second"
        );
        assert_eq!(
            backups[2].filename, "hosts-20240101_120000.bak",
            "oldest backup should be last"
        );
    }

    #[test]
    fn test_list_backups_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts-20240101_120000.bak");
        let content = "test backup content";
        fs::write(&path, content).unwrap();

        let backups = list_backups(dir.path()).unwrap();
        assert_eq!(backups.len(), 1);

        let info = &backups[0];
        assert_eq!(info.id, "hosts-20240101_120000.bak");
        assert_eq!(info.filename, "hosts-20240101_120000.bak");
        assert_eq!(info.size, content.len() as u64);
        assert!(info.path.contains("hosts-20240101_120000.bak"));
        assert!(!info.timestamp.is_empty());
    }
}
