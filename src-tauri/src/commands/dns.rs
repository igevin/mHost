use std::sync::atomic::Ordering;

use mhost_core::{MhostError, ProfileMode};
use tauri::State;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// DNS 模式命令
// ---------------------------------------------------------------------------

/// 启动/停止 DNS 模式。
#[tauri::command]
pub async fn set_dns_mode(enabled: bool, state: State<'_, AppState>) -> Result<(), MhostError> {
    let _guard = state.dns_lock.lock().await;

    if enabled {
        // 1. 获取当前系统 DNS 并保存
        let original = mhost_dns::platform::get_system_dns()
            .map_err(|e| MhostError::InvalidInput(format!("get system dns failed: {}", e)))?;
        // 先把 original 写到 state（in-memory），后续 enable_dns_mode 成功
        // 后还会再把同一份值持久化到 manifest —— 此刻只 clone 一份给 state。
        let original_for_state = original.clone();
        match state.original_dns.lock() {
            Ok(mut guard) => *guard = original_for_state,
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                *guard = original_for_state;
            }
        }

        // 2. 创建 DnsConfig 和 DnsServer（macOS 上使用非特权端口）
        // upstream 使用系统原始 DNS 作为兜底，确保无规则匹配时行为不变
        let upstream = {
            let system_dns = mhost_dns::platform::get_system_dns().unwrap_or_default();
            if system_dns.is_empty() {
                vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()]
            } else {
                system_dns
            }
        };
        let config = mhost_dns::DnsConfig {
            port: mhost_dns::MHOST_DNS_PORT,
            upstream,
            ..Default::default()
        };
        let dns_port = config.port;
        let server = mhost_dns::DnsServer::new(config)
            .map_err(|e| MhostError::InvalidInput(format!("dns server init failed: {}", e)))?;

        // 3. 加载所有 enabled 的 DNS 模式 Profile，reload_rules
        let profiles = state
            .storage
            .list_profiles_by_mode(ProfileMode::Dns)
            .map_err(MhostError::from)?;
        let enabled_profiles: Vec<_> = profiles.into_iter().filter(|p| p.enabled).collect();
        server.reload_rules(&enabled_profiles);

        // 4. 启动 DnsServer（spawn 到后台）
        server
            .start()
            .await
            .map_err(|e| MhostError::InvalidInput(format!("dns server start failed: {}", e)))?;

        // 5. 设置系统 DNS 和端口转发（macOS 上需要管理员权限）
        if let Err(e) = mhost_dns::platform::enable_dns_mode(dns_port) {
            let _ = server.stop().await;
            return Err(MhostError::InvalidInput(format!(
                "Failed to enable DNS mode: {}",
                e
            )));
        }

        // 6. 保存 server 实例到 state
        match state.dns_server.lock() {
            Ok(mut guard) => *guard = Some(server),
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                *guard = Some(server);
            }
        }

        // 7. 更新 manifest：写入 dns_enabled = true 并保存 original_dns。
        //    original_dns 必须在 enable_dns_mode 成功之后、save_manifest 之前写入，
        //    因为系统 DNS 一旦被改写为 127.0.0.1，崩溃后只能从 manifest 恢复原值。
        let mut manifest = state.storage.load_manifest().map_err(MhostError::from)?;
        manifest.dns_enabled = Some(true);
        manifest.original_dns = Some(original);
        state
            .storage
            .save_manifest(&manifest)
            .map_err(MhostError::from)?;

        // 8. 设置 dns_enabled = true
        state.dns_enabled.store(true, Ordering::Relaxed);
    } else {
        // 1. 恢复系统 DNS 和移除端口转发
        let original = {
            match state.original_dns.lock() {
                Ok(guard) => guard.clone(),
                Err(poisoned) => poisoned.into_inner().clone(),
            }
        };
        mhost_dns::platform::disable_dns_mode(&original)
            .map_err(|e| MhostError::InvalidInput(format!("disable dns mode failed: {}", e)))?;

        // 2. 停止 DnsServer
        let server_opt = {
            match state.dns_server.lock() {
                Ok(mut guard) => guard.take(),
                Err(poisoned) => {
                    let mut guard = poisoned.into_inner();
                    guard.take()
                }
            }
        };
        if let Some(server) = server_opt {
            server
                .stop()
                .await
                .map_err(|e| MhostError::InvalidInput(format!("dns server stop failed: {}", e)))?;
        }

        // 3. 更新 manifest.dns_enabled = false
        let mut manifest = state.storage.load_manifest().map_err(MhostError::from)?;
        manifest.dns_enabled = Some(false);
        state
            .storage
            .save_manifest(&manifest)
            .map_err(MhostError::from)?;

        // 4. 设置 dns_enabled = false
        state.dns_enabled.store(false, Ordering::Relaxed);
    }

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
    match state.dns_server.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(server) => Ok(mhost_core::DnsStatus {
                running: server.is_running(),
                port: server.port(),
                upstream: server.upstream().to_vec(),
                rule_count: server.rule_count(),
                cache_capacity: server.cache_capacity(),
            }),
            None => Ok(mhost_core::DnsStatus {
                running: false,
                port: 53,
                upstream: vec![],
                rule_count: 0,
                cache_capacity: 0,
            }),
        },
        Err(poisoned) => {
            let guard = poisoned.into_inner();
            match guard.as_ref() {
                Some(server) => Ok(mhost_core::DnsStatus {
                    running: server.is_running(),
                    port: server.port(),
                    upstream: server.upstream().to_vec(),
                    rule_count: server.rule_count(),
                    cache_capacity: server.cache_capacity(),
                }),
                None => Ok(mhost_core::DnsStatus {
                    running: false,
                    port: 53,
                    upstream: vec![],
                    rule_count: 0,
                    cache_capacity: 0,
                }),
            }
        }
    }
}
