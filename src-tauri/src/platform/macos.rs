//! macOS platform adapters.
//!
//! Provides native macOS integration via objc2, such as
//! controlling the application's Dock/activation policy.

#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

/// Hide the app from the Dock (accessory policy).
///
/// Call this when the main window is hidden so the user
/// can only restore the window via the menu bar tray icon.
#[cfg(target_os = "macos")]
pub fn set_activation_policy_accessory() {
    if let Some(mtm) = MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    } else {
        eprintln!("[mHost] Warning: set_activation_policy_accessory called from non-main thread, skipped");
    }
}

/// Show the app in the Dock (regular policy).
///
/// Call this when the main window is restored so the Dock
/// icon reappears and the app behaves as a normal windowed app.
#[cfg(target_os = "macos")]
pub fn set_activation_policy_regular() {
    if let Some(mtm) = MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    } else {
        eprintln!("[mHost] Warning: set_activation_policy_regular called from non-main thread, skipped");
    }
}
