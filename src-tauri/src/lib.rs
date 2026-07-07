pub mod commands;
pub mod platform;
pub mod state;
#[cfg(target_os = "macos")]
pub mod tray;
pub mod tray_logic;

use std::sync::atomic::{AtomicBool, Ordering};

use commands::{apply::*, dns::*, profile::*, profile_io::*, snapshot::*, validate::*};
use state::AppState;
use tauri::{Manager, RunEvent};

/// **fix issue #67 bug 3**: 防止 RunEvent::ExitRequested 递归触发 cleanup。
///
/// `handle.exit(0)` 会再次触发 ExitRequested → cleanup → handle.exit(0)
/// 死循环，stderr 刷几百条「exit: Tauri ExitRequested」。首次 ExitRequested
/// 把这个标志置 true，后续直接 bail（仍然 prevent_exit()）。
static EXIT_CLEANUP_STARTED: AtomicBool = AtomicBool::new(false);

/// App 退出时的统一清理入口（fix: 用户反馈"关闭 app 后系统 DNS 没还原"）。
///
/// 三条退出路径都汇聚到这里：
///   1) tray Quit（用户在场 → `interactive=true`，proxy 死了时走
///      osascript sudo 兜底，让用户当场看到恢复成功）
///   2) Tauri `RunEvent::ExitRequested`（Cmd-Q + 兜底 → `interactive=false`，
///      用户可能不在场，marker 留给下次启动 `try_recover_dns` 兜底）
///   3) SIGINT/SIGTERM handler（Ctrl+C / kill / OS 关机 → `interactive=false`）
///
/// 与 `cleanup_dns_on_exit` 的关系：本函数**包装** `cleanup_dns_on_exit`，
/// 添加 Tauri 特定的 watchdog + `handle.exit(0)`。幂等性由
/// `cleanup_dns_on_exit` 内部的 `dns_enabled` 标志保证（重复调用 no-op）。
///
/// # 为什么调用方用 `block_on` 而不是直接 `.await`
/// RunEvent 回调（Path A）和 `handle_menu_event` 回调（tray Quit）都
/// 运行在 tao 的主事件循环，**不是** tokio task。Tauri 2 的
/// `async_runtime::block_on` 在这种同步上下文里调用是安全的（tauri 文档
/// 显式支持）。`SIGINT/SIGTERM` handler（Path B）运行在 spawn 出来的
/// tokio task 里，直接 `.await` 即可，不需要 `block_on`。
///
/// # 为什么 Path B 不走这里
/// Path B 的现有实现已经 `await cleanup_dns_on_exit` 后再 `handle.exit(0)`，
/// 本身可靠。集中到这里会引入 `handle.exit(0)` → ExitRequested 递归的
/// 额外路径（虽然 EXIT_CLEANUP_STARTED 守卫能挡住，但增加无意义的递归）。
/// 保持 Path B 内联以减少 blast radius。
async fn cleanup_and_exit<R: tauri::Runtime>(app_handle: &tauri::AppHandle<R>, interactive: bool) {
    // 1. 同步执行 DNS cleanup：写 manifest、恢复系统 DNS、停 DnsServer。
    if let Some(state) = app_handle.try_state::<AppState>() {
        if let Err(e) = commands::dns::cleanup_dns_on_exit(state.inner(), interactive).await {
            eprintln!("[mHost] DNS cleanup on exit failed: {}", e);
        }
    }
    // 2. 兜底 watchdog：在某些 tao 版本下 `handle.exit(0)` 不会真正终止进程
    //    （issue #67 bug 3）。如果 400ms 后进程还活着，强退。
    let handle = app_handle.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        eprintln!("[mHost] graceful exit timed out, force-exiting process");
        std::process::exit(0);
    });
    // 3. 优雅退出（best effort，被 watchdog 兜底）。
    handle.exit(0);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = match tauri::async_runtime::block_on(AppState::new()) {
        Ok(state) => state,
        Err(e) => {
            eprintln!("[mHost] Failed to initialize AppState: {}", e);
            std::process::exit(1);
        }
    };

    let app = match tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            list_profiles,
            get_profile,
            create_profile,
            update_profile,
            delete_profile,
            set_profile_enabled,
            enable_and_apply,
            generate_preview_plan,
            generate_apply_plan,
            apply_hosts,
            rollback_hosts,
            read_system_hosts,
            validate_hosts_text,
            validate_hosts_errors,
            get_managed_block_content,
            get_last_applied,
            import_profile,
            export_profile,
            duplicate_profile,
            export_profile_to_file,
            import_profile_from_file,
            save_snapshot,
            list_snapshots,
            load_snapshot,
            delete_snapshot,
            set_dns_mode,
            get_dns_mode,
            reload_dns_rules,
            get_dns_status,
            list_dns_profiles,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            if let Err(e) = crate::tray::build_tray(app.handle()) {
                eprintln!("[mHost] Failed to build tray: {}", e);
            }

            // 独立 signal handler —— 覆盖 Ctrl+C / kill / OS 关机等
            // 硬退出场景（Tauri 2 RunEvent::ExitRequested 在这些场景
            // 下经常不触发）。在 Tauri 自己的 tokio runtime 里 spawn，
            // 与 RunEvent 钩子互不干扰，cleanup_dns_on_exit 内部幂等。
            //
            // fix (bug 3, Ctrl+C 不退出):
            //   在某些 tao / Tauri 2 版本下，`handle.exit(0)` 不会真正
            //   终止进程 —— 表现为「Ctrl+C 之后 app 还在」。
            //   兜底：先尝试优雅退出，400ms 后若进程还活着，强退。
            let sig_app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                eprintln!(
                    "[mHost] signal handler installed (pid={})",
                    std::process::id()
                );
                #[cfg(unix)]
                {
                    use tokio::signal::unix::{signal, SignalKind};
                    let mut sigterm = match signal(SignalKind::terminate()) {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("[mHost] SIGTERM handler install failed: {}", e);
                            return;
                        }
                    };
                    let mut sigint = match signal(SignalKind::interrupt()) {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("[mHost] SIGINT handler install failed: {}", e);
                            return;
                        }
                    };
                    tokio::select! {
                        _ = sigterm.recv() => {
                            eprintln!("[mHost] exit: SIGTERM received, cleaning up DNS");
                        }
                        _ = sigint.recv() => {
                            eprintln!("[mHost] exit: SIGINT received, cleaning up DNS");
                        }
                    }
                    if let Some(state) = sig_app_handle.try_state::<AppState>() {
                        if let Err(e) =
                            commands::dns::cleanup_dns_on_exit(state.inner(), false).await
                        {
                            eprintln!("[mHost] DNS cleanup on signal failed: {}", e);
                        }
                    }
                    // 兜底 force-exit：handle.exit(0) 在某些 tao 版本下
                    // 不真正终止进程。优雅退出 + 400ms 后强退。
                    let graceful = sig_app_handle.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(400));
                        eprintln!("[mHost] graceful exit timed out, force-exiting process");
                        std::process::exit(0);
                    });
                    graceful.exit(0);
                }
                #[cfg(not(unix))]
                {
                    // 非 Unix 平台（理论上用不到，mhost 是 macOS-only），
                    // 仅作占位。
                    let _ = sig_app_handle;
                }
            });

            // Intercept window close to hide instead of exit
            if let Some(window) = app.get_webview_window("main") {
                let handle = app.handle().clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        if let Some(window) = handle.get_webview_window("main") {
                            let _ = window.hide();
                            #[cfg(target_os = "macos")]
                            crate::platform::macos::set_activation_policy_accessory();
                        }
                    }
                });
            }

            Ok(())
        })
        .build(tauri::generate_context!())
    {
        Ok(app) => app,
        Err(e) => {
            eprintln!("[mHost] Tauri application build error: {}", e);
            std::process::exit(1);
        }
    };

    // fix: 用户反馈"退出后 DNS 出问题"
    //
    // 双保险退出清理：
    //   A) Tauri 2 `RunEvent::ExitRequested` 钩子 —— 覆盖 tray 退出 /
    //      Cmd-Q 等正常退出（窗口关闭已被 prevent_close 拦截为 hide）
    //   B) setup() 里 spawn 的 tokio signal handler（SIGINT/SIGTERM）——
    //      覆盖 Ctrl+C、kill、OS 关机等硬退出。Tauri 2 在这些场景下
    //      RunEvent::ExitRequested 经常不触发。
    //
    // 两条路径 cleanup_dns_on_exit 内部幂等（dns_enabled=false 是
    // no-op），重复触发不会出问题。
    app.run(|app_handle, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            // **fix issue #67 bug 3**: 防止 ExitRequested 递归。第一次
            // ExitRequested 进来 swap 到 true 并跑 cleanup；cleanup 调
            // handle.exit(0) 会再次触发 ExitRequested，第二次 swap 后
            // 拿到 true → bail，避免 stderr 刷几百条「exit: Tauri ExitRequested」。
            if EXIT_CLEANUP_STARTED.swap(true, Ordering::SeqCst) {
                api.prevent_exit();
                return;
            }
            eprintln!("[mHost] exit: Tauri ExitRequested, cleaning up DNS");
            api.prevent_exit();
            // **fix (app-close DNS cleanup)**：之前用 `tauri::async_runtime::spawn`
            // fire-and-forget，spawn task 不一定在 Tauri tearDown 之前跑完，
            // cleanup 实际没执行就被 400ms watchdog 强退 → 系统 DNS 卡在
            // 127.0.0.1。现在用 `block_on` 同步等待 `cleanup_and_exit` 完成，
            // RunEvent 回调在 tao 主线程（不是 tokio task），block_on 安全。
            tauri::async_runtime::block_on(async move {
                cleanup_and_exit(app_handle, false).await;
            });
        }
    });
}
