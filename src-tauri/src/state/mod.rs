use mhost_apply::writer::HostsWriter;
use mhost_core::{MhostError, ProfileMode};
use mhost_storage::migration::migrate_v1_to_v2;
use mhost_storage::storage::{FileStorage, Storage};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

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

        // 清理上次退出残留的 signal / original DNS 文件（fix: proxy
        // self-cleanup）。如果 mhost 上次崩溃 / kill -9 没机会清理，
        // 这些 /tmp 文件会留下。下次启动时让下次启用的 enable 路径
        // 重新写（覆盖）。
        #[cfg(target_os = "macos")]
        {
            let _ = std::fs::remove_file("/tmp/mhost-dns-original.txt");
            let _ = std::fs::remove_file("/tmp/mhost-dns-shutdown.signal");
        }

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
                    eprintln!(
                        "[mHost] DNS auto-recovery failed: {}. Resetting dns_enabled to false.",
                        e
                    );
                    dns_enabled = false;
                    {
                        let mut updated_manifest = manifest.clone();
                        updated_manifest.dns_enabled = Some(false);
                        if let Err(e) = storage.save_manifest(&updated_manifest) {
                            eprintln!(
                                "[mHost] Failed to update manifest after DNS recovery failure: {}",
                                e
                            );
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
        // 1. 优先从 manifest.original_dns 恢复（避免再次问系统 —— 系统 DNS
        //    此时已经是 127.0.0.1，问到的也是错的）。若 manifest 没保存则
        //    fallback 到 get_system_dns。
        let manifest = storage.load_manifest()?;
        let mut original: Vec<String> = if let Some(saved) = &manifest.original_dns {
            saved.clone()
        } else {
            // legacy 路径：v2.0 没把 original_dns 持久化到 manifest。
            // 系统 DNS 此时大概率是 127.0.0.1（v2.0 写过的），
            // 不能把 127.0.0.1 当作「原始值」回写。
            mhost_dns::platform::get_system_dns()
                .map_err(|e| MhostError::InvalidInput(format!("get system dns failed: {}", e)))?
        };

        // 1.1 保护性回写：只把「不像 v2.0 残留」的 original 回写。
        //   - 如果 original 为空（用户在 v2.0 后没配过系统 DNS）→ 写空 vec
        //   - 如果 original 含 127.0.0.1（v2.0 留下来的伪 original）→
        //     之前是直接跳过；但这样 state.original_dns 是空、退出时
        //     cleanup 路径无法恢复（Pi-hole fallback 会拒绝）→ 用户
        //     永远卡在 127.0.0.1。
        //     修复：用 vec!["Empty"] 作为兜底（DHCP default），
        //     这样退出时 networksetup -setdnsservers <iface> Empty 能
        //     恢复 DHCP，Pi-hole 用户（少数场景）会丢失 Pi-hole 但
        //     至少能用互联网。
        let mut manifest = manifest;
        if manifest.original_dns.is_none() {
            let looks_like_v2_residue = original.iter().any(|s| s == "127.0.0.1" || s == "::1");
            if !looks_like_v2_residue {
                manifest.original_dns = Some(original.clone());
                if let Err(e) = storage.save_manifest(&manifest) {
                    eprintln!(
                        "[mHost] Failed to persist original_dns after recovery: {}",
                        e
                    );
                }
            } else {
                eprintln!(
                    "[mHost] Detected v2.0 residue (system DNS has 127.0.0.1); \
                     using DHCP default as fallback original for safe cleanup"
                );
                // 兜底：把 ["Empty"] 持久化，作为退出恢复目标。
                original = vec!["Empty".to_string()];
                manifest.original_dns = Some(original.clone());
                if let Err(e) = storage.save_manifest(&manifest) {
                    eprintln!("[mHost] Failed to persist fallback original_dns: {}", e);
                }
            }
        }

        // 2. 创建 DnsConfig 和 DnsServer（upstream 使用系统原始 DNS 兜底）
        //
        // 注意：original 可能等于 vec!["Empty"]（v2.0 残留兜底），
        // 这是个「恢复目标」placeholder，不能作为 upstream DNS。
        // 所以这里对 upstream 单独 fallback。
        let upstream = if original.is_empty() || original == vec!["Empty".to_string()] {
            vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()]
        } else {
            original.clone()
        };
        let dns_port = mhost_dns::MHOST_DNS_PORT;
        let config = mhost_dns::DnsConfig {
            port: dns_port,
            upstream,
            ..Default::default()
        };
        let server = mhost_dns::DnsServer::new(config)
            .map_err(|e| MhostError::InvalidInput(format!("dns server init failed: {}", e)))?;

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
        // fix（proxy self-cleanup）：把 original 传给 proxy，让它
        // 退出时能自己恢复系统 DNS。
        if let Err(e) = mhost_dns::platform::enable_dns_mode(dns_port, &original) {
            let _ = server.stop().await;
            return Err(MhostError::InvalidInput(format!(
                "Failed to enable DNS mode: {}",
                e
            )));
        }

        Ok((server, original))
    }
}
