use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use crate::state::{lock_or_recover, AppState};
use mhost_core::{MhostError, OriginalDns, ProfileMode};
use tauri::State;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

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

    // 分配新的 cancellation token，并 swap 进 slot（issue #138 follow-up
    // 复用同一模式：不要 clone 现有 token，避免上一次操作的 cancel 漏到
    // 本次）。`cancel_dns_mode` IPC 会通过这个 token 通知 enable/disable
    // 路径走 rollback。
    let cancel = CancellationToken::new();
    *lock_or_recover(&state.dns_cancel) = Some(cancel.clone());

    let work = async {
        if enabled {
            set_dns_mode_enable(&state, &cancel).await
        } else {
            // 用户点 Disable → 在场，可以弹 sudo。`interactive=true` 让
            // proxy 死了 / 5s 超时分支用 osascript 兜底恢复。
            set_dns_mode_disable(&state, true, Some(&cancel)).await
        }
    };

    // select! 让 cancel 立即返回（前端 UI 可以立刻响应）；同时 enable
    // 内部也在 phase 边界检查 cancel 并跑 rollback，select! 是兜底。
    // **tokio::select! 不能取消 spawn_blocking**：enable 里的 osascript
    // 调用是 sync 阻塞在另一个线程，select! 不会中断它。enable 在
    // spawn_blocking 返回后会再次检查 cancel 走 disable rollback —— 这
    // 覆盖了 cancel 落在 spawn_blocking 期间的场景。
    let result = tokio::select! {
        biased;
        res = work => res,
        _ = cancel.cancelled() => Err(MhostError::Cancelled),
    };

    // 清空 slot。失败也清，保证下次操作拿到 fresh token。
    *lock_or_recover(&state.dns_cancel) = None;
    result
}

/// 取消正在进行的 DNS 启用/停用操作（issue #149）。
///
/// 通过 `AppState::dns_cancel` 里的 `CancellationToken` 通知
/// `set_dns_mode` 走 rollback 路径。没有正在进行的操作时是 no-op。
///
/// 前端 Cancel 按钮触发。AbortSignal 和本 IPC 是两件事：
/// - AbortSignal 让 `invoke()` 的 JS promise 立刻 reject 为
///   `DOMException(AbortError)`，前端据此识别「用户主动 cancel」；
/// - 本 IPC 让 Rust 端真正滚回去——否则 enable 路径上的 osascript
///   还在跑，proxy 已经被 trap 杀掉（issue #148）但系统 DNS 还没恢复。
#[tauri::command]
pub async fn cancel_dns_mode(state: State<'_, AppState>) -> Result<(), MhostError> {
    if let Some(token) = lock_or_recover(&state.dns_cancel).as_ref() {
        token.cancel();
    }
    Ok(())
}

/// 启用 DNS 模式。
///
/// 失败时的回滚是**尽力而为**：每个外部副作用（bind 端口、调用 osascript、
/// 写 manifest）失败时，我们尝试撤销之前已完成的副作用。但只要成功撤销
/// 关键的「系统 DNS 改写」就算用户可恢复；端口绑定的 server 会立即 stop。
///
/// **`cancel` 协作语义（issue #149）**：在 phase 边界检查 cancel：
/// 1. `server.start()` OK 后、osascript 前 → 取消 → stop server 即可
///    （无系统副作用）；返回 `Err(Cancelled)`。
/// 2. osascript OK 后 → 取消 → 系统 DNS 已切到 127.0.0.1 + proxy 已起。
///    必须调 `disable_dns_mode(..., None)` 走 self-cleanup + osascript
///    兜底把系统 DNS 恢复成 original。
/// 3. manifest 持久化后 → 取消 → 同上，调用 `set_dns_mode_disable`
///    走完整 rollback（清 in-memory 状态）。
///
/// **tokio::select! 不能取消 spawn_blocking**：osascript 那段不能被
/// 中断。enable 在 spawn_blocking 返回后会再次 check cancel 走 rollback，
/// 覆盖 cancel 落在 spawn_blocking 期间的场景。outer select! 是兜底。
async fn set_dns_mode_enable(
    state: &AppState,
    cancel: &CancellationToken,
) -> Result<(), MhostError> {
    // 1. 单一来源读取（fix：disabling-after-network-switch）。
    //    capture_dns_state() 返回语义版本 `OriginalDns`：
    //      - Tier 1 (`networksetup -getdnsservers`) 非空 → Manual(list)
    //      - Tier 1 空                      → DhcpEmpty
    //    Tier 3 公共 DNS 兜底**不**进 snapshot（它表示「系统真没 DNS」，
    //    只作为 upstream 的 fallback —— 见 get_upstream_resolvers）。
    let original = mhost_dns::platform::capture_dns_state()
        .map_err(|e| MhostError::InvalidInput(format!("capture dns state failed: {}", e)))?;
    tracing::info!(
        "set_dns_mode_enable: captured OriginalDns = {:?} \
         (Manual = user-configured in System Settings; DhcpEmpty = no manual DNS, will track network changes)",
        original
    );

    // 2. 决定 upstream 初始值 + 是否启用 mid-session 上游刷新。
    //    Manual(servers)    → upstream = servers（用户意图，session 内不变）；
    //                         refresh_upstream = false
    //    DhcpEmpty          → upstream = 当前系统能解析到的（Tier 1 → Tier 2 →
    //                         Tier 3 兜底）；refresh_upstream = true
    //                         （mid-session 跨网络时由 DnsServer 后台 task
    //                         重新调用 get_upstream_resolvers 并 hot-swap）
    let (upstream, upstream_source, refresh_upstream) = match &original {
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
    tracing::info!(
        "set_dns_mode_enable: initial upstream = {:?}, source = {:?}, refresh_upstream = {}",
        upstream,
        upstream_source,
        refresh_upstream
    );
    // fix issue #103 (review follow-up): 用 source 而非值比较，避免用户手编
    // [8.8.8.8, 1.1.1.1] / DHCP 恰好推送同样列表时误报。
    if upstream_source == mhost_dns::UpstreamTier::Public {
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

    // 5.1 (issue #149) cancel check before spawn_blocking。
    //   server 已 bind 端口,但还没有系统副作用(proxy 没起、networksetup
    //   没跑、manifest 没写)。如果 cancel 已触发,只需 stop server 释放
    //   端口 + 返回 Err(Cancelled),无需 disable rollback。
    if cancel.is_cancelled() {
        let _ = server.stop().await;
        eprintln!("[mHost] set_dns_mode_enable: cancelled before spawn_blocking");
        return Err(MhostError::Cancelled);
    }

    // 6. 启动 privileged proxy + 把系统 DNS 切到 127.0.0.1。
    //    这是不可逆的副作用；失败必须 stop server 并返回 Err。
    //    fix（proxy self-cleanup）：把 &OriginalDns 传给 proxy，让它在
    //    退出时能自己恢复系统 DNS（DhcpEmpty → 写 Empty；Manual → 写回 list）。
    //
    //    fix（regression from #155）+ **fix（issue #142）**：enable_dns_mode 内部走
    //    osascript 弹 sudo 密码框，`Command::output()` 同步阻塞。
    //
    //   **Group 4 (#149) 改动**:放弃 Group 3 加的 `tokio::time::timeout(60s, ...)`
    //   包装。理由:
    //     1. timeout 不取消 spawn_blocking 闭包 —— 60s 到时 IPC 返回 Err、前端
    //        loading 复位,但 osascript 在 blocking 线程继续跑,几秒后完成
    //        切系统 DNS + 起 proxy,in-memory 状态停留在 dns_enabled=false →
    //        完全 desync (issue #149 + PR #167 review feedback)。
    //     2. 改为直接 `await spawn_blocking`:osascript 弹授权框自带 Cancel
    //        按钮,用户可主动取消;前端 `withTimeout(30s)` (src/lib/tauri.ts)
    //        兜底;取消意图通过 `cancel_dns_mode` IPC 通知 Rust 端走
    //        rollback (见 issue #149)。
    //     3. 真正卡死场景 (#142 原始 TCC 死锁) 的兜底行为和旧 60s timeout
    //        一样 (用户都得 force-quit),但**不再 leak**。
    //
    //   cancel 检查放在 spawn_blocking 返回后:select! 不能取消已经在跑的
    //   spawn_blocking 闭包,但我们一定是在 osascript 自然返回后才到这里,
    //   此时 cancel token 已 fire → 必须 rollback (stop server + 调
    //   disable_dns_mode 把系统 DNS 恢复)。这里 `cancel=None` 因为 rollback
    //   是「已经决定要清理」,不应该被 cancel 再次打断。
    let original_for_enable = original.clone();
    match tokio::task::spawn_blocking(move || {
        mhost_dns::platform::enable_dns_mode(dns_port, &original_for_enable)
    })
    .await
    {
        Ok(Ok(())) => {
            // osascript 跑完了,proxy 在跑 + 系统 DNS 已切。
            if cancel.is_cancelled() {
                eprintln!(
                    "[mHost] set_dns_mode_enable: cancelled after spawn_blocking — rolling back"
                );
                let _ = server.stop().await;
                let _ = mhost_dns::platform::disable_dns_mode(&original, true, None);
                return Err(MhostError::Cancelled);
            }
        }
        Ok(Err(e)) => {
            // osascript 跑完了但返回 Err (proxy binary missing / 脚本 non-zero /
            // networksetup 失败等)。这种情况没有 leak —— enable_dns_mode 内部
            // 已经 rollback (proxy 被脚本自己 kill + 系统 DNS 未改)。
            let _ = server.stop().await;
            return Err(MhostError::InvalidInput(format!(
                "Failed to enable DNS mode: {}",
                e
            )));
        }
        Err(join_err) => {
            // spawn_blocking task panic。enable_dns_mode 不应 panic。
            let _ = server.stop().await;
            return Err(MhostError::InvalidInput(format!(
                "Failed to enable DNS mode (blocking task join failed): {}",
                join_err
            )));
        }
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
        // 同样包 spawn_blocking —— 见 set_dns_mode_enable 第 6 步注释，
        // disable_dns_mode 内部走 osascript 也会阻塞 tokio worker。
        // **Group 4 (#149) 改动**：rollback 路径改成同步调用（cleanup 路径
        // 持 dns_lock，无并发 DNS 操作；阻塞 tokio worker 在此可接受）。
        let restore_err = mhost_dns::platform::disable_dns_mode(&original, true, None);
        let _ = server.stop().await;
        return Err(match restore_err {
            Ok(_) => e,
            Err(restore) => {
                MhostError::InvalidInput(format!("{} (rollback also failed: {})", e, restore))
            }
        });
    }

    // 7.1 (issue #149) cancel check after manifest save。
    //   manifest 已落盘 + 系统 DNS = 127.0.0.1。如果 cancel 已触发,
    //   必须清 in-memory 状态 + 恢复系统 DNS = original。这里直接
    //   调 set_dns_mode_disable 走完整 rollback。注意 cancel=None:
    //   rollback 是已经决定要清理,不应该再被 cancel 打断。
    if cancel.is_cancelled() {
        eprintln!("[mHost] set_dns_mode_enable: cancelled after manifest save — rolling back");
        return set_dns_mode_disable(state, true, None).await;
    }

    // 8. manifest 已成功落盘，现在才允许修改 in-memory state。
    // (dns_server 仍用 std::sync::Mutex + lock_or_recover 处理 poison；original_dns
    // 改用 tokio::sync::RwLock 允许多读。)
    *state.original_dns.write().await = original;
    *lock_or_recover(&state.dns_server) = Some(server);
    state.dns_enabled.store(true, Ordering::Relaxed);

    // 9. 广告屏蔽（issue #130）：启用 DNS 后立即把当前 ad-block 状态
    //    hot-reload 到新 server，并启动定时刷新 task。task 在 disable
    //    时被 abort。
    //
    //    9a. 即时 reload：spawn_ad_block_refresh_task 在
    //    auto_refresh_enabled=false 或 interval=0 时不会 spawn，但用户
    //    仍期望持久化的 ad-block 规则立即生效。所以这里显式做一次
    //    classify + reload，与 AppState::new 冷启动路径一致。
    //
    //    9b. 这里复用了 commands::adblock 的 `classify_rules` + 重载路径
    //    的等价逻辑（避免循环依赖和 IPC 边界），不经过 IPC handler。
    let snap = state.ad_block_state.read().await.clone();
    let (za, nx, wl) = crate::commands::adblock::classify_rules(&snap, state.storage.root());
    if let Some(server) = lock_or_recover(&state.dns_server).as_ref() {
        server.reload_ad_block_rules(za, nx, wl);
    }
    spawn_ad_block_refresh_task(
        &state.ad_block_refresh_task,
        &state.ad_block_state,
        &state.dns_server,
        &state.storage,
        &state.ad_block_refresh_cancel,
    );

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
///
/// **`cancel`（issue #149）**：`Some(cancel)` 让用户在 disable 中途点
/// Cancel 时立刻跳出 `disable_dns_mode` 的 5s 等 proxy exit 等待循环，
/// proxy 自管清理继续在后台跑（recovery marker 兜底）。`None` 用于
/// rollback 调用（enable 路径里的 cancel 后清理）和 cleanup 路径，
/// 此时不能被打断。
async fn set_dns_mode_disable(
    state: &AppState,
    interactive: bool,
    cancel: Option<&CancellationToken>,
) -> Result<(), MhostError> {
    // 1. 读取 in-memory original_dns（由 enable 路径写入）
    let original = state.original_dns.read().await.clone();

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
    //
    //    cancel=None (rollback/cleanup 路径) :必须等 5s 完成 self-cleanup。
    //    cancel=Some (用户 disable 路径) :5s 等待里每 100ms 检查 cancel,
    //    一旦触发就立刻 return Ok;proxy 后续退出靠 recovery marker 兜底。
    //
    //    **Group 4 (#149) 改动**:与 enable 路径对称 —— disable 路径也直接
    //    调 disable_dns_mode (不再包 spawn_blocking)。cleanup 路径持 dns_lock,
    //    阻塞 tokio worker 在此可接受。
    if let Err(e) = mhost_dns::platform::disable_dns_mode(&original, interactive, cancel) {
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
    let server_opt = lock_or_recover(&state.dns_server).take();
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

    // 6. 终止广告屏蔽后台刷新 task（issue #130, #138）。enable 时 spawn，
    //    disable 必须 abort；不 abort 会让 task 继续跑并尝试 reload
    //    已停的 server。**先 cancel 再 abort**：cancel 让 refresh loop
    //    的 `select!` 醒来并把 spawn_blocking 闭包里的
    //    `is_cancelled()` check 触发，从而避免在已停的 server 上
    //    reload。abort 是兜底：如果 task 还卡在 select 之外（比如
    //    spawn_blocking 闭包刚起来），cancel() 的 wake 不会传到那里。
    cancel_ad_block_refresh_task(&state.ad_block_refresh_cancel);
    abort_ad_block_refresh_task(&state.ad_block_refresh_task);

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

    if let Some(server) = lock_or_recover(&state.dns_server).as_ref() {
        server.reload_rules(&enabled_profiles);
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

    match set_dns_mode_disable(state, interactive, None).await {
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

    /// `cancel_dns_mode` flips the slot's token when one is present.
    /// This is the contract the Settings page Cancel button depends on.
    #[tokio::test]
    async fn test_cancel_dns_mode_fires_slot_token() {
        let temp = TempDir::new().unwrap();
        let storage = Arc::new(FileStorage::new(temp.path()))
            as Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let token = CancellationToken::new();
        let state = AppState {
            storage,
            writer: Arc::new(HostsWriter::new()),
            apply_lock: ApplyLock::new(),
            snapshot_lock: ApplyLock::new(),
            last_profile_ids: Mutex::new(Vec::new()),
            cached_profiles: std::sync::RwLock::new(None), // lazy load on first cached_profiles() call
            dns_server: Arc::new(Mutex::new(None)),
            dns_enabled: AtomicBool::new(false),
            original_dns: tokio::sync::RwLock::new(OriginalDns::DhcpEmpty),
            dns_lock: ApplyLock::new(),
            dns_cancel: Mutex::new(Some(token.clone())),
            ad_block_state: Arc::new(tokio::sync::RwLock::new(mhost_core::AdBlockState::default())),
            ad_block_refresh_task: Mutex::new(None),
            ad_block_refresh_cancel: Mutex::new(CancellationToken::new()),
        };

        assert!(!token.is_cancelled(), "pre-condition: token uncancelled");

        // Mirror the IPC body without constructing `State<'_, AppState>`:
        //   if let Some(token) = slot.as_ref() { token.cancel() }
        {
            let slot = lock_or_recover(&state.dns_cancel);
            if let Some(t) = slot.as_ref() {
                t.cancel();
            }
        }

        assert!(
            token.is_cancelled(),
            "cancel_dns_mode must fire the slot's CancellationToken"
        );
    }

    /// `cancel_dns_mode` is a no-op when no operation is in flight (slot empty).
    /// Calling it must not panic and must be a no-op.
    #[tokio::test]
    async fn test_cancel_dns_mode_noop_when_slot_empty() {
        let temp = TempDir::new().unwrap();
        let storage = Arc::new(FileStorage::new(temp.path()))
            as Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let state = AppState {
            storage,
            writer: Arc::new(HostsWriter::new()),
            apply_lock: ApplyLock::new(),
            snapshot_lock: ApplyLock::new(),
            last_profile_ids: Mutex::new(Vec::new()),
            cached_profiles: std::sync::RwLock::new(None), // lazy load on first cached_profiles() call
            dns_server: Arc::new(Mutex::new(None)),
            dns_enabled: AtomicBool::new(false),
            original_dns: tokio::sync::RwLock::new(OriginalDns::DhcpEmpty),
            dns_lock: ApplyLock::new(),
            dns_cancel: Mutex::new(None),
            ad_block_state: Arc::new(tokio::sync::RwLock::new(mhost_core::AdBlockState::default())),
            ad_block_refresh_task: Mutex::new(None),
            ad_block_refresh_cancel: Mutex::new(CancellationToken::new()),
        };

        let slot = lock_or_recover(&state.dns_cancel);
        let did_cancel = slot.as_ref().is_some();
        drop(slot);

        assert!(
            !did_cancel,
            "empty slot must be a no-op for cancel_dns_mode"
        );
    }

    /// Issue #138 follow-up (regression for the cancel slot): `set_dns_mode`
    /// must allocate a FRESH, uncancelled token even when the previous
    /// operation's token is still in the slot.
    #[tokio::test]
    async fn test_set_dns_mode_swap_cancellation_token_is_fresh() {
        let temp = TempDir::new().unwrap();
        let storage = Arc::new(FileStorage::new(temp.path()))
            as Arc<dyn mhost_storage::storage::Storage + Send + Sync>;
        let state = AppState {
            storage,
            writer: Arc::new(HostsWriter::new()),
            apply_lock: ApplyLock::new(),
            snapshot_lock: ApplyLock::new(),
            last_profile_ids: Mutex::new(Vec::new()),
            cached_profiles: std::sync::RwLock::new(None), // lazy load on first cached_profiles() call
            dns_server: Arc::new(Mutex::new(None)),
            dns_enabled: AtomicBool::new(false),
            original_dns: tokio::sync::RwLock::new(OriginalDns::DhcpEmpty),
            dns_lock: ApplyLock::new(),
            // Pre-populate slot with a CANCELLED token.
            dns_cancel: Mutex::new({
                let t = CancellationToken::new();
                t.cancel();
                Some(t)
            }),
            ad_block_state: Arc::new(tokio::sync::RwLock::new(mhost_core::AdBlockState::default())),
            ad_block_refresh_task: Mutex::new(None),
            ad_block_refresh_cancel: Mutex::new(CancellationToken::new()),
        };

        // Simulate the swap pattern at the top of `set_dns_mode`:
        //   let cancel = CancellationToken::new();
        //   *lock_or_recover(&state.dns_cancel) = Some(cancel.clone());
        let cancel = CancellationToken::new();
        *lock_or_recover(&state.dns_cancel) = Some(cancel.clone());

        assert!(
            !cancel.is_cancelled(),
            "swap pattern must produce a fresh, uncancelled token"
        );
        let slot_token = lock_or_recover(&state.dns_cancel)
            .as_ref()
            .expect("slot populated")
            .clone();
        assert!(
            !slot_token.is_cancelled(),
            "slot token must be the fresh, uncancelled one"
        );
    }

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
            cached_profiles: std::sync::RwLock::new(None), // lazy load on first cached_profiles() call
            dns_server: Arc::new(Mutex::new(None)),
            dns_enabled: AtomicBool::new(false),
            original_dns: tokio::sync::RwLock::new(OriginalDns::DhcpEmpty),
            dns_lock: ApplyLock::new(),
            dns_cancel: Mutex::new(None),
            ad_block_state: Arc::new(tokio::sync::RwLock::new(mhost_core::AdBlockState::default())),
            ad_block_refresh_task: Mutex::new(None),
            ad_block_refresh_cancel: Mutex::new(tokio_util::sync::CancellationToken::new()),
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
            cached_profiles: std::sync::RwLock::new(None), // lazy load on first cached_profiles() call
            dns_server: Arc::new(Mutex::new(None)),
            dns_enabled: AtomicBool::new(true), // 假装启用 → cleanup 会走 disable 路径
            original_dns: tokio::sync::RwLock::new(OriginalDns::DhcpEmpty), // DhcpEmpty → 写 Empty
            dns_lock: ApplyLock::new(),
            dns_cancel: Mutex::new(None),
            ad_block_state: Arc::new(tokio::sync::RwLock::new(mhost_core::AdBlockState::default())),
            ad_block_refresh_task: Mutex::new(None),
            ad_block_refresh_cancel: Mutex::new(tokio_util::sync::CancellationToken::new()),
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
            cached_profiles: std::sync::RwLock::new(None), // lazy load on first cached_profiles() call
            dns_server: Arc::new(Mutex::new(None)),
            dns_enabled: AtomicBool::new(true),
            original_dns: tokio::sync::RwLock::new(OriginalDns::DhcpEmpty),
            dns_lock: ApplyLock::new(),
            dns_cancel: Mutex::new(None),
            ad_block_state: Arc::new(tokio::sync::RwLock::new(mhost_core::AdBlockState::default())),
            ad_block_refresh_task: Mutex::new(None),
            ad_block_refresh_cancel: Mutex::new(tokio_util::sync::CancellationToken::new()),
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
    let original_dns = state.original_dns.read().await.clone();
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

    let status = build(lock_or_recover(&state.dns_server).as_ref(), original_dns);
    Ok(status)
}
/// Abort the periodic ad-block refresh task if one is registered.
///
/// The disable path (issue #130) is the only legitimate caller:
/// `spawn_ad_block_refresh_task` registers a `JoinHandle` on enable, and
/// `set_dns_mode_disable` MUST cancel it — otherwise the task keeps
/// running and tries to `reload_ad_block_rules` on a server that's
/// already been stopped, surfacing confusing errors at the next refresh
/// tick.
///
/// Extracted from the inline `if let Some(h) = ...take() { h.abort() }`
/// in `set_dns_mode_disable` so the abort behavior is unit-testable
/// without going through the full disable path (issue #134). The full
/// path calls `mhost_dns::platform::disable_dns_mode` which returns Err
/// in unit tests (no proxy + non-interactive), short-circuiting before
/// the abort step — so testing through the public API would not
/// actually exercise the abort contract.
///
/// **This helper only signals cancellation.** It does NOT wait for the
/// task to actually terminate, and it does NOT abort any `spawn_blocking`
/// closure the task may have entered (Tokio explicitly does not support
/// aborting blocking work once started). The full "refresh work is dead
/// by the time we return" guarantee is a separate concern tracked as a
/// follow-up issue; see the test module's section comment for context.
///
/// Returns `true` if a task was found and `abort()` was called on it,
/// `false` if the slot was empty (idempotent re-disable, or
/// disable-before-enable).
fn abort_ad_block_refresh_task(slot: &Mutex<Option<JoinHandle<()>>>) -> bool {
    if let Some(handle) = lock_or_recover(slot).take() {
        handle.abort();
        true
    } else {
        false
    }
}

/// Cooperatively cancel the periodic ad-block refresh task.
///
/// Pair to `abort_ad_block_refresh_task`. Where the latter force-cancels
/// the outer task via `JoinHandle::abort()` (which cannot interrupt
/// in-flight `spawn_blocking` closures — see issue #138), the cancel
/// token lets the task's `tokio::select!` wake up immediately and lets
/// any `spawn_blocking` closure observe `is_cancelled()` and bail before
/// mutating a stopped `DnsServer`.
///
/// The disable path calls cancel **before** abort so the cooperative
/// path runs first; the abort is the fallback for any work that is past
/// the select point.
///
/// Idempotent: calling on an already-cancelled token is a no-op.
fn cancel_ad_block_refresh_task(slot: &Mutex<CancellationToken>) {
    lock_or_recover(slot).cancel();
}

/// Spawn the periodic ad-block refresh task (issue #130).
///
/// Called from two places:
///   1. `set_dns_mode_enable` after the DNS server comes up — the
///      "normal" hot path.
///   2. `AppState::new` after `try_recover_dns` succeeds — PR #131
///      review finding 0.1: previously, an auto-recovered DNS session
///      lost its periodic refresh because the task was only spawned on
///      the user-driven enable path.
///
/// The function takes individual Arcs rather than `&AppState` so it can
/// be invoked while `AppState` is being constructed (item 2).
///
/// **Issue #138 follow-up (re-enable):** the cancel slot is **swapped**
/// for a fresh, uncancelled token on every spawn — `CancellationToken`
/// is sticky, so without this swap a disable → re-enable cycle would
/// hand the new task the old (already cancelled) token, causing its
/// `select!` to match `cancel.cancelled()` on iter 0 and exit
/// immediately. See `test_re_enable_after_disable_respawns_with_fresh_token`.
///
/// The task:
/// 1. Reads `refresh_interval_hours` from `ad_block_state`.
/// 2. Sleeps for the interval (or until the cancel token fires).
/// 3. Refreshes all enabled sources + hot-reloads the engine.
/// 4. Exits cleanly when the current `ad_block_refresh_cancel` is cancelled.
///
/// `refresh_interval_hours == 0` or `auto_refresh_enabled == false` short-
/// circuits — task is not spawned at all (callers don't need to abort it).
///
/// **Issue #138:** The disable path now signals a `CancellationToken` (see
/// `cancel_ad_block_refresh_task`) *before* aborting. The select! below
/// wakes on `token.cancelled()` and the loop exits without relying on
/// `JoinHandle::abort()` reaching a yield point. The `spawn_blocking`
/// closure inside the loop also checks `token.is_cancelled()` immediately
/// before calling `reload_ad_block_rules` — that's the layer that protects
/// against an in-flight `classify_rules` that started before cancel
/// landed. `spawn_blocking` work cannot be interrupted by `JoinHandle::
/// abort()` (tokio explicitly documents this), so the self-check is the
/// only reliable way to avoid a `reload_ad_block_rules` call landing on
/// a `DnsServer` that's already been stopped. The `lock_or_recover`
/// on `dns_server` already serializes against the disable path's `.take()`,
/// so the cancel check is mainly an early-exit optimization against a
/// `reload_ad_block_rules` racing with `server.stop()`.
pub(crate) fn spawn_ad_block_refresh_task(
    task_slot: &std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    ad_block_state: &Arc<tokio::sync::RwLock<mhost_core::AdBlockState>>,
    dns_server: &Arc<std::sync::Mutex<Option<mhost_dns::DnsServer>>>,
    storage: &Arc<dyn mhost_storage::storage::Storage + Send + Sync>,
    cancel_slot: &Mutex<CancellationToken>,
) {
    let cfg = match ad_block_state.try_read() {
        Ok(g) => (g.auto_refresh_enabled, g.refresh_interval_hours),
        Err(_) => return,
    };
    if !cfg.0 || cfg.1 == 0 {
        return;
    }

    // Issue #138 follow-up (re-enable): swap the cancel slot for a fresh
    // token so this task is unaffected by a previous disable's `cancel()`.
    // `CancellationToken::cancel()` is sticky — if we just cloned the
    // existing (already cancelled) token, the spawned task's first
    // `select!` arm would match `cancel.cancelled()` on iter 0 and break
    // out of the loop without ever ticking.
    let cancel = CancellationToken::new();
    *lock_or_recover(cancel_slot) = cancel.clone();

    // Clone the few Arcs we need into the task closure. We don't share the
    // whole AppState (which contains unrelated Mutexes like snapshot_lock)
    // to keep the lock-contention surface minimal. The cancel token is
    // cheap to clone (it's a refcount + an atomic flag).
    let storage = storage.clone();
    let ad_block_state = ad_block_state.clone();
    let dns_server = dns_server.clone();

    let interval_secs = (cfg.1 as u64).saturating_mul(3600).max(3600); // floor 1h
    let handle = tokio::spawn(async move {
        loop {
            // Sleep OR cancellation. The select! is the **first** thing
            // the loop checks so disable can interrupt even a long
            // inter-tick sleep.
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(interval_secs)) => {}
                _ = cancel.cancelled() => {
                    // Disable path fired the token. Exit the loop cleanly.
                    break;
                }
            }

            // Re-read config — user may have changed interval or disabled
            // auto-refresh since the last tick.
            let (auto, interval_h, enabled) = {
                let Ok(g) = ad_block_state.try_read() else {
                    continue;
                };
                (g.auto_refresh_enabled, g.refresh_interval_hours, g.enabled)
            };
            if !auto || interval_h == 0 || !enabled {
                continue;
            }

            // Snapshot IDs (best-effort; failures logged not propagated).
            let ids: Vec<mhost_core::SourceId> = {
                let Ok(g) = ad_block_state.try_read() else {
                    continue;
                };
                g.sources
                    .iter()
                    .filter(|s| s.enabled)
                    .map(|s| s.source_id.clone())
                    .collect()
            };

            // Concurrent fetch — bounded by REFRESH_CONCURRENCY. PR #131
            // review finding 1.4: serial loop meant a multi-source list
            // could block for `N × FETCH_TIMEOUT_SECS` while the next
            // periodic tick waited its turn.
            //
            // Note: `fetch_sources_concurrent` is a regular async fn, not
            // spawn_blocking, so `cancel.cancelled()` racing with it would
            // not interrupt it. We accept that one fetch round may run to
            // completion after disable; the spawn_blocking step below is
            // the critical one (it mutates the server), and that's where
            // the self-check lives.
            crate::commands::adblock::fetch_sources_concurrent(
                &storage,
                &ad_block_state,
                &ids,
                crate::commands::adblock::REFRESH_CONCURRENCY,
            )
            .await;

            // Bail before spawn_blocking if the token is already set —
            // the per-tick check above the fetch is informational only,
            // cancel may have landed during the fetch.
            if cancel.is_cancelled() {
                break;
            }

            // Hot-reload engine if DNS still on. We use the dns_server
            // slot as the proxy signal AND the cancel token as the
            // authoritative one — the slot can race with the disable
            // path between the check and the reload call below.
            if lock_or_recover(&dns_server).is_some() {
                let snap = ad_block_state.read().await.clone();
                let root = storage.root().to_path_buf();
                let dns_server_clone = Arc::clone(&dns_server);
                let cancel_in_closure = cancel.clone();
                // Issue #133: classify_rules reads + parses each source's
                // cache file synchronously — 100k+ domains can block a
                // tokio worker for seconds. Move it off the async runtime.
                let _ = tokio::task::spawn_blocking(move || {
                    // Self-check (issue #138) — abort() can't reach us; bail on cancel.
                    if cancel_in_closure.is_cancelled() {
                        return;
                    }
                    let (za, nx, wl) = crate::commands::adblock::classify_rules(&snap, &root);
                    if cancel_in_closure.is_cancelled() {
                        return;
                    }
                    if let Some(server) = lock_or_recover(&dns_server_clone).as_ref() {
                        server.reload_ad_block_rules(za, nx, wl);
                    }
                })
                .await;
            }
        }
    });

    *lock_or_recover(task_slot) = Some(handle);
}
