use std::sync::atomic::Ordering;

use mhost_core::{MhostError, ProfileMode};
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
///   1. `get_system_dns()` 一次性读取 `original`
///   2. 构造 DnsConfig + 启动 DnsServer
///   3. `enable_dns_mode(port)` 修改系统 DNS（可回滚）
///   4. **持久化** manifest（dns_enabled=true, original_dns=Some(original)）
///   5. 仅在第 4 步成功后才更新 in-memory state
///
/// 停用序列与启用对称：
///   1. 读取当前 `state.original_dns`
///   2. **持久化** manifest（dns_enabled=false, original_dns 保留）
///   3. `disable_dns_mode(original)` 恢复系统 DNS
///   4. 停止 DnsServer
///   5. 仅在所有副作用成功后清空 in-memory dns_server
#[tauri::command]
pub async fn set_dns_mode(enabled: bool, state: State<'_, AppState>) -> Result<(), MhostError> {
    let _guard = state.dns_lock.lock().await;

    if enabled {
        set_dns_mode_enable(&state).await
    } else {
        set_dns_mode_disable(&state).await
    }
}

/// 启用 DNS 模式。
///
/// 失败时的回滚是**尽力而为**：每个外部副作用（bind 端口、调用 osascript、
/// 写 manifest）失败时，我们尝试撤销之前已完成的副作用。但只要成功撤销
/// 关键的「系统 DNS 改写」就算用户可恢复；端口绑定的 server 会立即 stop。
async fn set_dns_mode_enable(state: &AppState) -> Result<(), MhostError> {
    // 1. 单一来源读取：state.original_dns 和 DnsConfig.upstream 都从这一次
    //    get_system_dns() 派生，杜绝双重调用之间的 TOCTOU。
    let original = mhost_dns::platform::get_system_dns()
        .map_err(|e| MhostError::InvalidInput(format!("get system dns failed: {}", e)))?;

    let upstream = if original.is_empty() {
        // 系统的确没有配置 DNS（DHCP 没下发、用户手动清空等）—— 用公共
        // resolver 兜底，保证无规则匹配时仍能解析公网域名。
        vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()]
    } else {
        original.clone()
    };

    // 2. 构造并启动 DnsServer（macOS 上监听非特权端口 1053）
    let config = mhost_dns::DnsConfig {
        port: mhost_dns::MHOST_DNS_PORT,
        upstream,
        ..Default::default()
    };
    let dns_port = config.port;
    let server = mhost_dns::DnsServer::new(config)
        .map_err(|e| MhostError::InvalidInput(format!("dns server init failed: {}", e)))?;

    // 3. 加载已启用的 DNS 模式 Profile，注入规则
    let profiles = state
        .storage
        .list_profiles_by_mode(ProfileMode::Dns)
        .map_err(MhostError::from)?;
    let enabled_profiles: Vec<_> = profiles.into_iter().filter(|p| p.enabled).collect();
    server.reload_rules(&enabled_profiles);

    // 4. 启动 server（绑定 1053）。失败时还没有副作用，仅回滚构造。
    if let Err(e) = server.start().await {
        return Err(MhostError::InvalidInput(format!(
            "dns server start failed: {}",
            e
        )));
    }

    // 5. 启动 privileged proxy + 把系统 DNS 切到 127.0.0.1。
    //    这是不可逆的副作用；失败必须 stop server 并返回 Err。
    if let Err(e) = mhost_dns::platform::enable_dns_mode(dns_port) {
        let _ = server.stop().await;
        return Err(MhostError::InvalidInput(format!(
            "Failed to enable DNS mode: {}",
            e
        )));
    }

    // 6. **PERSIST MANIFEST FIRST** —— 持久层是 commit point。
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
        let restore_err = mhost_dns::platform::disable_dns_mode(&original);
        let _ = server.stop().await;
        return Err(match restore_err {
            Ok(_) => e,
            Err(restore) => {
                MhostError::InvalidInput(format!("{} (rollback also failed: {})", e, restore))
            }
        });
    }

    // 7. manifest 已成功落盘，现在才允许修改 in-memory state。
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
async fn set_dns_mode_disable(state: &AppState) -> Result<(), MhostError> {
    // 1. 读取 in-memory original_dns（由 enable 路径写入）
    let original = match state.original_dns.lock() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };

    // 安全保护：如果 original 是空（说明 enable 路径从未成功持久化或被
    // 外部状态破坏），不允许继续——否则会把系统 DNS 写成 "Empty" 断网。
    if original.is_empty() {
        return Err(MhostError::InvalidInput(
            "refusing to disable DNS: original_dns snapshot is empty; \
             re-enable DNS mode once to refresh the snapshot"
                .to_string(),
        ));
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
    if let Err(e) = mhost_dns::platform::disable_dns_mode(&original) {
        // 已经成功写了 manifest 标 false，所以这里只用 InvalidInput
        // 提示用户「系统 DNS 没恢复成功，需要手动检查」。
        return Err(MhostError::InvalidInput(format!(
            "Failed to restore system DNS: {}. \
             Manually run `networksetup -setdnsservers <interface> {}`",
            e,
            original.join(" ")
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

/// 获取 DNS 服务运行状态。
#[tauri::command]
pub async fn get_dns_status(
    state: State<'_, AppState>,
) -> Result<mhost_core::DnsStatus, MhostError> {
    fn build(server: Option<&mhost_dns::DnsServer>) -> mhost_core::DnsStatus {
        match server {
            Some(s) => mhost_core::DnsStatus {
                running: s.is_running(),
                port: s.port(),
                upstream: s.upstream().to_vec(),
                rule_count: s.rule_count(),
                cache_capacity: s.cache_capacity(),
            },
            None => mhost_core::DnsStatus {
                running: false,
                port: 53,
                upstream: vec![],
                rule_count: 0,
                cache_capacity: 0,
            },
        }
    }

    let status = match state.dns_server.lock() {
        Ok(guard) => build(guard.as_ref()),
        Err(poisoned) => build(poisoned.into_inner().as_ref()),
    };
    Ok(status)
}
