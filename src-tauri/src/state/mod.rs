use mhost_apply::writer::HostsWriter;
use mhost_core::{AdBlockState, MhostError, OriginalDns, Profile, ProfileMode};
use mhost_storage::migration::migrate_v1_to_v2;
use mhost_storage::storage::{FileStorage, Storage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Poison-recovery helper for `std::sync::Mutex` (issue #130, PR #131
/// re-review). Returns the inner guard even if a previous holder panicked.
///
/// `tokio::sync::Mutex` (used by [`ApplyLock`]) does not have poison — a
/// panicked holder releases the lock automatically — so this helper is
/// Async mutex to serialize apply operations and prevent concurrent writes to /etc/hosts.
/// Security fix (#16): Prevents race conditions when user rapidly toggles profiles.
/// Perf fix (#26): Changed to `tokio::sync::Mutex` to allow holding across await points.
///
/// Poisoning note: `tokio::sync::Mutex` does NOT poison — when a guard is
/// dropped (including via panic), the lock is released automatically. So
/// `ApplyLock` callers can use `.lock().await` directly without recovery.
/// Use [`lock_or_recover`] for `std::sync::Mutex` poisoning scenarios.
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

/// Acquire a `std::sync::Mutex` guard, recovering transparently from poison.
///
/// `std::sync::Mutex` enters a poisoned state when a thread panics while
/// holding the lock — subsequent `.lock()` calls return `Err(PoisonError)`.
/// Rather than crash the whole app and leave the user with an unrecoverable
/// profile/DNS state, we accept the (rare) partial-write scenario as the
/// lesser evil: in-memory state may be briefly stale, but the next successful
/// apply/save corrects it. Users would rather see a working app than a
/// crash dialog after a stray panic during an IPC handler.
///
/// Only use this for `std::sync::Mutex`. `tokio::sync::Mutex` (used by
/// `ApplyLock`) does not poison and does not need recovery.
pub(crate) fn lock_or_recover<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// `std::sync::RwLock` 的 poison recovery（issue #181 P-R12 + P-R15 配套）。
/// 用法同 [`lock_or_recover`]，区分 read / write guard 类型。
pub(crate) fn lock_or_recover_rwlock<T>(
    rwlock: &std::sync::RwLock<T>,
) -> std::sync::RwLockReadGuard<'_, T> {
    match rwlock.read() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(crate) fn lock_or_recover_rwlock_write<T>(
    rwlock: &std::sync::RwLock<T>,
) -> std::sync::RwLockWriteGuard<'_, T> {
    match rwlock.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub struct AppState {
    pub storage: Arc<dyn Storage + Send + Sync>,
    pub writer: Arc<HostsWriter>,
    pub apply_lock: ApplyLock,
    /// N2: Serialize snapshot save/delete operations to prevent races.
    pub snapshot_lock: Arc<ApplyLock>,
    /// Perf fix (#29): Track last rendered profile IDs to avoid unnecessary menu rebuilds.
    pub last_profile_ids: Mutex<Vec<String>>,
    /// Perf fix (P-R12 + P-R15, issue #181): Cached profile list to avoid
    /// re-reading every profile JSON on every tray build / apply / IPC read.
    ///
    /// `None` = not loaded; first [`cached_profiles`] call populates.
    /// All profile mutation IPC handlers (create / update / delete / set_enabled /
    /// duplicate / import) **must** call [`invalidate_profile_cache`] after
    /// their storage write so the next read sees fresh data.
    ///
    /// Uses `std::sync::RwLock` (not tokio) because tray callbacks are
    /// sync and don't need to hold across await; lock acquisition is
    /// non-blocking as long as no panics while holding write guard.
    pub cached_profiles: RwLock<Option<Vec<Profile>>>,
    // DNS 相关
    pub dns_server: Arc<Mutex<Option<mhost_dns::DnsServer>>>,
    pub dns_enabled: AtomicBool,
    /// 启用 DNS 模式时捕获的 snapshot（语义版本）。
    /// **fix（disabling-after-network-switch）**：原 `Vec<String>` 没有
    /// 「manual vs DHCP」的区分，导致 disable 时把 DHCP 推的 IP 错误
    /// 回写到系统 DNS。现在用 `OriginalDns` 区分，DhcpEmpty 写 Empty
    /// （= DHCP default）。
    pub original_dns: tokio::sync::RwLock<OriginalDns>,
    /// 串行化 DNS 模式切换操作。
    pub dns_lock: ApplyLock,

    /// Cooperative cancellation signal for the in-flight DNS enable/disable
    /// operation (issue #149).
    ///
    /// `set_dns_mode` allocates a fresh `CancellationToken` on entry and
    /// swaps it into this slot; `cancel_dns_mode` fires the token so the
    /// long-running enable path can observe cancellation at its phase
    /// boundaries and roll back. The token is cleared on `set_dns_mode`
    /// completion.
    ///
    /// Like `ad_block_refresh_cancel` (issue #138), this is wrapped in a
    /// `Mutex` so callers can replace the slot (rather than mutate a
    /// shared token) — `CancellationToken::cancel()` is sticky, so a
    /// disable → re-enable cycle must not hand the new operation the
    /// previously-cancelled token. See `dns::set_dns_mode` for the swap
    /// contract and tests for the rollback behavior.
    pub dns_cancel: Mutex<Option<CancellationToken>>,

    // -------------------------------------------------------------------
    // 广告屏蔽（issue #130）
    // -------------------------------------------------------------------
    /// 当前持久化的 ad block 状态（含 sources / whitelist / refresh 配置）。
    /// 命令层 `set_*` 操作通过 `tokio::sync::RwLock::write().await` 修改，
    /// DNS 集成读路径用 `.read().await` 拿 snapshot 喂给
    /// `DnsServer::reload_ad_block_rules`。
    pub ad_block_state: Arc<tokio::sync::RwLock<AdBlockState>>,
    /// 后台 ad block 定时刷新 task 句柄（`spawn_ad_block_refresh_task`）。
    /// `set_dns_mode_disable` / `cleanup_dns_on_exit` 时 `take()` 出来 abort。
    pub ad_block_refresh_task: Mutex<Option<JoinHandle<()>>>,
    /// ad block 定时刷新 task 的 cancel 令牌。
    /// `cancel()` 唤醒 `select!` 中的 sleep 分支，让 disable / cleanup 立即
    /// 生效；`spawn_ad_block_refresh_task` 在 spawn 前 swap 一个新 token
    /// 避免上次 token 的 stickiness 干扰下次启用（issue #138）。
    pub ad_block_refresh_cancel: Mutex<tokio_util::sync::CancellationToken>,
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
        let storage_root = storage.root().to_path_buf();
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

        let dns_server = Arc::new(Mutex::new(dns_server_opt));

        let state = Self {
            storage,
            writer,
            apply_lock: ApplyLock(tokio::sync::Mutex::new(())),
            snapshot_lock: Arc::new(ApplyLock(tokio::sync::Mutex::new(()))),
            last_profile_ids: Mutex::new(Vec::new()),
            cached_profiles: RwLock::new(None), // lazy load on first cached_profiles() call
            dns_server,
            dns_enabled: AtomicBool::new(dns_enabled),
            original_dns: tokio::sync::RwLock::new(original_dns),
            dns_lock: ApplyLock(tokio::sync::Mutex::new(())),
            dns_cancel: Mutex::new(None),
            // Ad block（issue #130）：从 adblock.json 恢复，损坏时自动备份。
            ad_block_state: Arc::new(tokio::sync::RwLock::new(
                mhost_storage::adblock::read_state_or_default_with_backup(&storage_root),
            )),
            ad_block_refresh_task: Mutex::new(None),
            ad_block_refresh_cancel: Mutex::new(tokio_util::sync::CancellationToken::new()),
        };

        // 冷启动自动恢复（PR #131 review P1-1）：如果上次退出时
        // dns_enabled=true，DNS server 已经起来 + 持久化的 ad-block
        // 状态已加载；立即 hot-reload 当前规则到刚构造的 engine，并启动
        // 定时刷新 task。否则会有一段「DNS 通了但 ad-block 没生效」的空窗。
        //
        // **PR #154 review (P2) defensive**: `classify_rules` is sync and
        // reads each source's cache file from disk + parses 100k+ domains.
        // On a slow filesystem (network mount, encrypted APFS) this could
        // block `AppState::new` for seconds. Wrap the read+parse+reload
        // in `spawn_blocking` so it runs on a dedicated blocking thread.
        // `spawn_ad_block_refresh_task` itself is async (it spawns a
        // tokio task + schedules a select!), so it stays on the async
        // runtime — only the sync pipeline needs the offload.
        if state.dns_enabled.load(Ordering::Relaxed) {
            let snap = state.ad_block_state.read().await.clone();
            let storage_root = state.storage.root().to_path_buf();
            let dns_server = Arc::clone(&state.dns_server);
            let result = tokio::task::spawn_blocking(move || {
                let (za, nx, wl) = crate::commands::adblock::classify_rules(&snap, &storage_root);
                if let Some(server) = crate::state::lock_or_recover(&dns_server).as_ref() {
                    server.reload_ad_block_rules(za, nx, wl);
                }
            })
            .await;
            if let Err(e) = result {
                eprintln!(
                    "[mHost] cold-start ad-block reload join error: {} (continuing without hot-reload)",
                    e
                );
            }
            crate::commands::dns::spawn_ad_block_refresh_task(
                &state.ad_block_refresh_task,
                &state.ad_block_state,
                &state.dns_server,
                &state.storage,
                &state.ad_block_refresh_cancel,
            );
        }

        Ok(state)
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
            // **fix (issue #152, root cause 1)**: marker is written by
            // `platform::disable_dns_mode` to `runtime_dir()/mhost-dns-disable-recovery.marker`
            // (see `mhost_dns::platform::disable_recovery_marker_file()`,
            // `platform.rs:82`). The hard-coded `/tmp/...` path here was
            // dead code — `disable_dns_mode` never wrote to `/tmp`, so
            // this `if` branch never fired, and `force_dns_restore_if_needed`
            // was never called from this site. After a failed disable the
            // marker sat orphaned on disk while system DNS stayed at
            // 127.0.0.1.
            //
            // Use the canonical helper to read the same path the disable
            // path writes to.
            let marker_path = mhost_dns::platform::disable_recovery_marker_file();
            if marker_path.exists() {
                eprintln!(
                    "[mHost] try_recover_dns: disable recovery marker found at {}, forcing restore",
                    marker_path.display()
                );
                // fix：force_dns_restore_if_needed 内部走 osascript，
                // 同步裸调会阻塞 tokio worker。挪到 spawn_blocking。
                let force_restore_result = tokio::task::spawn_blocking(|| {
                    mhost_dns::platform::force_dns_restore_if_needed()
                })
                .await;
                match force_restore_result {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => eprintln!("[mHost] force restore failed: {}", e),
                    Err(join_err) => eprintln!(
                        "[mHost] force_dns_restore_if_needed blocking task join failed: {}",
                        join_err
                    ),
                }
                // `force_dns_restore_if_needed` deletes the marker itself
                // on success; if it failed, the marker remains and we
                // will retry next launch.
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
        //
        // fix：与 commands/dns.rs set_dns_mode_enable 第 6 步同理，
        // enable_dns_mode 内部走 osascript 同步阻塞，挪到 spawn_blocking。
        let enable_result = tokio::task::spawn_blocking({
            let original = original.clone();
            move || mhost_dns::platform::enable_dns_mode(dns_port, &original)
        })
        .await;
        let enable_outcome = match enable_result {
            Ok(inner) => inner,
            Err(join_err) => {
                let _ = server.stop().await;
                return Err(MhostError::InvalidInput(format!(
                    "Failed to enable DNS mode (blocking task join failed): {}",
                    join_err
                )));
            }
        };
        if let Err(e) = enable_outcome {
            let _ = server.stop().await;
            return Err(MhostError::InvalidInput(format!(
                "Failed to enable DNS mode: {}",
                e
            )));
        }

        Ok((server, original))
    }

    // -------------------------------------------------------------------
    // Profile list cache (issue #181 P-R12 + P-R15)
    // -------------------------------------------------------------------

    /// 读缓存；miss 时从 storage 加载并写入缓存。
    ///
    /// **双重检查锁**：read 命中直接 clone 返回（锁立即 drop）；
    /// miss 才走 write 路径 load + store。Tray 每次 build 都读，
    /// 二次读零 I/O 是主要收益。
    pub fn cached_profiles(&self) -> Result<Vec<Profile>, mhost_core::StorageError> {
        // Fast path: cache hit.
        {
            let guard = lock_or_recover_rwlock(&self.cached_profiles);
            if let Some(c) = guard.as_ref() {
                return Ok(c.clone());
            }
        }
        // Slow path: load from storage, store to cache, return clone.
        let profiles = self.storage.list_profiles()?;
        *lock_or_recover_rwlock_write(&self.cached_profiles) = Some(profiles.clone());
        Ok(profiles)
    }

    /// 失效缓存：mutation 后调用，下一次 [`cached_profiles`] 会重新从 storage 拉。
    /// 比 write-through（更新单条 Vec 项）简单，且在 `disable_other_profiles` 这种
    /// 循环写 N 条场景下也只需在末尾调一次。
    pub fn invalidate_profile_cache(&self) {
        *lock_or_recover_rwlock_write(&self.cached_profiles) = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn lock_or_recover_returns_guard_on_normal_lock() {
        let m = Mutex::new(42u32);
        {
            let guard = lock_or_recover(&m);
            assert_eq!(*guard, 42);
        }
        // lock is released; subsequent lock succeeds
        *lock_or_recover(&m) = 100;
        assert_eq!(*lock_or_recover(&m), 100);
    }

    #[test]
    fn lock_or_recover_recovers_from_poison() {
        let m = Arc::new(Mutex::new(String::from("before-panic")));
        let m2 = m.clone();

        // Poison the mutex by panicking while holding it.
        let join = std::thread::spawn(move || {
            let mut g = m2.lock().unwrap();
            g.push_str("-partial");
            panic!("simulated panic while holding lock");
        });
        let _ = join.join(); // Err is expected — the thread panicked.

        // Sanity check: the underlying mutex is now poisoned.
        assert!(m.lock().is_err(), "mutex should be poisoned");

        // The helper must transparently recover.
        let guard = lock_or_recover(&m);
        assert_eq!(guard.as_str(), "before-panic-partial");
    }

    #[test]
    fn lock_or_recover_allows_mutation_after_recovery() {
        let m = Arc::new(Mutex::new(0u32));
        let m2 = m.clone();

        let join = std::thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("simulated panic");
        });
        let _ = join.join();

        // After recovery, the helper should hand out a writable guard.
        *lock_or_recover(&m) = 999;
        assert_eq!(*lock_or_recover(&m), 999);
    }

    // -----------------------------------------------------------------------
    // lock_or_recover_rwlock (issue #181 P-R12 + P-R15)
    // -----------------------------------------------------------------------

    #[test]
    fn lock_or_recover_rwlock_returns_read_guard_on_normal_lock() {
        let r: std::sync::RwLock<u32> = std::sync::RwLock::new(42);
        let guard = lock_or_recover_rwlock(&r);
        assert_eq!(*guard, 42);
    }

    #[test]
    fn lock_or_recover_rwlock_write_allows_mutation() {
        let r: std::sync::RwLock<u32> = std::sync::RwLock::new(0);
        *lock_or_recover_rwlock_write(&r) = 100;
        assert_eq!(*lock_or_recover_rwlock(&r), 100);
    }

    #[test]
    fn lock_or_recover_rwlock_recovers_from_poison() {
        let r = std::sync::Arc::new(std::sync::RwLock::new("data".to_string()));
        let r2 = r.clone();

        let join = std::thread::spawn(move || {
            let _g = r2.write().unwrap();
            panic!("simulated panic in write guard");
        });
        let _ = join.join();

        // The RwLock is now poisoned, but our helper must recover transparently.
        let guard = lock_or_recover_rwlock(&r);
        assert_eq!(guard.as_str(), "data");
    }

    #[test]
    fn lock_or_recover_rwlock_write_recovers_from_poison() {
        let r = std::sync::Arc::new(std::sync::RwLock::new(0u32));
        let r2 = r.clone();

        let join = std::thread::spawn(move || {
            let _g = r2.write().unwrap();
            panic!("simulated panic");
        });
        let _ = join.join();

        // Should be able to write after recovery.
        *lock_or_recover_rwlock_write(&r) = 999;
        assert_eq!(*lock_or_recover_rwlock(&r), 999);
    }

    // -----------------------------------------------------------------------
    // AppState::cached_profiles (issue #181 P-R12 + P-R15)
    // -----------------------------------------------------------------------
    //
    // 这些 test 需要一个真实构造的 AppState。我们构造最小的状态（只关心
    // storage + cached_profiles 字段），避开 DNS / manifest 等无关字段。

    use mhost_storage::storage::{FileStorage, Storage};
    use tempfile::TempDir;

    /// 构造一个最小可用的 AppState：只填 storage 和 cached_profields，
    /// 其他字段填默认值，足够验证 cache 行为。
    fn make_test_state() -> (TempDir, std::sync::Arc<AppState>) {
        let temp_dir = TempDir::new().unwrap();
        let storage = std::sync::Arc::new(FileStorage::new(temp_dir.path()))
            as std::sync::Arc<dyn Storage + Send + Sync>;
        let state = AppState {
            storage,
            writer: std::sync::Arc::new(mhost_apply::writer::HostsWriter::new()),
            apply_lock: ApplyLock::new(),
            snapshot_lock: Arc::new(ApplyLock::new()),
            last_profile_ids: std::sync::Mutex::new(Vec::new()),
            cached_profiles: std::sync::RwLock::new(None),
            dns_server: std::sync::Arc::new(std::sync::Mutex::new(None)),
            dns_enabled: std::sync::atomic::AtomicBool::new(false),
            original_dns: tokio::sync::RwLock::new(OriginalDns::DhcpEmpty),
            dns_lock: ApplyLock::new(),
            dns_cancel: std::sync::Mutex::new(None),
            ad_block_state: std::sync::Arc::new(tokio::sync::RwLock::new(
                mhost_core::AdBlockState::default(),
            )),
            ad_block_refresh_task: std::sync::Mutex::new(None),
            ad_block_refresh_cancel: std::sync::Mutex::new(
                tokio_util::sync::CancellationToken::new(),
            ),
        };
        (temp_dir, std::sync::Arc::new(state))
    }

    #[test]
    fn cached_profiles_lazy_loads_on_first_call() {
        let (_temp, state) = make_test_state();
        // First call should populate the cache by reading from storage.
        let profiles = state.cached_profiles().unwrap();
        assert!(profiles.is_empty());
    }

    #[test]
    fn cached_profiles_returns_clones_without_mutating_cache() {
        let (_temp, state) = make_test_state();

        // Populate cache
        let first = state.cached_profiles().unwrap();
        assert!(first.is_empty());

        // Modify external storage (cache should NOT auto-refresh)
        let mut profile = mhost_core::Profile::new("late-add");
        profile.id = mhost_core::ProfileId(uuid::Uuid::new_v4());
        state.storage.save_profile(&profile).unwrap();

        // cached_profiles() still returns old (cached) value
        let second = state.cached_profiles().unwrap();
        assert!(
            second.is_empty(),
            "cache must not auto-refresh on external write"
        );
        assert_eq!(first.len(), second.len());

        // invalidate_profile_cache forces refresh
        state.invalidate_profile_cache();
        let third = state.cached_profiles().unwrap();
        assert_eq!(third.len(), 1);
        assert_eq!(third[0].name, "late-add");
    }

    #[test]
    fn cached_profiles_returns_independent_clones() {
        // Multiple readers should not interfere with each other.
        let (_temp, state) = make_test_state();
        let mut p = mhost_core::Profile::new("shared");
        p.id = mhost_core::ProfileId(uuid::Uuid::new_v4());
        state.storage.save_profile(&p).unwrap();

        let a = state.cached_profiles().unwrap();
        let b = state.cached_profiles().unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        // distinct Vec instances
        assert!(!std::ptr::eq(a.as_ptr(), b.as_ptr()));
    }
}
