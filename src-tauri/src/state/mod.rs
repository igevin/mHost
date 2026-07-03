use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use mhost_storage::migration::migrate_v1_to_v2;
use mhost_storage::storage::{FileStorage, Storage};
use mhost_apply::writer::HostsWriter;
use mhost_core::{MhostError, ProfileMode};

/// Async mutex to serialize apply operations and prevent concurrent writes to /etc/hosts.
/// Security fix (#16): Prevents race conditions when user rapidly toggles profiles.
/// Perf fix (#26): Changed to tokio::sync::Mutex to allow holding across await points.
/// Note: tokio::sync::Mutex does not have poison recovery like std::sync::Mutex.
/// If a spawn_blocking task panics while holding the lock, the lock is released
/// automatically (tokio::sync::Mutex is not poisoned), so recovery is implicit.
pub struct ApplyLock(pub tokio::sync::Mutex<()>);

impl Default for ApplyLock {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplyLock {
    pub fn new() -> Self {
        Self(tokio::sync::Mutex::new(()))
    }

    /// Acquire the lock asynchronously.
    pub async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.0.lock().await
    }

    /// Acquire the lock in a blocking context (e.g., `spawn_blocking`).
    pub fn blocking_lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.0.blocking_lock()
    }
}

pub struct AppState {
    pub storage: Arc<dyn Storage + Send + Sync>,
    pub writer: Arc<HostsWriter>,
    pub apply_lock: ApplyLock,
    /// N2: Serialize snapshot save/delete operations to prevent races.
    pub snapshot_lock: ApplyLock,
    /// Perf fix (#29): Track last rendered profile IDs to avoid unnecessary menu rebuilds.
    pub last_profile_ids: Mutex<Vec<String>>,
    // DNS 相关
    pub dns_server: Arc<Mutex<Option<mhost_dns::DnsServer>>>,
    pub dns_enabled: AtomicBool,
    pub original_dns: Mutex<Vec<String>>,
    /// 串行化 DNS 模式切换操作。
    pub dns_lock: ApplyLock,
}

impl AppState {
    pub async fn new() -> Result<Self, MhostError> {
        let file_storage = FileStorage::default()?;

        // 清理上次可能残留的 dns-proxy 进程（macOS）
        #[cfg(target_os = "macos")]
        mhost_dns::platform::cleanup_stale_proxy();

        // v1 → v2 数据迁移：失败记录错误日志，不阻断应用启动
        if let Ok(fs) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            migrate_v1_to_v2(&file_storage)
        })) {
            match fs {
                Ok(true) => eprintln!("[mHost] v1 → v2 data migration completed successfully."),
                Ok(false) => {}
                Err(e) => eprintln!("[mHost] v1 → v2 data migration failed: {}", e),
            }
        } else {
            eprintln!("[mHost] v1 → v2 data migration panicked, continuing startup.");
        }

        let storage = Arc::new(file_storage);
        let writer = Arc::new(HostsWriter::new());

        // 从 manifest 恢复 DNS 模式状态（不存在则创建默认）
        let manifest = match storage.load_manifest() {
            Ok(m) => m,
            Err(_) => {
                let default = mhost_storage::manifest::Manifest::new(env!("CARGO_PKG_VERSION"));
                let _ = storage.save_manifest(&default);
                default
            }
        };
        let mut dns_enabled = manifest.dns_enabled.unwrap_or(false);
        let mut dns_server_opt: Option<mhost_dns::DnsServer> = None;
        let mut original_dns = Vec::new();

        // 如果上次退出时 DNS 处于启用状态，尝试自动恢复 DNS 服务
        if dns_enabled {
            match Self::try_recover_dns(storage.clone()).await {
                Ok((server, original)) => {
                    dns_server_opt = Some(server);
                    original_dns = original;
                    eprintln!("[mHost] DNS service auto-recovered successfully.");
                }
                Err(e) => {
                    eprintln!("[mHost] DNS auto-recovery failed: {}. Resetting dns_enabled to false.", e);
                    dns_enabled = false;
                    {
                        let mut updated_manifest = manifest.clone();
                        updated_manifest.dns_enabled = Some(false);
                        if let Err(e) = storage.save_manifest(&updated_manifest) {
                            eprintln!("[mHost] Failed to update manifest after DNS recovery failure: {}", e);
                        }
                    }
                }
            }
        }

        Ok(Self {
            storage,
            writer,
            apply_lock: ApplyLock(tokio::sync::Mutex::new(())),
            snapshot_lock: ApplyLock(tokio::sync::Mutex::new(())),
            last_profile_ids: Mutex::new(Vec::new()),
            dns_server: Arc::new(Mutex::new(dns_server_opt)),
            dns_enabled: AtomicBool::new(dns_enabled),
            original_dns: Mutex::new(original_dns),
            dns_lock: ApplyLock(tokio::sync::Mutex::new(())),
        })
    }

    /// 尝试自动恢复 DNS 服务。
    /// 返回 (DnsServer, original_dns) 若成功。
    async fn try_recover_dns(
        storage: Arc<dyn Storage + Send + Sync>,
    ) -> Result<(mhost_dns::DnsServer, Vec<String>), MhostError> {
        // 1. 获取当前系统 DNS 并保存
        let original = mhost_dns::platform::get_system_dns()
            .map_err(|e| MhostError::InvalidInput(format!("get system dns failed: {}", e)))?;

        // 2. 创建 DnsConfig 和 DnsServer（upstream 使用系统原始 DNS 兜底）
        let upstream = if original.is_empty() {
            vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()]
        } else {
            original.clone()
        };
        let dns_port = {
            #[cfg(target_os = "macos")]
            { 1053u16 }
            #[cfg(not(target_os = "macos"))]
            { 53u16 }
        };
        let config = mhost_dns::DnsConfig {
            port: dns_port,
            upstream,
            ..Default::default()
        };
        let server = mhost_dns::DnsServer::new(config);

        // 3. 加载所有 enabled 的 DNS 模式 Profile，reload_rules
        let profiles = storage
            .list_profiles_by_mode(ProfileMode::Dns)
            .map_err(MhostError::from)?;
        let enabled_profiles: Vec<_> = profiles.into_iter().filter(|p| p.enabled).collect();
        server.reload_rules(&enabled_profiles);

        // 4. 启动 DnsServer（spawn 到后台）
        server
            .start()
            .await
            .map_err(|e| MhostError::InvalidInput(format!("dns server start failed: {}", e)))?;

        // 5. 启动 dns-proxy 并设置系统 DNS
        if let Err(e) = mhost_dns::platform::enable_dns_mode(dns_port) {
            let _ = server.stop().await;
            return Err(MhostError::InvalidInput(format!(
                "Failed to enable DNS mode: {}",
                e
            )));
        }

        Ok((server, original))
    }
}
