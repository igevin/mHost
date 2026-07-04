//! Storage trait and implementations

use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

use mhost_core::{Profile, ProfileId, ProfileMode, StorageError};

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
    /// 列出所有 Profile（向后兼容，默认返回 hosts 模式）。
    fn list_profiles(&self) -> Result<Vec<Profile>, StorageError>;
    /// 列出所有 Profile，同时返回解析过程中遇到的错误。
    fn list_profiles_with_errors(&self) -> Result<(Vec<Profile>, Vec<StorageError>), StorageError>;
    /// 按模式列出 Profile。
    fn list_profiles_by_mode(&self, mode: ProfileMode) -> Result<Vec<Profile>, StorageError>;
    /// 列出所有 Profile（跨模式）。
    fn list_all_profiles(&self) -> Result<Vec<Profile>, StorageError>;
    /// 加载 Manifest。
    fn load_manifest(&self) -> Result<Manifest, StorageError>;
    /// 保存 Manifest。
    fn save_manifest(&self, manifest: &Manifest) -> Result<(), StorageError>;
    /// 返回存储根目录路径。
    fn root(&self) -> &Path;
}

// ---------------------------------------------------------------------------
// FileStorage
// ---------------------------------------------------------------------------

/// 基于本地文件系统的存储实现。
///
/// 存储结构（v2）：
/// ```text
/// {root}/
///   manifest.json
///   profiles/
///     hosts/
///       {profile_id}.json
///     dns/
///       {profile_id}.json
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

    /// 返回存储根目录路径。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 返回指定模式对应的 profiles 子目录路径。
    fn profiles_dir_for_mode(&self, mode: ProfileMode) -> PathBuf {
        match mode {
            ProfileMode::Hosts => self.root.join("profiles").join("hosts"),
            ProfileMode::Dns => self.root.join("profiles").join("dns"),
        }
    }

    /// 返回指定 Profile ID 和模式对应的文件路径。
    fn profile_path_for_mode(&self, id: &ProfileId, mode: ProfileMode) -> PathBuf {
        self.profiles_dir_for_mode(mode)
            .join(format!("{}.json", id))
    }

    /// 遍历 hosts 和 dns 子目录，查找指定 ID 的 Profile 文件路径。
    fn find_profile_path(&self, id: &ProfileId) -> Option<PathBuf> {
        for mode in [ProfileMode::Hosts, ProfileMode::Dns] {
            let path = self.profile_path_for_mode(id, mode);
            if path.exists() {
                return Some(path);
            }
        }
        None
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

    /// 读取指定目录下的所有 Profile 文件，返回 (profiles, errors)。
    fn read_profiles_from_dir(
        &self,
        dir: &Path,
    ) -> Result<(Vec<Profile>, Vec<StorageError>), StorageError> {
        if !dir.exists() {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut profiles = Vec::new();
        let mut errors = Vec::new();
        for entry in fs::read_dir(dir)
            .map_err(|e| StorageError::Io(format!("读取目录失败 [{}]: {}", dir.display(), e)))?
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
}

impl Storage for FileStorage {
    fn load_profile(&self, id: &ProfileId) -> Result<Profile, StorageError> {
        let path = self
            .find_profile_path(id)
            .ok_or_else(|| StorageError::ProfileNotFound(id.clone()))?;
        let content = fs::read_to_string(&path)
            .map_err(|e| StorageError::Io(format!("读取 Profile 失败: {}", e)))?;
        let profile: Profile = serde_json::from_str(&content).map_err(|e| {
            StorageError::ManifestCorrupted(format!("解析 Profile JSON 失败: {}", e))
        })?;
        Ok(profile)
    }

    fn save_profile(&self, profile: &Profile) -> Result<(), StorageError> {
        let dir = self.profiles_dir_for_mode(profile.mode);
        self.ensure_dir(&dir)?;
        let path = self.profile_path_for_mode(&profile.id, profile.mode);
        let json = serde_json::to_string_pretty(profile)
            .map_err(|e| StorageError::ManifestCorrupted(format!("序列化 Profile 失败: {}", e)))?;
        atomic_write(&path, json.as_bytes())
            .map_err(|e| StorageError::Io(format!("写入 Profile 失败: {}", e)))?;
        Ok(())
    }

    fn delete_profile(&self, id: &ProfileId) -> Result<(), StorageError> {
        let path = self
            .find_profile_path(id)
            .ok_or_else(|| StorageError::ProfileNotFound(id.clone()))?;
        fs::remove_file(&path)
            .map_err(|e| StorageError::Io(format!("删除 Profile 失败: {}", e)))?;
        Ok(())
    }

    fn list_profiles(&self) -> Result<Vec<Profile>, StorageError> {
        // 向后兼容：默认返回 hosts 模式
        self.list_profiles_by_mode(ProfileMode::Hosts)
    }

    fn list_profiles_with_errors(&self) -> Result<(Vec<Profile>, Vec<StorageError>), StorageError> {
        // 向后兼容：默认返回 hosts 模式
        let dir = self.profiles_dir_for_mode(ProfileMode::Hosts);
        self.read_profiles_from_dir(&dir)
    }

    fn list_profiles_by_mode(&self, mode: ProfileMode) -> Result<Vec<Profile>, StorageError> {
        let dir = self.profiles_dir_for_mode(mode);
        let (profiles, _errors) = self.read_profiles_from_dir(&dir)?;
        Ok(profiles)
    }

    fn list_all_profiles(&self) -> Result<Vec<Profile>, StorageError> {
        let mut all_profiles = Vec::new();
        for mode in [ProfileMode::Hosts, ProfileMode::Dns] {
            let profiles = self.list_profiles_by_mode(mode)?;
            all_profiles.extend(profiles);
        }
        Ok(all_profiles)
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

    fn root(&self) -> &Path {
        &self.root
    }
}

// ---------------------------------------------------------------------------
// Atomic write helper
// ---------------------------------------------------------------------------

/// 原子写入：使用 `tempfile::NamedTempFile` 生成唯一临时文件，再通过 `fs::rename` 替换目标文件。
///
/// 使用 `NamedTempFile` 避免并发写入时的固定临时文件名竞态条件。
/// 如果写入过程中发生错误，临时文件会自动清理。
fn atomic_write(path: &Path, content: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temp_file = tempfile::NamedTempFile::new_in(parent)?;
    temp_file.write_all(content)?;
    temp_file.flush()?;

    let temp_path = temp_file.into_temp_path();
    fs::rename(&temp_path, path)?;

    Ok(())
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
        assert_eq!(loaded.rules[1].ip, Some("::1".parse::<IpAddr>().unwrap()));
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

        assert_eq!(loaded.version, 2);
        assert_eq!(loaded.app_version, "0.1.0");
        assert_eq!(loaded.dns_enabled, Some(false));
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
        assert_eq!(loaded.version, 2);
        assert_eq!(loaded.app_version, "0.2.0");
        assert_eq!(loaded.dns_enabled, Some(false));
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

        let hosts_dir = temp_dir.path().join("profiles").join("hosts");
        assert!(hosts_dir.exists());
        assert!(hosts_dir.is_dir());
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
            .join("hosts")
            .join(format!("{}.json", profile.id));
        assert!(expected_path.exists());
    }

    // -----------------------------------------------------------------------
    // Profile mode separation
    // -----------------------------------------------------------------------

    #[test]
    fn test_save_and_load_dns_profile() {
        let (_temp, storage) = create_test_storage();
        let mut profile = Profile::new("dns_test");
        profile.mode = ProfileMode::Dns;

        storage.save_profile(&profile).unwrap();
        let loaded = storage.load_profile(&profile.id).unwrap();

        assert_eq!(profile, loaded);
        assert_eq!(loaded.mode, ProfileMode::Dns);
    }

    #[test]
    fn test_list_profiles_by_mode() {
        let (_temp, storage) = create_test_storage();
        let mut dns_profile = Profile::new("dns_profile");
        dns_profile.mode = ProfileMode::Dns;
        let hosts_profile = Profile::new("hosts_profile");

        storage.save_profile(&dns_profile).unwrap();
        storage.save_profile(&hosts_profile).unwrap();

        let hosts_listed = storage.list_profiles_by_mode(ProfileMode::Hosts).unwrap();
        assert_eq!(hosts_listed.len(), 1);
        assert_eq!(hosts_listed[0].id, hosts_profile.id);

        let dns_listed = storage.list_profiles_by_mode(ProfileMode::Dns).unwrap();
        assert_eq!(dns_listed.len(), 1);
        assert_eq!(dns_listed[0].id, dns_profile.id);
    }

    #[test]
    fn test_list_all_profiles() {
        let (_temp, storage) = create_test_storage();
        let mut dns_profile = Profile::new("dns_profile");
        dns_profile.mode = ProfileMode::Dns;
        let hosts_profile = Profile::new("hosts_profile");

        storage.save_profile(&dns_profile).unwrap();
        storage.save_profile(&hosts_profile).unwrap();

        let all = storage.list_all_profiles().unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|p| p.id == hosts_profile.id));
        assert!(all.iter().any(|p| p.id == dns_profile.id));
    }

    #[test]
    fn test_list_profiles_default_hosts() {
        let (_temp, storage) = create_test_storage();
        let mut dns_profile = Profile::new("dns_profile");
        dns_profile.mode = ProfileMode::Dns;
        let hosts_profile = Profile::new("hosts_profile");

        storage.save_profile(&dns_profile).unwrap();
        storage.save_profile(&hosts_profile).unwrap();

        // list_profiles 默认返回 hosts 模式（向后兼容）
        let listed = storage.list_profiles().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, hosts_profile.id);
    }

    #[test]
    fn test_delete_dns_profile() {
        let (_temp, storage) = create_test_storage();
        let mut profile = Profile::new("to_delete_dns");
        profile.mode = ProfileMode::Dns;

        storage.save_profile(&profile).unwrap();
        let loaded = storage.load_profile(&profile.id).unwrap();
        assert_eq!(profile.id, loaded.id);

        storage.delete_profile(&profile.id).unwrap();
        let result = storage.load_profile(&profile.id);
        assert!(
            matches!(result, Err(StorageError::ProfileNotFound(_))),
            "删除 DNS Profile 后应返回 ProfileNotFound"
        );
    }

    #[test]
    fn test_load_profile_cross_mode() {
        let (_temp, storage) = create_test_storage();
        let mut dns_profile = Profile::new("dns_cross");
        dns_profile.mode = ProfileMode::Dns;

        storage.save_profile(&dns_profile).unwrap();

        // 不指定模式也能从 dns 目录加载
        let loaded = storage.load_profile(&dns_profile.id).unwrap();
        assert_eq!(loaded.mode, ProfileMode::Dns);
    }

    #[test]
    fn test_list_profiles_empty_mode_dirs() {
        let (_temp, storage) = create_test_storage();

        let hosts = storage.list_profiles_by_mode(ProfileMode::Hosts).unwrap();
        let dns = storage.list_profiles_by_mode(ProfileMode::Dns).unwrap();
        let all = storage.list_all_profiles().unwrap();

        assert!(hosts.is_empty());
        assert!(dns.is_empty());
        assert!(all.is_empty());
    }
}
