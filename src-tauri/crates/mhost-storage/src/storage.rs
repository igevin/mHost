//! Storage trait and implementations

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use mhost_core::{Profile, ProfileId, StorageError};

use crate::manifest::Manifest;

// ---------------------------------------------------------------------------
// Storage trait
// ---------------------------------------------------------------------------

/// 存储抽象接口，定义 Profile 和 Manifest 的持久化操作。
pub trait Storage {
    /// 根据 ID 加载 Profile。
    fn load_profile(&self, id: &ProfileId) -> Result<Profile, StorageError>;
    /// 保存 Profile。
    fn save_profile(&self, profile: &Profile) -> Result<(), StorageError>;
    /// 删除 Profile。
    fn delete_profile(&self, id: &ProfileId) -> Result<(), StorageError>;
    /// 列出所有 Profile（损坏的文件会被静默跳过）。
    fn list_profiles(&self) -> Result<Vec<Profile>, StorageError>;
    /// 列出所有 Profile，同时返回解析过程中遇到的错误。
    fn list_profiles_with_errors(&self) -> Result<(Vec<Profile>, Vec<StorageError>), StorageError>;
    /// 加载 Manifest。
    fn load_manifest(&self) -> Result<Manifest, StorageError>;
    /// 保存 Manifest。
    fn save_manifest(&self, manifest: &Manifest) -> Result<(), StorageError>;
}

// ---------------------------------------------------------------------------
// FileStorage
// ---------------------------------------------------------------------------

/// 基于本地文件系统的存储实现。
///
/// 存储结构：
/// ```text
/// {root}/
///   manifest.json
///   profiles/
///     {profile_id}.json
///   backups/
///   settings.json
/// ```
pub struct FileStorage {
    root: PathBuf,
}

impl FileStorage {
    /// 使用默认系统数据目录创建存储（macOS: `~/Library/Application Support/mHost`）。
    #[allow(clippy::should_implement_trait)]
    pub fn default() -> Result<Self, StorageError> {
        let root = dirs::data_dir()
            .ok_or_else(|| StorageError::Io("无法获取系统数据目录".to_string()))?
            .join("mHost");
        Ok(Self::new(&root))
    }

    /// 使用指定根目录创建存储（测试时常用）。
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// 返回 profiles 目录路径。
    fn profiles_dir(&self) -> PathBuf {
        self.root.join("profiles")
    }

    /// 返回指定 Profile ID 对应的文件路径。
    fn profile_path(&self, id: &ProfileId) -> PathBuf {
        self.profiles_dir().join(format!("{}.json", id))
    }

    /// 返回 manifest 文件路径。
    fn manifest_path(&self) -> PathBuf {
        self.root.join("manifest.json")
    }

    /// 确保目录存在。
    fn ensure_dir(&self, path: &Path) -> Result<(), StorageError> {
        if !path.exists() {
            fs::create_dir_all(path)
                .map_err(|e| StorageError::Io(format!("创建目录失败: {}", e)))?;
        }
        Ok(())
    }
}

impl Storage for FileStorage {
    fn load_profile(&self, id: &ProfileId) -> Result<Profile, StorageError> {
        let path = self.profile_path(id);
        if !path.exists() {
            return Err(StorageError::ProfileNotFound(id.clone()));
        }
        let content = fs::read_to_string(&path)
            .map_err(|e| StorageError::Io(format!("读取 Profile 失败: {}", e)))?;
        let profile: Profile = serde_json::from_str(&content).map_err(|e| {
            StorageError::ManifestCorrupted(format!("解析 Profile JSON 失败: {}", e))
        })?;
        Ok(profile)
    }

    fn save_profile(&self, profile: &Profile) -> Result<(), StorageError> {
        self.ensure_dir(&self.profiles_dir())?;
        let path = self.profile_path(&profile.id);
        let json = serde_json::to_string_pretty(profile)
            .map_err(|e| StorageError::ManifestCorrupted(format!("序列化 Profile 失败: {}", e)))?;
        atomic_write(&path, json.as_bytes())
            .map_err(|e| StorageError::Io(format!("写入 Profile 失败: {}", e)))?;
        Ok(())
    }

    fn delete_profile(&self, id: &ProfileId) -> Result<(), StorageError> {
        let path = self.profile_path(id);
        if !path.exists() {
            return Err(StorageError::ProfileNotFound(id.clone()));
        }
        fs::remove_file(&path)
            .map_err(|e| StorageError::Io(format!("删除 Profile 失败: {}", e)))?;
        Ok(())
    }

    fn list_profiles(&self) -> Result<Vec<Profile>, StorageError> {
        let (profiles, _errors) = self.list_profiles_with_errors()?;
        Ok(profiles)
    }

    fn list_profiles_with_errors(&self) -> Result<(Vec<Profile>, Vec<StorageError>), StorageError> {
        let dir = self.profiles_dir();
        if !dir.exists() {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut profiles = Vec::new();
        let mut errors = Vec::new();
        for entry in fs::read_dir(&dir)
            .map_err(|e| StorageError::Io(format!("读取 profiles 目录失败: {}", e)))?
        {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    errors.push(StorageError::Io(format!("遍历目录项失败: {}", e)));
                    continue;
                }
            };
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                let content = match fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(e) => {
                        errors.push(StorageError::Io(format!(
                            "读取 Profile 文件失败 [{}]: {}",
                            path.display(),
                            e
                        )));
                        continue;
                    }
                };
                let profile: Profile = match serde_json::from_str(&content) {
                    Ok(p) => p,
                    Err(e) => {
                        errors.push(StorageError::ManifestCorrupted(format!(
                            "解析 Profile JSON 失败 [{}]: {}",
                            path.display(),
                            e
                        )));
                        continue;
                    }
                };
                profiles.push(profile);
            }
        }
        Ok((profiles, errors))
    }

    fn load_manifest(&self) -> Result<Manifest, StorageError> {
        let path = self.manifest_path();
        if !path.exists() {
            return Err(StorageError::ManifestCorrupted(
                "manifest.json 不存在".to_string(),
            ));
        }
        let content = fs::read_to_string(&path)
            .map_err(|e| StorageError::Io(format!("读取 manifest 失败: {}", e)))?;
        let manifest: Manifest = serde_json::from_str(&content).map_err(|e| {
            StorageError::ManifestCorrupted(format!("解析 manifest JSON 失败: {}", e))
        })?;
        Ok(manifest)
    }

    fn save_manifest(&self, manifest: &Manifest) -> Result<(), StorageError> {
        self.ensure_dir(&self.root)?;
        let path = self.manifest_path();
        let json = serde_json::to_string_pretty(manifest)
            .map_err(|e| StorageError::ManifestCorrupted(format!("序列化 manifest 失败: {}", e)))?;
        atomic_write(&path, json.as_bytes())
            .map_err(|e| StorageError::Io(format!("写入 manifest 失败: {}", e)))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Atomic write helper
// ---------------------------------------------------------------------------

/// 原子写入：先写入 `.tmp` 临时文件，再通过 `fs::rename` 替换目标文件。
///
/// 如果写入过程中发生错误，会尝试清理临时文件。
fn atomic_write(path: &Path, content: &[u8]) -> io::Result<()> {
    let temp = path.with_extension("tmp");
    match fs::write(&temp, content) {
        Ok(()) => {
            fs::rename(&temp, path)?;
            Ok(())
        }
        Err(e) => {
            // 尝试清理临时文件，忽略清理失败
            let _ = fs::remove_file(&temp);
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mhost_core::HostRule;
    use std::net::IpAddr;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // Helper
    // -----------------------------------------------------------------------

    fn create_test_storage() -> (TempDir, FileStorage) {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());
        (temp_dir, storage)
    }

    fn profile_with_rules(name: &str, rules: Vec<(IpAddr, Vec<String>)>) -> Profile {
        let mut profile = Profile::new(name);
        for (ip, domains) in rules {
            profile.rules.push(HostRule::new(ip, domains));
        }
        profile
    }

    // -----------------------------------------------------------------------
    // Profile CRUD
    // -----------------------------------------------------------------------

    #[test]
    fn test_save_and_load_profile() {
        let (_temp, storage) = create_test_storage();
        let profile = Profile::new("test");

        storage.save_profile(&profile).unwrap();
        let loaded = storage.load_profile(&profile.id).unwrap();

        assert_eq!(profile, loaded);
    }

    #[test]
    fn test_save_and_load_profile_with_rules() {
        let (_temp, storage) = create_test_storage();
        let profile = profile_with_rules(
            "dev",
            vec![
                (
                    "127.0.0.1".parse().unwrap(),
                    vec!["a.com".to_string(), "b.com".to_string()],
                ),
                ("::1".parse().unwrap(), vec!["localhost".to_string()]),
            ],
        );

        storage.save_profile(&profile).unwrap();
        let loaded = storage.load_profile(&profile.id).unwrap();

        assert_eq!(profile, loaded);
        assert_eq!(loaded.rules.len(), 2);
        assert_eq!(loaded.rules[0].domains, vec!["a.com", "b.com"]);
        assert_eq!(loaded.rules[1].ip, "::1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn test_list_profiles() {
        let (_temp, storage) = create_test_storage();
        let cases = vec![
            ("p1", Profile::new("profile_1")),
            ("p2", Profile::new("profile_2")),
            ("p3", Profile::new("profile_3")),
        ];

        for (_name, profile) in &cases {
            storage.save_profile(profile).unwrap();
        }

        let listed = storage.list_profiles().unwrap();
        assert_eq!(listed.len(), cases.len());

        // 验证每个保存的 profile 都在列表中
        for (_name, profile) in &cases {
            assert!(
                listed.iter().any(|p| p.id == profile.id),
                "profile {} should be listed",
                profile.name
            );
        }
    }

    #[test]
    fn test_list_profiles_empty() {
        let (_temp, storage) = create_test_storage();
        let listed = storage.list_profiles().unwrap();
        assert!(listed.is_empty());
    }

    #[test]
    fn test_delete_profile() {
        let (_temp, storage) = create_test_storage();
        let profile = Profile::new("to_delete");

        storage.save_profile(&profile).unwrap();
        let loaded = storage.load_profile(&profile.id).unwrap();
        assert_eq!(profile.id, loaded.id);

        storage.delete_profile(&profile.id).unwrap();
        let result = storage.load_profile(&profile.id);
        assert!(
            matches!(result, Err(StorageError::ProfileNotFound(_))),
            "删除后应返回 ProfileNotFound"
        );
    }

    #[test]
    fn test_delete_profile_not_found() {
        let (_temp, storage) = create_test_storage();
        let id = ProfileId(uuid::Uuid::new_v4());
        let result = storage.delete_profile(&id);
        assert!(
            matches!(result, Err(StorageError::ProfileNotFound(_))),
            "删除不存在的 Profile 应返回 ProfileNotFound"
        );
    }

    #[test]
    fn test_load_profile_not_found() {
        let (_temp, storage) = create_test_storage();
        let id = ProfileId(uuid::Uuid::new_v4());
        let result = storage.load_profile(&id);
        assert!(
            matches!(result, Err(StorageError::ProfileNotFound(_))),
            "加载不存在的 Profile 应返回 ProfileNotFound"
        );
    }

    // -----------------------------------------------------------------------
    // Atomic write
    // -----------------------------------------------------------------------

    #[test]
    fn test_atomic_write_creates_target() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("target.json");
        let content = b"hello world";

        atomic_write(&target, content).unwrap();

        assert!(target.exists());
        let read = fs::read_to_string(&target).unwrap();
        assert_eq!(read, "hello world");
        // 临时文件不应残留
        let temp = target.with_extension("tmp");
        assert!(!temp.exists());
    }

    #[test]
    fn test_atomic_write_replaces_existing() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("target.json");
        fs::write(&target, "old content").unwrap();

        atomic_write(&target, b"new content").unwrap();

        let read = fs::read_to_string(&target).unwrap();
        assert_eq!(read, "new content");
    }

    #[test]
    fn test_atomic_write_cleans_up_temp_on_success() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("target.json");
        fs::write(&target, "original").unwrap();

        atomic_write(&target, b"updated").unwrap();

        let read = fs::read_to_string(&target).unwrap();
        assert_eq!(read, "updated");
        assert!(!target.with_extension("tmp").exists());
    }

    #[test]
    fn test_atomic_write_cleans_up_temp_on_failure() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("target.json");
        fs::write(&target, "original").unwrap();

        // 构造一个 rename 会失败的场景：目标路径是一个已存在的目录。
        // atomic_write 会先成功写入 .tmp 文件，但 fs::rename 到目录时会失败。
        let blocking_dir = temp_dir.path().join("blocking_dir");
        fs::create_dir(&blocking_dir).unwrap();
        // 将 blocking_dir 移动到目标位置，使目标路径成为一个目录
        let target_as_dir = temp_dir.path().join("target.json");
        fs::remove_file(&target_as_dir).unwrap();
        fs::rename(&blocking_dir, &target_as_dir).unwrap();

        // 原子写入应该失败（因为无法将文件 rename 到一个已存在的目录）
        let result = atomic_write(&target_as_dir, b"updated");
        assert!(result.is_err(), "原子写入应该失败");

        // 验证目标路径仍然是目录，未被破坏为文件
        assert!(target_as_dir.is_dir(), "目标路径应保持为目录，不应被破坏");
    }

    // -----------------------------------------------------------------------
    // Manifest
    // -----------------------------------------------------------------------

    #[test]
    fn test_manifest_version() {
        let (_temp, storage) = create_test_storage();
        let manifest = Manifest::new("0.1.0");

        storage.save_manifest(&manifest).unwrap();
        let loaded = storage.load_manifest().unwrap();

        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.app_version, "0.1.0");
        assert_eq!(manifest.updated_at, loaded.updated_at);
    }

    #[test]
    fn test_manifest_load_not_found() {
        let (_temp, storage) = create_test_storage();
        let result = storage.load_manifest();
        assert!(
            matches!(result, Err(StorageError::ManifestCorrupted(_))),
            "manifest 不存在时应返回错误"
        );
    }

    #[test]
    fn test_manifest_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());
        let manifest = Manifest::new("0.2.0");

        storage.save_manifest(&manifest).unwrap();

        // 用新的 FileStorage 实例读取，验证持久化
        let storage2 = FileStorage::new(temp_dir.path());
        let loaded = storage2.load_manifest().unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.app_version, "0.2.0");
    }

    // -----------------------------------------------------------------------
    // Storage directory structure
    // -----------------------------------------------------------------------

    #[test]
    fn test_storage_creates_directories() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());
        let profile = Profile::new("test");

        storage.save_profile(&profile).unwrap();

        let profiles_dir = temp_dir.path().join("profiles");
        assert!(profiles_dir.exists());
        assert!(profiles_dir.is_dir());
    }

    #[test]
    fn test_profile_file_naming() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());
        let profile = Profile::new("test");

        storage.save_profile(&profile).unwrap();

        let expected_path = temp_dir
            .path()
            .join("profiles")
            .join(format!("{}.json", profile.id));
        assert!(expected_path.exists());
    }
}
