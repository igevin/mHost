//! Backup management for the hosts writer
//!
//! Handles creating timestamped backups of the hosts file and pruning
//! old backups when the count exceeds the configured maximum.

use chrono::Utc;
use mhost_core::MhostError;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Maximum number of backup files to retain.
const MAX_BACKUPS: usize = 10;

/// Create a timestamped backup of the given content.
///
/// After creating the backup, enforces the maximum backup limit by
/// removing the oldest backups if the count exceeds `MAX_BACKUPS`.
///
/// **fix（H2, issue #90）**：backup 文件用 `OpenOptions` + `mode(0o600)` 写，
/// 不再用 `fs::write` 默认 umask（macOS 上是 0o644）。backups 可能含内部
/// dev/staging 主机名 + ad-block patterns，多用户机器上其他本地用户可
/// 能读取 → 收紧权限。
pub fn create_backup(backup_dir: &Path, content: &str) -> Result<PathBuf, MhostError> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::create_dir_all(backup_dir)?;
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let path = backup_dir.join(format!("hosts-{}.bak", timestamp));
    // mode(0o600) owner-only — 备份内容含敏感内部主机名
    // create(true).truncate(true)：timestamp 到秒级，同秒内多次备份
    // （如测试场景或快速连续写入）允许覆盖同名文件；生产中同一秒两次
    // 备份几乎不可能，但 truncate 兜底更稳。
    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        f.write_all(content.as_bytes())?;
        f.flush()?;
    }

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
                format!(
                    "failed to prune old backups: {} deletions failed",
                    failed_count
                ),
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    /// 单元测试（fix H2, issue #90）：backup 文件创建后必须是 0o600 权限，
    /// 不能用 fs::write 默认 umask（macOS 上是 0o644，多用户机器可读）。
    #[test]
    fn test_backup_file_created_with_0o600() {
        let dir = tempfile::tempdir().unwrap();
        let content = "127.0.0.1 internal-api.corp.example.com\n";

        let path = create_backup(dir.path(), content).expect("create_backup 失败");

        // 关键断言：权限 = 0o600（owner read/write, 其他无权限）
        let meta = fs::metadata(&path).expect("stat 失败");
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "backup 文件权限应为 0o600（owner-only），实际 0o{:o}",
            mode
        );

        // 内容一致（确认功能未坏）
        let read_back = fs::read_to_string(&path).expect("read 失败");
        assert_eq!(read_back, content);
    }

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
}
