//! macOS platform adapters.
//!
//! Provides native macOS integration via objc2, such as
//! controlling the application's Dock/activation policy
//! and intercepting Cmd-Q quit requests via a custom
//! NSApplicationDelegate.

#[cfg(target_os = "macos")]
use objc2::declare::ClassBuilder;
#[cfg(target_os = "macos")]
use objc2::ffi::NSUInteger;
#[cfg(target_os = "macos")]
use objc2::runtime::{AnyObject, Sel};
#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2::{class, msg_send, sel};
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicPtr, Ordering};

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
        eprintln!(
            "[mHost] Warning: set_activation_policy_accessory called from non-main thread, skipped"
        );
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
        eprintln!(
            "[mHost] Warning: set_activation_policy_regular called from non-main thread, skipped"
        );
    }
}

// ===========================================================================
// **fix issue #100**: macOS Cmd-Q quit interception
//
// Problem: Tauri 2's NSApplicationDelegate (in tao) does NOT fire
// `RunEvent::ExitRequested` on Cmd-Q. applicationShouldTerminate:
// returns NSTerminateNow directly, so our Tauri cleanup hook never
// gets a chance to run. The app terminates with system DNS still
// pointing at 127.0.0.1.
//
// Fix: install a custom NSApplicationDelegate via objc2 ClassBuilder.
// On applicationShouldTerminate:, we synchronously run cleanup via
// tauri::async_runtime::block_on, then return NSTerminateNow to
// allow the OS to terminate the process.
//
// The cleanup function pointer is stored in an AtomicPtr that's set by
// install_quit_handler() at app setup. We use a function pointer (not a
// closure) so the C-ABI extern "C" fn can invoke it.
//
// Tauri's own NSApplicationDelegate is replaced. tao's only delegate
// method on macOS is applicationShouldTerminate: (which we override)
// plus empty will/did-finish-launching stubs (NSApp will call those on
// our delegate instead — harmless no-ops).
// ===========================================================================

#[cfg(target_os = "macos")]
type CleanupFn = unsafe extern "C" fn();

#[cfg(target_os = "macos")]
static CLEANUP_FN: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());

#[cfg(target_os = "macos")]
extern "C" fn application_should_terminate(
    _this: *mut AnyObject,
    _cmd: Sel,
    _sender: *mut AnyObject,
) -> NSUInteger {
    eprintln!("[mHost] macos quit: applicationShouldTerminate: intercepted");
    let cleanup = CLEANUP_FN.load(Ordering::Acquire);
    if !cleanup.is_null() {
        let f: CleanupFn = unsafe { std::mem::transmute(cleanup) };
        unsafe { f() };
    }
    // NSTerminateNow == 1, allow the OS to proceed with termination
    1
}

/// Install a custom NSApplicationDelegate that intercepts Cmd-Q (and any
/// other quit request that goes through applicationShouldTerminate:) and
/// runs the provided cleanup function synchronously before allowing the
/// OS to terminate the app.
///
/// **Must be called from the main thread, after Tauri has set its own
/// delegate** (i.e. from within `Builder::setup`).
#[cfg(target_os = "macos")]
pub fn install_quit_handler(cleanup: CleanupFn) {
    let mtm = match MainThreadMarker::new() {
        Some(m) => m,
        None => {
            eprintln!("[mHost] Warning: install_quit_handler called from non-main thread, skipped");
            return;
        }
    };

    CLEANUP_FN.store(cleanup as *mut (), Ordering::Release);

    let superclass = class!(NSObject);
    let mut builder = match ClassBuilder::new(c"MhostQuitHandlerDelegate", superclass) {
        Some(b) => b,
        None => {
            eprintln!("[mHost] Warning: ClassBuilder::new failed");
            return;
        }
    };

    // application_should_terminate is `extern "C" fn` (not unsafe), but
    // ClassBuilder's add_method takes a function matching the type
    // encoding. Coerce via fn pointer cast.
    let imp: extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> NSUInteger =
        application_should_terminate;
    unsafe {
        builder.add_method::<AnyObject, _>(sel!(applicationShouldTerminate:), imp);
    }

    let cls = builder.register();
    let delegate: *mut AnyObject = unsafe { msg_send![cls, new] };
    let app = NSApplication::sharedApplication(mtm);
    let _: () = unsafe { msg_send![&*app, setDelegate: delegate] };
    eprintln!("[mHost] macos quit handler installed");
}
