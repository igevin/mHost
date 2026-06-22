//! macOS tray menu implementation.
//!
//! This module is only compiled on macOS. It provides the system tray icon,
//! menu, and event handling for profile switching via the menu bar.
#![cfg(target_os = "macos")]

use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Runtime,
};

use crate::commands::apply::enable_and_apply_logic;
use crate::state::AppState;
use crate::tray_logic;
use mhost_core::ProfileId;

const TRAY_ID: &str = "main-tray";
const PROFILES_SUBMENU_ID: &str = "profiles_submenu";

/// Re-export from tray_logic to ensure single source of truth.
pub use crate::tray_logic::PROFILE_ID_PREFIX;

/// Event emitted when tray profile selection changes.
///
/// Payload: `()` (empty tuple). Frontend should refresh profile list.
/// Emitted after a successful profile switch via the tray menu.
pub const TRAY_PROFILES_UPDATED_EVENT: &str = "tray:profiles-updated";

/// Build the initial tray icon and menu.
pub fn build_tray<R: Runtime>(app: &AppHandle<R>) -> Result<(), Box<dyn std::error::Error>> {
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-icon.png"))?;

    let menu = build_menu(app)?;

    let tooltip = build_tooltip(app);

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(true)
        .tooltip(tooltip)
        .menu(&menu)
        .on_menu_event(|app, event| handle_menu_event(app, event))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

/// Build the tray menu from current profiles.
fn build_menu<R: Runtime>(app: &AppHandle<R>) -> Result<Menu<R>, Box<dyn std::error::Error>> {
    let state = app.state::<AppState>();
    let profiles = state.storage.list_profiles()?;

    // Perf fix (#29): Track last rendered profile IDs
    let profile_ids: Vec<String> = profiles.iter().map(|p| p.id.to_string()).collect();
    if let Ok(mut last) = state.last_profile_ids.lock() {
        *last = profile_ids;
    }

    // Build profile check menu items
    let mut profile_items: Vec<CheckMenuItem<R>> = Vec::new();
    for p in &profiles {
        let id = format!("{}{}", PROFILE_ID_PREFIX, p.id);
        let item = CheckMenuItem::with_id(
            app,
            id,
            &p.name,
            true,
            p.enabled,
            None::<&str>,
        )?;
        profile_items.push(item);
    }

    let profile_refs: Vec<&dyn tauri::menu::IsMenuItem<R>> = profile_items
        .iter()
        .map(|item| item as &dyn tauri::menu::IsMenuItem<R>)
        .collect();

    let profiles_submenu = Submenu::with_id_and_items(
        app,
        PROFILES_SUBMENU_ID,
        "环境配置",
        true,
        &profile_refs,
    )?;

    let sep1 = PredefinedMenuItem::separator(app)?;
    let adblock = MenuItem::with_id(app, "adblock", "广告屏蔽（即将推出）", false, None::<&str>)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let refresh = MenuItem::with_id(app, "refresh_rules", "刷新远程规则", true, Some("CmdOrR"))?;
    let open_window = MenuItem::with_id(app, "open_window", "打开主窗口", true, Some("CmdOrO"))?;
    let sep3 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &profiles_submenu,
            &sep1,
            &adblock,
            &sep2,
            &refresh,
            &open_window,
            &sep3,
            &quit,
        ],
    )?;

    Ok(menu)
}

/// Build tooltip text from current enabled profile.
fn build_tooltip<R: Runtime>(app: &AppHandle<R>) -> String {
    let state = app.state::<AppState>();
    let profiles = match state.storage.list_profiles() {
        Ok(p) => p,
        Err(_) => return tray_logic::build_tooltip_text(None),
    };

    let enabled_name = profiles.iter().find(|p| p.enabled).map(|p| p.name.as_str());
    tray_logic::build_tooltip_text(enabled_name)
}

/// Handle tray menu events.
pub fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, event: tauri::menu::MenuEvent) {
    let action = tray_logic::resolve_menu_action(event.id.as_ref());

    match action {
        tray_logic::TrayMenuAction::SwitchProfile(profile_id) => {
            let app_clone = app.clone();
            let profile_id_clone = profile_id.clone();

            tauri::async_runtime::spawn_blocking(move || {
                let state = app_clone.state::<AppState>();
                // Security fix (#16): Acquire apply lock to prevent concurrent writes
                // Note: tray uses blocking context, so we use try_lock or a blocking approach
                let _guard = state.apply_lock.blocking_lock();
                eprintln!("[mHost] Tray: waiting for user authorization (if needed)...");
                let storage = state.storage.as_ref();
                let writer = &*state.writer;

                // Determine target enabled state: if already enabled, disable it;
                // otherwise enable it (disable others).
                let profile_id = match profile_id_clone.parse::<ProfileId>() {
                    Ok(id) => id,
                    Err(e) => {
                        // Security fix (#22): truncate profile ID to first 8 chars to prevent log injection
                        let safe_id = &profile_id_clone[..8.min(profile_id_clone.len())];
                        eprintln!("[mHost] Tray switch profile failed: invalid profile ID '{}...': {}", safe_id, e);
                        return;
                    }
                };
                let target_enabled = match storage.load_profile(&profile_id) {
                    Ok(p) => !p.enabled,
                    Err(e) => {
                        eprintln!(
                            "[mHost] Tray switch profile failed: could not load profile '{}': {}",
                            profile_id, e
                        );
                        return;
                    }
                };

                match enable_and_apply_logic(&profile_id, target_enabled, storage, writer) {
                    Ok(()) => {
                        let _ = update_tray_checkmark(&app_clone);
                        let _ = app_clone.emit(TRAY_PROFILES_UPDATED_EVENT, ());
                    }
                    Err(e) => {
                        eprintln!("[mHost] Tray switch profile failed: {}", e);
                    }
                }
            });
        }
        tray_logic::TrayMenuAction::RefreshRules => {
            // TODO: implement refresh remote rules
            println!("[mHost] Refresh rules clicked");
        }
        tray_logic::TrayMenuAction::OpenWindow => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        tray_logic::TrayMenuAction::Quit => {
            app.exit(0);
        }
        tray_logic::TrayMenuAction::AdBlock => {
            // Placeholder: ad block is coming soon
        }
        tray_logic::TrayMenuAction::Unknown => {
            println!("[mHost] Unknown tray menu action: {:?}", event.id);
        }
    }
}

/// Update only checkmark states and tooltip.
pub fn update_tray_checkmark<R: Runtime>(app: &AppHandle<R>) -> Result<(), Box<dyn std::error::Error>> {
    let tray = match app.tray_by_id(TRAY_ID) {
        Some(t) => t,
        None => return Err("tray not found".into()),
    };

    // Since TrayIcon doesn't expose menu() getter, we rebuild the menu
    // to update checkmarks when we can't access the existing items.
    let new_menu = build_menu(app)?;
    tray.set_menu(Some(new_menu))?;

    // Update tooltip
    let tooltip = build_tooltip(app);
    if let Err(e) = tray.set_tooltip(Some(tooltip)) {
        eprintln!("[mHost] Warning: failed to set tray tooltip: {}", e);
    }

    Ok(())
}

/// Full menu rebuild when profile list changes.
pub fn update_tray_menu<R: Runtime>(app: &AppHandle<R>) {
    // Read current profile IDs from existing menu
    let old_profile_ids = match get_current_profile_ids_from_menu(app) {
        Ok(ids) => ids,
        Err(_) => Vec::new(),
    };

    // Read new profile IDs from AppState
    let new_profile_ids = {
        let state = app.state::<AppState>();
        match state.storage.list_profiles() {
            Ok(profiles) => profiles.into_iter().map(|p| p.id.to_string()).collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        }
    };

    match tray_logic::determine_menu_update_kind(&old_profile_ids, &new_profile_ids) {
        tray_logic::MenuUpdateKind::CheckOnly => {
            if let Err(e) = update_tray_checkmark(app) {
                eprintln!("[mHost] Warning: failed to update tray checkmark: {}", e);
            }
        }
        tray_logic::MenuUpdateKind::Rebuild => {
            if let Err(e) = rebuild_tray_menu(app) {
                eprintln!("[mHost] Warning: failed to rebuild tray menu: {}", e);
            }
        }
    }
}

/// Rebuild the entire tray menu.
fn rebuild_tray_menu<R: Runtime>(app: &AppHandle<R>) -> Result<(), Box<dyn std::error::Error>> {
    let tray = match app.tray_by_id(TRAY_ID) {
        Some(t) => t,
        None => return Err("tray not found".into()),
    };

    let new_menu = build_menu(app)?;
    tray.set_menu(Some(new_menu))?;

    let tooltip = build_tooltip(app);
    tray.set_tooltip(Some(tooltip))?;

    Ok(())
}

/// Extract current profile IDs from the tray menu.
/// Perf fix (#29): Read from AppState instead of returning empty.
fn get_current_profile_ids_from_menu<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let state = app.state::<AppState>();
    let ids = state.last_profile_ids.lock().map_err(|e| e.to_string())?;
    Ok(ids.clone())
}
