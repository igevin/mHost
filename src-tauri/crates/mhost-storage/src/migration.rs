//! v1 → v2 数据迁移逻辑
//!
//! 迁移内容：
//! 1. 备份当前 root 目录到 `backup_pre_v2/`
//! 2. 将 `profiles/` 下所有 `.json` 文件移动到 `profiles/hosts/`
//! 3. 创建 `profiles/dns/` 空目录
//! 4. 更新 manifest.version = 2, manifest.dns_enabled = false

use std::fs;
use std::path::Path;

use chrono::Utc;
use mhost_core::StorageError;

use crate::storage::{FileStorage, Storage};

/// 检测并执行 v1 → v2 迁移。
///
/// 如果 manifest 不存在或 version >= 2，则不执行任何操作。
/// 返回 `Ok(true)` 表示执行了迁移，`Ok(false)` 表示无需迁移。
pub fn migrate_v1_to_v2(storage: &FileStorage) -> Result<bool, StorageError> {
    let manifest = match storage.load_manifest() {
        Ok(m) => m,
        Err(StorageError::ManifestCorrupted(_)) => {
            // manifest 不存在，视为全新安装，无需迁移
            return Ok(false);
        }
        Err(e) => return Err(e),
    };

    if manifest.version >= 2 {
        return Ok(false);
    }

    let root = storage.root();

    // 1. 备份当前 root 目录
    backup_root(root)?;

    // 2. 将 profiles/ 下所有 .json 文件移动到 profiles/hosts/
    move_profiles_to_hosts(root)?;

    // 3. 创建 profiles/dns/ 空目录
    let dns_dir = root.join("profiles").join("dns");
    fs::create_dir_all(&dns_dir)
        .map_err(|e| StorageError::Io(format!("创建 dns 目录失败: {}", e)))?;

    // 4. 更新 manifest
    let mut new_manifest = manifest;
    new_manifest.version = 2;
    new_manifest.dns_enabled = Some(false);
    new_manifest.original_dns = None;
    new_manifest.updated_at = Utc::now();

    // 5. 保存新 manifest
    storage.save_manifest(&new_manifest)?;

    Ok(true)
}

/// 备份 root 目录到 `backup_pre_v2/`（带时间戳后缀避免覆盖）。
fn backup_root(root: &Path) -> Result<(), StorageError> {
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%3f");
    let backup_name = format!("backup_pre_v2_{}", timestamp);
    let backup_path = root.parent().unwrap_or(root).join(&backup_name);

    // 如果 root 没有父目录（不太可能），直接在 root 同级创建备份
    let backup_target = if backup_path == *root {
        let file_name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "mhost_backup".to_string());
        root.with_file_name(format!("{}_backup", file_name))
    } else {
        backup_path
    };

    copy_dir_all(root, &backup_target)
        .map_err(|e| StorageError::Io(format!("备份目录失败: {}", e)))?;

    Ok(())
}

/// 将 profiles/ 下所有 .json 文件移动到 profiles/hosts/。
fn move_profiles_to_hosts(root: &Path) -> Result<(), StorageError> {
    let profiles_dir = root.join("profiles");
    if !profiles_dir.exists() {
        return Ok(());
    }

    let hosts_dir = profiles_dir.join("hosts");
    fs::create_dir_all(&hosts_dir)
        .map_err(|e| StorageError::Io(format!("创建 hosts 目录失败: {}", e)))?;

    for entry in fs::read_dir(&profiles_dir)
        .map_err(|e| StorageError::Io(format!("读取 profiles 目录失败: {}", e)))?
    {
        let entry = entry.map_err(|e| StorageError::Io(format!("遍历目录项失败: {}", e)))?;
        let path = entry.path();

        // 只移动 .json 文件，跳过子目录
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
            let file_name = path
                .file_name()
                .ok_or_else(|| StorageError::Io(format!("获取文件名失败: {}", path.display())))?;
            let target = hosts_dir.join(file_name);
            fs::rename(&path, &target).map_err(|e| {
                StorageError::Io(format!(
                    "移动文件失败 [{}] -> [{}]: {}",
                    path.display(),
                    target.display(),
                    e
                ))
            })?;
        }
    }

    Ok(())
}

/// 递归复制目录。
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest_path = dst.join(entry.file_name());

        if path.is_dir() {
            copy_dir_all(&path, &dest_path)?;
        } else {
            fs::copy(&path, &dest_path)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::storage::Storage;
    use mhost_core::{Profile, ProfileMode};
    use tempfile::TempDir;

    fn create_v1_storage(root: &Path) -> Result<FileStorage, StorageError> {
        let storage = FileStorage::new(root);

        // 创建 v1 manifest
        let v1_manifest = Manifest {
            version: 1,
            app_version: "0.1.0".to_string(),
            updated_at: chrono::Utc::now(),
            dns_enabled: None,
            original_dns: None,
        };
        storage.save_manifest(&v1_manifest)?;

        // 在 profiles/ 根目录下创建一些 Profile（v1 风格）
        let profiles_dir = root.join("profiles");
        fs::create_dir_all(&profiles_dir).unwrap();

        Ok(storage)
    }

    #[test]
    fn test_migrate_v1_to_v2_normal() {
        let temp_dir = TempDir::new().unwrap();
        let storage = create_v1_storage(temp_dir.path()).unwrap();

        // 创建一些 v1 Profile
        let profile1 = Profile::new("p1");
        let profile2 = Profile::new("p2");
        let old_path1 = temp_dir
            .path()
            .join("profiles")
            .join(format!("{}.json", profile1.id));
        let old_path2 = temp_dir
            .path()
            .join("profiles")
            .join(format!("{}.json", profile2.id));
        fs::write(&old_path1, serde_json::to_string_pretty(&profile1).unwrap()).unwrap();
        fs::write(&old_path2, serde_json::to_string_pretty(&profile2).unwrap()).unwrap();

        // 执行迁移
        let migrated = migrate_v1_to_v2(&storage).unwrap();
        assert!(migrated, "应执行迁移");

        // 验证 manifest 已更新
        let manifest = storage.load_manifest().unwrap();
        assert_eq!(manifest.version, 2);
        assert_eq!(manifest.dns_enabled, Some(false));
        assert_eq!(
            manifest.original_dns, None,
            "v1 迁移后 original_dns 应默认为 None"
        );

        // 验证 Profile 已移动到 hosts/ 子目录
        let new_path1 = temp_dir
            .path()
            .join("profiles")
            .join("hosts")
            .join(format!("{}.json", profile1.id));
        let new_path2 = temp_dir
            .path()
            .join("profiles")
            .join("hosts")
            .join(format!("{}.json", profile2.id));
        assert!(new_path1.exists(), "Profile1 应移动到 hosts/");
        assert!(new_path2.exists(), "Profile2 应移动到 hosts/");
        assert!(!old_path1.exists(), "旧路径不应再存在");
        assert!(!old_path2.exists(), "旧路径不应再存在");

        // 验证 dns/ 目录已创建
        let dns_dir = temp_dir.path().join("profiles").join("dns");
        assert!(dns_dir.exists() && dns_dir.is_dir());

        // 验证备份已创建
        let parent = temp_dir.path().parent().unwrap();
        let backup_dir = fs::read_dir(parent)
            .unwrap()
            .find(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("backup_pre_v2_")
            })
            .expect("应创建备份目录");
        assert!(backup_dir.unwrap().path().exists());
    }

    #[test]
    fn test_migrate_v1_to_v2_empty_profiles() {
        let temp_dir = TempDir::new().unwrap();
        let storage = create_v1_storage(temp_dir.path()).unwrap();

        // profiles/ 目录为空
        let migrated = migrate_v1_to_v2(&storage).unwrap();
        assert!(migrated, "应执行迁移");

        let manifest = storage.load_manifest().unwrap();
        assert_eq!(manifest.version, 2);

        let hosts_dir = temp_dir.path().join("profiles").join("hosts");
        let dns_dir = temp_dir.path().join("profiles").join("dns");
        assert!(hosts_dir.exists());
        assert!(dns_dir.exists());
    }

    #[test]
    fn test_migrate_v1_to_v2_no_manifest() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());

        // 没有 manifest，视为全新安装
        let migrated = migrate_v1_to_v2(&storage).unwrap();
        assert!(!migrated, "无 manifest 时不应执行迁移");
    }

    #[test]
    fn test_migrate_v1_to_v2_already_v2() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());

        // 直接创建 v2 manifest
        let v2_manifest = Manifest::new("0.2.0");
        storage.save_manifest(&v2_manifest).unwrap();

        let migrated = migrate_v1_to_v2(&storage).unwrap();
        assert!(!migrated, "v2 不应再执行迁移");
    }

    #[test]
    fn test_migrate_v1_to_v2_with_corrupted_profile() {
        let temp_dir = TempDir::new().unwrap();
        let storage = create_v1_storage(temp_dir.path()).unwrap();

        // 创建一个损坏的 JSON 文件
        let bad_file = temp_dir.path().join("profiles").join("bad.json");
        fs::write(&bad_file, "this is not json").unwrap();

        // 创建一个正常的 Profile
        let good_profile = Profile::new("good");
        let good_file = temp_dir
            .path()
            .join("profiles")
            .join(format!("{}.json", good_profile.id));
        fs::write(
            &good_file,
            serde_json::to_string_pretty(&good_profile).unwrap(),
        )
        .unwrap();

        // 执行迁移
        let migrated = migrate_v1_to_v2(&storage).unwrap();
        assert!(migrated, "应执行迁移");

        // 损坏的文件也应被移动到 hosts/
        let moved_bad = temp_dir
            .path()
            .join("profiles")
            .join("hosts")
            .join("bad.json");
        assert!(moved_bad.exists(), "损坏文件也应被移动");

        // 正常文件也应被移动
        let moved_good = temp_dir
            .path()
            .join("profiles")
            .join("hosts")
            .join(format!("{}.json", good_profile.id));
        assert!(moved_good.exists());
    }

    #[test]
    fn test_migrate_v1_to_v2_profile_backward_compatible() {
        let temp_dir = TempDir::new().unwrap();
        let storage = create_v1_storage(temp_dir.path()).unwrap();

        // 创建 v1 Profile（不含 mode 字段）
        let profile = Profile::new("legacy");
        let old_file = temp_dir
            .path()
            .join("profiles")
            .join(format!("{}.json", profile.id));
        fs::write(&old_file, serde_json::to_string_pretty(&profile).unwrap()).unwrap();

        // 执行迁移
        let migrated = migrate_v1_to_v2(&storage).unwrap();
        assert!(migrated);

        // 迁移后应能从 hosts/ 加载，且 mode 默认为 Hosts
        let loaded = storage.load_profile(&profile.id).unwrap();
        assert_eq!(loaded.mode, ProfileMode::Hosts);
        assert_eq!(loaded.name, "legacy");
    }
}
