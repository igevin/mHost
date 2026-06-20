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
    for entry in backups.iter().take(to_remove) {
        let _ = fs::remove_file(entry.path());
    }

    Ok(())
}
