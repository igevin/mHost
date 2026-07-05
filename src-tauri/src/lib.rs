pub mod commands;
pub mod platform;
pub mod state;
#[cfg(target_os = "macos")]
pub mod tray;
pub mod tray_logic;

use commands::{apply::*, dns::*, profile::*, profile_io::*, snapshot::*, validate::*};
use state::AppState;
use tauri::{Manager, RunEvent};

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
                        if let Err(e) = commands::dns::cleanup_dns_on_exit(state.inner()).await {
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
            eprintln!("[mHost] exit: Tauri ExitRequested, cleaning up DNS");
            api.prevent_exit();
            let handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(state) = handle.try_state::<AppState>() {
                    if let Err(e) = commands::dns::cleanup_dns_on_exit(state.inner()).await {
                        eprintln!("[mHost] DNS cleanup on Tauri exit failed: {}", e);
                    }
                }
                // fix (bug 3): handle.exit(0) 在某些 tao 版本下不真正
                // 终止进程。400ms 后若还活着就强退（与 Path B 一致）。
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    std::process::exit(0);
                });
                handle.exit(0);
            });
        }
    });
}
