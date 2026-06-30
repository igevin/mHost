pub mod commands;
pub mod platform;
pub mod state;
#[cfg(target_os = "macos")]
pub mod tray;
pub mod tray_logic;

use commands::{apply::*, profile::*, profile_io::*, validate::*};
use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = match AppState::new() {
        Ok(state) => state,
        Err(e) => {
            eprintln!("[mHost] Failed to initialize AppState: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = tauri::Builder::default()
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
            list_backups,
            rollback_to_backup,
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
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            if let Err(e) = crate::tray::build_tray(&app.handle()) {
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
        .run(tauri::generate_context!())
    {
        eprintln!("[mHost] Tauri application error: {}", e);
        std::process::exit(1);
    }
}
