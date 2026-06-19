pub mod commands;
pub mod platform;
pub mod state;

use commands::{apply::*, profile::*};
use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new())
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
