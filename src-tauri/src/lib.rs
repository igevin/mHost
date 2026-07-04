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
    // Tauri 2 提供 `RunEvent::ExitRequested` 在退出前回调，调用
    // `api.prevent_exit()` 可以阻止退出、做 async 清理、然后 `app.exit()`
    // 放行。窗口关闭（WindowEvent::CloseRequested）已被拦截为 hide，
    // 所以这个钩子只在用户真正退出时触发（tray "退出"、Cmd-Q、
    // OS 关机等）。
    //
    // 关键：清理失败也必须放行退出，否则用户被卡死。
    app.run(|app_handle, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            api.prevent_exit();
            let handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(state) = handle.try_state::<AppState>() {
                    // AppState 是 'static（通过 .manage() 注入），inner() 返回 &AppState
                    if let Err(e) = commands::dns::cleanup_dns_on_exit(state.inner()).await {
                        eprintln!("[mHost] DNS cleanup on exit failed: {}", e);
                    }
                }
                handle.exit(0);
            });
        }
    });
}
