pub mod commands;
pub mod platform;
pub mod state;

use commands::{apply::*, profile::*, profile_io::*, validate::*};
use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = match AppState::new() {
        Ok(state) => state,
        Err(e) => {
            eprintln!("[mHost] Failed to initialize AppState: {}", e);
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            list_profiles,
            get_profile,
            create_profile,
            update_profile,
            delete_profile,
            set_profile_enabled,
            generate_apply_plan,
            apply_hosts,
            rollback_hosts,
            read_system_hosts,
            validate_hosts_text,
            get_managed_block_content,
            get_last_applied,
            import_profile,
            export_profile,
            duplicate_profile,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
