use mhost_apply::writer::HostsWriter;
use mhost_core::{MhostError, OriginalDns, ProfileMode};
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
    /// 启用 DNS 模式时捕获的 snapshot（语义版本）。
    /// **fix（disabling-after-network-switch）**：原 `Vec<String>` 没有
    /// 「manual vs DHCP」的区分，导致 disable 时把 DHCP 推的 IP 错误
    /// 回写到系统 DNS。现在用 `OriginalDns` 区分，DhcpEmpty 写 Empty
    /// （= DHCP default）。
    pub original_dns: Mutex<OriginalDns>,
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
        let mut original_dns = OriginalDns::DhcpEmpty;

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
    ) -> Result<(mhost_dns::DnsServer, OriginalDns), MhostError> {
        // fix (bug 2): 如果上次退出时留下恢复标记，proxy 之前没正常退出。
        // 强制再走一次 `networksetup -setdnsservers <iface> Empty`（DHCP 默认）
        // 兜底，文件清理掉。osascript sudo 弹窗**只在异常路径**出现：
        // 正常退出 proxy 自己恢复了，标记文件被删，到不了这里。
        #[cfg(target_os = "macos")]
        {
            if std::path::Path::new("/tmp/mhost-dns-disable-recovery.marker").exists() {
                eprintln!(
                    "[mHost] try_recover_dns: disable recovery marker found, forcing restore"
                );
                if let Err(e) = mhost_dns::platform::force_dns_restore_if_needed() {
                    eprintln!("[mHost] force restore failed: {}", e);
                }
            }
        }
        // 1. 优先从 manifest.original_dns 恢复（避免再次问系统 —— 系统 DNS
        //    此时已经是 127.0.0.1，问到的也是错的）。若 manifest 没保存则
        //    fallback 到 DhcpEmpty（v2.0 没持久化，安全兜底：让 disable 写
        //    Empty 而不是错误的 [127.0.0.1]）。
        let mut manifest = storage.load_manifest()?;
        let original: OriginalDns = match manifest.original_dns.clone() {
            Some(saved) => saved,
            None => {
                eprintln!(
                    "[mHost] try_recover_dns: manifest.original_dns is None; \
                     treating as DhcpEmpty (legacy v2.0 residue). \
                     Will not write 127.0.0.1 back as the user's original."
                );
                OriginalDns::DhcpEmpty
            }
        };

        // 1.1 persist back：把 typed value 写回 manifest，下次启动就有值。
        if manifest.original_dns.is_none() {
            manifest.original_dns = Some(original.clone());
            if let Err(e) = storage.save_manifest(&manifest) {
                eprintln!(
                    "[mHost] Failed to persist original_dns after recovery: {}",
                    e
                );
            }
        }

        // 2. 创建 DnsConfig 和 DnsServer
        //   - Manual(servers)  → upstream = servers（用户在 System Settings
        //     里配的，session 内不变）；refresh_upstream = false
        //   - DhcpEmpty        → upstream = 当前系统能解析到的（Tier 3 兜底
        //     包括在内），refresh_upstream = true（mid-session 跨网络会自动跟随）
        let (upstream, _upstream_source, refresh_upstream) = match &original {
            OriginalDns::Manual(servers) => (
                servers.clone(),
                mhost_dns::UpstreamTier::Networksetup,
                false,
            ),
            OriginalDns::DhcpEmpty => {
                let (s, src) = mhost_dns::platform::get_upstream_resolvers();
                (s, src, true)
            }
        };
        let dns_port = mhost_dns::MHOST_DNS_PORT;
        let config = mhost_dns::DnsConfig {
            port: dns_port,
            upstream,
            refresh_upstream,
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
