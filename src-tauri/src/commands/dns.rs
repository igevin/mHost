use std::sync::atomic::Ordering;

use mhost_core::{MhostError, OriginalDns, ProfileMode};
use tauri::State;

use crate::state::AppState;

/// 启动/停止 DNS 模式。
///
/// # 状态机正确性（fix: systematic DNS logic review）
///
/// **核心原则：persist-before-mutate** —— 先把目标状态写入 manifest（持久层），
/// 再修改 in-memory `AppState`。这样任何中间步骤失败都不会留下「半启用」
/// 状态：要么 manifest 已记录目标状态、in-memory 还没追上，要么整个失败
/// 状态可被下次启动的 `try_recover_dns` 识别并纠正。
///
/// 启用序列：
///   1. `capture_dns_state()` 一次性读取 `original`（语义版本：区分
///      `Manual(servers)` vs `DhcpEmpty`）
///   2. 构造 DnsConfig + 启动 DnsServer（`refresh_upstream` 由 `original`
///      决定：Manual → false，DhcpEmpty → true）
///   3. `enable_dns_mode(port)` 修改系统 DNS（可回滚；DhcpEmpty 不写
///      original.txt，因为没有手动 IP 可还原）
///   4. **持久化** manifest（dns_enabled=true, original_dns=Some(original)）
///   5. 仅在第 4 步成功后才更新 in-memory state
///
/// 停用序列与启用对称：
///   1. 读取当前 `state.original_dns: OriginalDns`
///   2. **持久化** manifest（dns_enabled=false, original_dns 保留）
///   3. `disable_dns_mode(&original, interactive)` 恢复系统 DNS
///      - Manual  → 写回 server 列表
///      - DhcpEmpty → 写 `Empty`（DHCP default，不泄漏 DHCP 推的 IP）
///   4. 停止 DnsServer
///   5. 仅在所有副作用成功后清空 in-memory dns_server
#[tauri::command]
pub async fn set_dns_mode(enabled: bool, state: State<'_, AppState>) -> Result<(), MhostError> {
    let _guard = state.dns_lock.lock().await;

    if enabled {
        set_dns_mode_enable(&state).await
    } else {
        // 用户点 Disable → 在场，可以弹 sudo。`interactive=true` 让
        // proxy 死了 / 5s 超时分支用 osascript 兜底恢复。
        set_dns_mode_disable(&state, true).await
    }
}

/// 启用 DNS 模式。
///
/// 失败时的回滚是**尽力而为**：每个外部副作用（bind 端口、调用 osascript、
/// 写 manifest）失败时，我们尝试撤销之前已完成的副作用。但只要成功撤销
/// 关键的「系统 DNS 改写」就算用户可恢复；端口绑定的 server 会立即 stop。
async fn set_dns_mode_enable(state: &AppState) -> Result<(), MhostError> {
    // 1. 单一来源读取（fix：disabling-after-network-switch）。
    //    capture_dns_state() 返回语义版本 `OriginalDns`：
    //      - Tier 1 (`networksetup -getdnsservers`) 非空 → Manual(list)
    //      - Tier 1 空                      → DhcpEmpty
    //    Tier 3 公共 DNS 兜底**不**进 snapshot（它表示「系统真没 DNS」，
    //    只作为 upstream 的 fallback —— 见 get_upstream_resolvers）。
    let original = mhost_dns::platform::capture_dns_state()
        .map_err(|e| MhostError::InvalidInput(format!("capture dns state failed: {}", e)))?;

    // 2. 决定 upstream 初始值 + 是否启用 mid-session 上游刷新。
    //    Manual(servers)    → upstream = servers（用户意图，session 内不变）；
    //                         refresh_upstream = false
    //    DhcpEmpty          → upstream = 当前系统能解析到的（Tier 1 → Tier 2 →
    //                         Tier 3 兜底）；refresh_upstream = true
    //                         （mid-session 跨网络时由 DnsServer 后台 task
    //                         重新调用 get_upstream_resolvers 并 hot-swap）
    let (upstream, refresh_upstream) = match &original {
        OriginalDns::Manual(servers) => (servers.clone(), false),
        OriginalDns::DhcpEmpty => (mhost_dns::platform::get_upstream_resolvers(), true),
    };
    if upstream == vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()] {
        eprintln!(
            "[mHost] no system DNS detected (networksetup empty + ipconfig empty); \
             using public fallback as upstream only (snapshot = DhcpEmpty). \
             Check your network connection."
        );
    }

    // 3. 构造并启动 DnsServer（macOS 上监听非特权端口 1053）
    let config = mhost_dns::DnsConfig {
        port: mhost_dns::MHOST_DNS_PORT,
        upstream,
        refresh_upstream,
        ..Default::default()
    };
    let dns_port = config.port;
    let server = mhost_dns::DnsServer::new(config)
        .map_err(|e| MhostError::InvalidInput(format!("dns server init failed: {}", e)))?;

    // 4. 加载已启用的 DNS 模式 Profile，注入规则
    let profiles = state
        .storage
        .list_profiles_by_mode(ProfileMode::Dns)
        .map_err(MhostError::from)?;
    let enabled_profiles: Vec<_> = profiles.into_iter().filter(|p| p.enabled).collect();
    server.reload_rules(&enabled_profiles);

    // 5. 启动 server（绑定 1053）。失败时还没有副作用，仅回滚构造。
    if let Err(e) = server.start().await {
        return Err(MhostError::InvalidInput(format!(
            "dns server start failed: {}",
            e
        )));
    }

    // 6. 启动 privileged proxy + 把系统 DNS 切到 127.0.0.1。
    //    这是不可逆的副作用；失败必须 stop server 并返回 Err。
    //    fix（proxy self-cleanup）：把 &OriginalDns 传给 proxy，让它在
    //    退出时能自己恢复系统 DNS（DhcpEmpty → 写 Empty；Manual → 写回 list）。
    if let Err(e) = mhost_dns::platform::enable_dns_mode(dns_port, &original) {
        let _ = server.stop().await;
        return Err(MhostError::InvalidInput(format!(
            "Failed to enable DNS mode: {}",
            e
        )));
    }

    // 7. **PERSIST MANIFEST FIRST** —— 持久层是 commit point。
    //    只有 manifest 写入成功后才允许修改 in-memory state。
    //    如果 save_manifest 失败，需要把系统 DNS 恢复 + 停 server，
    //    否则下次启动 try_recover_dns 会看到 dns_enabled=true 但实际服务已挂。
    let manifest_save_result = (|| -> Result<(), MhostError> {
        let mut manifest = state.storage.load_manifest().map_err(MhostError::from)?;
        manifest.dns_enabled = Some(true);
        manifest.original_dns = Some(original.clone());
        state
            .storage
            .save_manifest(&manifest)
            .map_err(MhostError::from)?;
        Ok(())
    })();

    if let Err(e) = manifest_save_result {
        // 尽力回滚：恢复系统 DNS + 停 server。
        // 用户刚接受了 enable 的 sudo 弹窗，回滚也用 interactive=true
        // 让 proxy 死了时也能走 osascript 兜底（同样弹 sudo 框）。
        let restore_err = mhost_dns::platform::disable_dns_mode(&original, true);
        let _ = server.stop().await;
        return Err(match restore_err {
            Ok(_) => e,
            Err(restore) => {
                MhostError::InvalidInput(format!("{} (rollback also failed: {})", e, restore))
            }
        });
    }

    // 8. manifest 已成功落盘，现在才允许修改 in-memory state。
    match state.original_dns.lock() {
        Ok(mut guard) => *guard = original,
        Err(poisoned) => {
            *poisoned.into_inner() = original;
        }
    }
    match state.dns_server.lock() {
        Ok(mut guard) => *guard = Some(server),
        Err(poisoned) => {
            *poisoned.into_inner() = Some(server);
        }
    }
    state.dns_enabled.store(true, Ordering::Relaxed);

    Ok(())
}

/// 停用 DNS 模式。
///
/// 与启用对称：先持久化 manifest，再做实际 stop + restore 副作用。
///
/// `interactive=true`：用户从 UI 点的 Disable（在场），proxy 没恢复时
/// 走 osascript 弹 sudo 兜底。
/// `interactive=false`：app 退出清理（用户可能不在场），不弹 sudo，
/// marker 保留给下次启动 `try_recover_dns` 走 `force_dns_restore_if_needed`。
async fn set_dns_mode_disable(state: &AppState, interactive: bool) -> Result<(), MhostError> {
    // 1. 读取 in-memory original_dns（由 enable 路径写入）
    let original = match state.original_dns.lock() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };

    // fix (bug 1, disable-mode refuses on empty snapshot):
    //   之前在 `state.original_dns` 为空 且 当前系统 DNS 含 127.0.0.1 时拒绝
    //   disable。这是合法场景：用户当时系统 DNS 是空的（DHCP 没下发 /
    //   用户手动清空），所以 `capture_dns_state()` 返回 `DhcpEmpty`。
    //   现在系统 DNS 是 127.0.0.1 是 mhost proxy 自己在用。
    //
    //   proxy.rs::restore_dns_and_exit 走自己的恢复路径：读 original.txt，
    //   空时（DhcpEmpty 不写文件）生成 `networksetup -setdnsservers
    //   <iface> Empty`（DHCP 默认）。
    //   所以 DhcpEmpty 是可恢复的；disable 路径安全。
    //
    //   此处只做日志，**不**返回错误。
    if matches!(original, OriginalDns::DhcpEmpty) {
        eprintln!(
            "[mHost] set_dns_mode_disable: original was DhcpEmpty (user had no manual \
             DNS config when DNS mode was enabled). Proxy will restore system DNS \
             to DHCP default via `networksetup -setdnsservers <iface> Empty`."
        );
    }

    // 2. **PERSIST MANIFEST FIRST** —— 把 dns_enabled 标 false，让
    //    下次启动 try_recover_dns 知道「不需要再恢复」。
    //    如果这一步失败，in-memory state 保持不变，调用方看到 Err 后
    //    可以重试；系统 DNS 此时尚未被改写。
    let mut manifest = state.storage.load_manifest().map_err(MhostError::from)?;
    manifest.dns_enabled = Some(false);
    state
        .storage
        .save_manifest(&manifest)
        .map_err(MhostError::from)?;

    // 3. 持久化成功后，做实际 stop：先恢复系统 DNS，再 stop server。
    //    restore_dns 失败会让用户留在「系统 DNS 指向 127.0.0.1」状态，
    //    但 in-memory 状态已经标 false，下次启动会按 dns_enabled=false
    //    处理；这是可恢复的。
    if let Err(e) = mhost_dns::platform::disable_dns_mode(&original, interactive) {
        // 已经成功写了 manifest 标 false，所以这里只用 InvalidInput
        // 提示用户「系统 DNS 没恢复成功，需要手动检查」。
        return Err(MhostError::InvalidInput(format!(
            "Failed to restore system DNS: {}. \
             Manually run `networksetup -setdnsservers <interface> {}`",
            e,
            original.restore_argv().join(" ")
        )));
    }

    // 4. 停 server（清空 in-memory dns_server）
    let server_opt = match state.dns_server.lock() {
        Ok(mut guard) => guard.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    };
    if let Some(server) = server_opt {
        if let Err(e) = server.stop().await {
            // server 已 stop 失败（端口占用？），但 manifest 已标 false，
            // 系统 DNS 已恢复，下次启动不会再启动服务。
            return Err(MhostError::InvalidInput(format!(
                "dns server stop failed: {} (system DNS already restored)",
                e
            )));
        }
    }

    // 5. 清 in-memory dns_enabled
    state.dns_enabled.store(false, Ordering::Relaxed);

    Ok(())
}

/// 获取 DNS 模式状态。
#[tauri::command]
pub async fn get_dns_mode(state: State<'_, AppState>) -> Result<bool, MhostError> {
    Ok(state.dns_enabled.load(Ordering::Relaxed))
}

/// 重新加载 DNS 规则（profile 变更后调用）。
#[tauri::command]
pub async fn reload_dns_rules(state: State<'_, AppState>) -> Result<(), MhostError> {
    if !state.dns_enabled.load(Ordering::Relaxed) {
        return Ok(());
    }

    let profiles = state
        .storage
        .list_profiles_by_mode(ProfileMode::Dns)
        .map_err(MhostError::from)?;
    let enabled_profiles: Vec<_> = profiles.into_iter().filter(|p| p.enabled).collect();

    match state.dns_server.lock() {
        Ok(guard) => {
            if let Some(server) = guard.as_ref() {
                server.reload_rules(&enabled_profiles);
            }
        }
        Err(poisoned) => {
            let guard = poisoned.into_inner();
            if let Some(server) = guard.as_ref() {
                server.reload_rules(&enabled_profiles);
            }
        }
    }

    Ok(())
}

/// App 退出时的 DNS 清理（fix: 用户反馈"退出后 DNS 出问题"）。
///
/// 由 `lib.rs::run()` / `lib.rs::cleanup_and_exit` 在三处调用：
///   1) Tray Quit 菜单（用户在场 → `interactive=true`，proxy 死了走
///      osascript sudo 兜底）
///   2) Tauri `RunEvent::ExitRequested` 钩子（Cmd-Q 兜底 → `interactive=false`）
///   3) setup() 里 spawn 的 tokio signal handler（SIGINT/SIGTERM，
///      覆盖 Ctrl+C / kill / OS 关机 → `interactive=false`）
///
/// 不持 Tauri `State<'_, AppState>` 的原因：RunEvent 回调运行在
/// Tauri 2 内部 task 上下文，没有命令调用栈，`State<'_, AppState>`
/// 这种借用参数无法构造。直接用 `&AppState`。
///
/// 幂等性（fix issue #67）：
///   - 入口先把 in-memory `dns_enabled` 标 false，让 SIGINT / ExitRequested /
///     tray Quit 三条路径竞态时只有第一个真正跑 cleanup；其余直接
///     early-return 走 no-op 分支。
///   - cleanup 本身失败（proxy 进程早死、osascript 兜底失败）是可恢复的：
///     `disable_dns_mode` 已经写了 recovery marker，下次启动
///     `try_recover_dns` 会兜底强退。所以这里**返回 Ok**，只在 stderr
///     留一条 warning，避免退出时连续刷两条「DNS cleanup failed」误导用户。
///
/// `interactive` 参数语义：
///   - `true`：调用方确认用户在场，proxy 没恢复时走 osascript sudo 兜底，
///     让用户当场看到恢复成功。Tray Quit 用这个值。
///   - `false`：用户可能不在场（OS 关机 / SIGINT），不弹 sudo，marker
///     保留给下次启动 `try_recover_dns` 兜底。ExitRequested + signal handler
///     用这个值。
pub async fn cleanup_dns_on_exit(state: &AppState, interactive: bool) -> Result<(), MhostError> {
    if !state.dns_enabled.load(Ordering::Relaxed) {
        return Ok(());
    }
    // 标记 in-memory 为 disabled，让后续 cleanup_dns_on_exit 调用的路径
    // （SIGINT + Tauri ExitRequested + tray Quit 三条路径竞态时）走 no-op。
    state.dns_enabled.store(false, Ordering::Relaxed);

    match set_dns_mode_disable(state, interactive).await {
        Ok(()) => Ok(()),
        Err(e) => {
            // 清理失败一般是 proxy 早死或 osascript 失败 —— 留给下次启动
            // 的 recovery marker 兜底。这里只记一条 warning，不返回 Err
            // （避免 lib.rs 的「DNS cleanup on signal/exit failed」误导用户）。
            eprintln!(
                "[mHost] DNS cleanup on exit: {} (recovery marker preserved for next launch)",
                e
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ApplyLock;
    use mhost_apply::writer::HostsWriter;
    use mhost_storage::storage::FileStorage;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    /// 单元测试：DNS 模式未启用时，cleanup_dns_on_exit 直接返回 Ok，
    /// 不做 disable 副作用（不调 networksetup）。
    ///
    /// 这个测试覆盖「DNS 模式没启用就退出」的情况 —— 退出不该抛错。
    #[tokio::test]
    async fn test_cleanup_dns_on_exit_noop_when_dns_disabled() {
        let temp = TempDir::new().unwrap();
        let storage = Arc::new(FileStorage::new(temp.path()))
            as Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let state = AppState {
            storage,
            writer: Arc::new(HostsWriter::new()),
            apply_lock: ApplyLock::new(),
            snapshot_lock: ApplyLock::new(),
            last_profile_ids: Mutex::new(Vec::new()),
            dns_server: Arc::new(Mutex::new(None)),
            dns_enabled: AtomicBool::new(false),
            original_dns: Mutex::new(OriginalDns::DhcpEmpty),
            dns_lock: ApplyLock::new(),
        };
        // dns_enabled = false → cleanup 应直接返回 Ok
        let result = cleanup_dns_on_exit(&state, false).await;
        assert!(result.is_ok(), "DNS disabled → cleanup should be a no-op");
    }

    /// 回归测试（bug 1 + bug 4 + disabling-after-network-switch fix）：
    ///   - bug 1: `DhcpEmpty` 不应让 disable 报「refusing to disable」。
    ///     proxy 会用 `networksetup -setdnsservers <iface> Empty` 兜底。
    ///   - bug 4: exit cleanup（`interactive=false`）走到非 interactive
    ///     + proxy 不在的分支，必须返回 `Err` 保留 marker，下次启动
    ///     `try_recover_dns` 走 `force_dns_restore_if_needed` 兜底。
    ///     如果返回 `Ok(())` 意味着系统 DNS 卡在 127.0.0.1。
    ///
    /// **fix（disabling-after-network-switch）**：DhcpEmpty snapshot 是
    /// 用户没手动配 DNS 的合法状态，disable 必须写 Empty（不是恢复 DHCP
    /// 推的某次具体 IP）。
    #[tokio::test]
    async fn test_set_dns_mode_disable_succeeds_with_dhcp_empty_snapshot() {
        let temp = TempDir::new().unwrap();
        let storage = Arc::new(FileStorage::new(temp.path()))
            as Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        // seed manifest (set_dns_mode_disable 会 load_manifest，缺少会 Err)
        storage
            .save_manifest(&mhost_storage::manifest::Manifest::new(env!(
                "CARGO_PKG_VERSION"
            )))
            .unwrap();
        let state = AppState {
            storage,
            writer: Arc::new(HostsWriter::new()),
            apply_lock: ApplyLock::new(),
            snapshot_lock: ApplyLock::new(),
            last_profile_ids: Mutex::new(Vec::new()),
            dns_server: Arc::new(Mutex::new(None)),
            dns_enabled: AtomicBool::new(true), // 假装启用 → cleanup 会走 disable 路径
            original_dns: Mutex::new(OriginalDns::DhcpEmpty), // DhcpEmpty → 写 Empty
            dns_lock: ApplyLock::new(),
        };
        // cleanup_dns_on_exit → set_dns_mode_disable(interactive=false)
        //   - original 是 DhcpEmpty → 只打印 warning（不返回 Err，bug 1 修复）
        //   - manifest 写 dns_enabled=false → 走 disable_dns_mode
        //   - 测试环境没有真 proxy + non-interactive → 保留 marker
        //     + 返回 Ok（fix issue #67 bug 4：cleanup 失败转 warning，
        //       避免 SIGINT + ExitRequested 两条路径刷两条 failed 误导用户；
        //       DNS 真没恢复由 recovery marker 兜底，下次启动 try_recover_dns 强退）
        let result = cleanup_dns_on_exit(&state, false).await;
        assert!(
            result.is_ok(),
            "cleanup_dns_on_exit should return Ok even on proxy failure (recovery marker \
             handles actual restoration); got {:?}",
            result
        );

        // 关键断言：disable_dns_mode 应该以 restore_argv = ["Empty"] 调用
        // networksetup，不是写回 DHCP-pushed 的某次 IP。这通过 OriginalDns
        // 的语义在 mhost-dns::platform 内部保证；这里只能验证 type 层
        // round-trip 的语义（restore_argv）。
        assert_eq!(
            OriginalDns::DhcpEmpty.restore_argv(),
            vec!["Empty".to_string()],
            "DhcpEmpty snapshot 必须产生 Empty restore target"
        );
    }

    /// 回归测试（app-close DNS cleanup）：
    ///   - interactive 参数不影响 dns_enabled 标志行为
    ///   - 多次调用必须幂等（Path A + Path B + tray Quit 三条路径竞态时
    ///     只有第一个真正跑 cleanup，其余 no-op）
    #[tokio::test]
    async fn test_cleanup_dns_on_exit_idempotent_across_calls() {
        let temp = TempDir::new().unwrap();
        let storage = Arc::new(FileStorage::new(temp.path()))
            as Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        storage
            .save_manifest(&mhost_storage::manifest::Manifest::new(env!(
                "CARGO_PKG_VERSION"
            )))
            .unwrap();
        let state = AppState {
            storage,
            writer: Arc::new(HostsWriter::new()),
            apply_lock: ApplyLock::new(),
            snapshot_lock: ApplyLock::new(),
            last_profile_ids: Mutex::new(Vec::new()),
            dns_server: Arc::new(Mutex::new(None)),
            dns_enabled: AtomicBool::new(true),
            original_dns: Mutex::new(OriginalDns::DhcpEmpty),
            dns_lock: ApplyLock::new(),
        };

        // 第一次 cleanup：跑 disable 路径。注意必须用 interactive=false
        // —— interactive=true 会在 proxy 不在时走 `osascript_restore`
        // 弹 sudo 密码框，CI 无人点击会永远卡住。`cleanup_dns_on_exit`
        // 入口 (line 321) 已经在调 `set_dns_mode_disable` 之前把
        // `dns_enabled` 标 false，所以 disable 走 non-interactive
        // 分支（返回 Err）也满足幂等性测试的核心断言。
        let r1 = cleanup_dns_on_exit(&state, false).await;
        assert!(r1.is_ok());
        assert!(
            !state.dns_enabled.load(Ordering::Relaxed),
            "first cleanup must clear dns_enabled"
        );

        // 第二次 cleanup（模拟 Path A/B 同时触发 → interactive=true）：
        // dns_enabled 已被标 false，必须 no-op，不能再去碰 set_dns_mode_disable
        // （那里会再次 save_manifest + 调 networksetup）。
        let r2 = cleanup_dns_on_exit(&state, true).await;
        assert!(
            r2.is_ok(),
            "second cleanup must be a no-op (idempotency for double-exit paths)"
        );
        assert!(
            !state.dns_enabled.load(Ordering::Relaxed),
            "dns_enabled stays false across multiple cleanup calls"
        );

        // 第三次（同样）
        let r3 = cleanup_dns_on_exit(&state, false).await;
        assert!(r3.is_ok(), "third cleanup must also be a no-op");
    }
}

/// 获取 DNS 服务运行状态。
#[tauri::command]
pub async fn get_dns_status(
    state: State<'_, AppState>,
) -> Result<mhost_core::DnsStatus, MhostError> {
    let original_dns = match state.original_dns.lock() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };
    fn build(
        server: Option<&mhost_dns::DnsServer>,
        original_dns: OriginalDns,
    ) -> mhost_core::DnsStatus {
        match server {
            Some(s) => mhost_core::DnsStatus {
                running: s.is_running(),
                port: s.port(),
                upstream: s.upstream(),
                original_dns,
                rule_count: s.rule_count(),
                cache_capacity: s.cache_capacity(),
            },
            None => mhost_core::DnsStatus {
                running: false,
                port: 53,
                upstream: vec![],
                original_dns,
                rule_count: 0,
                cache_capacity: 0,
            },
        }
    }

    let status = match state.dns_server.lock() {
        Ok(guard) => build(guard.as_ref(), original_dns),
        Err(poisoned) => build(poisoned.into_inner().as_ref(), original_dns),
    };
    Ok(status)
}
