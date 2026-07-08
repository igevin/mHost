pub mod commands;
pub mod platform;
pub mod state;
#[cfg(target_os = "macos")]
pub mod tray;
pub mod tray_logic;

use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use commands::{apply::*, dns::*, profile::*, profile_io::*, snapshot::*, validate::*};
use state::AppState;
use tauri::{Manager, RunEvent};

/// **fix issue #67 bug 3**: 防止 RunEvent::ExitRequested 递归触发 cleanup。
///
/// `handle.exit(0)` 会再次触发 ExitRequested → cleanup → handle.exit(0)
/// 死循环，stderr 刷几百条「exit: Tauri ExitRequested」。首次 ExitRequested
/// 把这个标志置 true，后续直接 bail（仍然 prevent_exit()）。
static EXIT_CLEANUP_STARTED: AtomicBool = AtomicBool::new(false);

/// **fix issue #100**: macOS Cmd-Q quit interception
///
/// `applicationShouldTerminate:` (called by NSApp on Cmd-Q) doesn't fire
/// `RunEvent::ExitRequested`. Tauri's tao delegate returns NSTerminateNow
/// immediately. So we install a custom NSApplicationDelegate (via objc2
/// in `platform::macos::install_quit_handler`) that calls this cleanup
/// function synchronously, then returns NSTerminateNow.
///
/// `extern "C" fn` so it can be passed across the FFI boundary. Stores
/// the Tauri AppHandle pointer in `MACOS_QUIT_CLEANUP_HANDLE` at setup
/// time and reads it here.
#[cfg(target_os = "macos")]
static MACOS_QUIT_CLEANUP_HANDLE: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());

#[cfg(target_os = "macos")]
unsafe extern "C" fn macos_quit_cleanup() {
    eprintln!("[mHost] macos_quit_cleanup: running DNS cleanup before terminate");
    let handle_ptr = MACOS_QUIT_CLEANUP_HANDLE.load(Ordering::Acquire);
    if handle_ptr.is_null() {
        eprintln!("[mHost] macos_quit_cleanup: no app handle stored, skipping");
        return;
    }
    // SAFETY: the pointer was set in setup() via `Box::leak(Box::new(handle))`.
    // `Box::into_raw` returns `*mut T` (not `*mut Box<T>`), so the pointer
    // directly addresses an `AppHandle<Wry>` value, properly aligned and
    // valid for the program's lifetime.
    let handle: &tauri::AppHandle<tauri::Wry> =
        &*(handle_ptr as *const tauri::AppHandle<tauri::Wry>);
    if let Some(state) = handle.try_state::<AppState>() {
        let result = tauri::async_runtime::block_on(async move {
            commands::dns::cleanup_dns_on_exit(state.inner(), true).await
        });
        if let Err(e) = result {
            eprintln!("[mHost] macos_quit_cleanup: cleanup failed: {}", e);
        }
    } else {
        eprintln!("[mHost] macos_quit_cleanup: AppState not found, skipping");
    }
}

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
            {
                // **fix issue #100**: store app handle + install Cmd-Q interceptor
                //
                // Use `Box::leak` to put the AppHandle on the heap with a
                // stable address for the program's lifetime. Storing a
                // reference to a stack local (`&handle`) would dangle once
                // `setup` returns — that's what caused the SIGSEGV crash
                // reported by the user.
                let handle_box = Box::new(app.handle().clone());
                let handle_ptr = Box::into_raw(handle_box) as *mut ();
                MACOS_QUIT_CLEANUP_HANDLE.store(handle_ptr, Ordering::Release);
                crate::platform::macos::install_quit_handler(macos_quit_cleanup);

                if let Err(e) = crate::tray::build_tray(app.handle()) {
                    eprintln!("[mHost] Failed to build tray: {}", e);
                }
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
                        // **fix (Ctrl+C / pnpm tauri dev 也要 sudo fallback)**：
                        // SIGINT/SIGTERM 通常来自用户的 Ctrl+C 或 kill（用户在场），
                        // → `interactive=true` 让 proxy 没自恢复时走 osascript 兜底。
                        // OS 关机场景下 sudo 弹窗会短暂无人响应，但 recovery marker
                        // 留给下次启动 `try_recover_dns` 兜底。
                        if let Err(e) =
                            commands::dns::cleanup_dns_on_exit(state.inner(), true).await
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

            // Intercept window close to hide instead of exit.
            // **fix issue #100**: with `install_quit_handler` installed above
            // (custom NSApplicationDelegate via objc2 that intercepts
            // applicationShouldTerminate: synchronously), Cmd-Q cleanup
            // works regardless of activation policy. We can re-enable
            // Accessory mode on window hide so the app stays tray-only.
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
            eprintln!("[mHost] exit: Tauri ExitRequested, cleaning up DNS on std::thread");
            api.prevent_exit();
            // **fix (Cmd-Q + tray Quit + 所有 ExitRequested 路径)**：
            //
            // 之前两版尝试都不工作：
            //   - `tauri::async_runtime::block_on(cleanup_and_exit)` 在
            //     Cmd-Q 路径下不跑完（怀疑 tao 主线程 + tokio runtime
            //     race，或 macOS tray-only 模式下 prevent_exit 被忽略）。
            //   - `libc::kill(self, SIGTERM)` 转发到 SIGINT/SIGTERM handler：
            //     Ctrl+C 路径下因为 handle.exit(0) 二次触发 ExitRequested
            //     才走这条路径，所以"看起来"工作了；Cmd-Q 路径下 SIGTERM
            //     转发可能不可靠。
            //
            // 现在用 std::thread::spawn 在独立 OS 线程上跑 cleanup：
            //   - 完全脱离 tao 主线程 + Tauri runtime
            //   - 用 tauri::async_runtime::block_on 跑 async cleanup
            //     （与 tray Quit 同一机制，但不在 tao 线程上调用）
            //   - cleanup 完成后 handle.exit(0) → 触发 ExitRequested 二次
            //     → 递归守卫拦 → 400ms watchdog 兜底
            let handle = app_handle.clone();
            std::thread::spawn(move || {
                let cleanup_handle = handle.clone();
                let result = tauri::async_runtime::block_on(async move {
                    if let Some(state) = cleanup_handle.try_state::<AppState>() {
                        commands::dns::cleanup_dns_on_exit(state.inner(), true).await
                    } else {
                        Ok(())
                    }
                });
                if let Err(e) = result {
                    eprintln!("[mHost] DNS cleanup on Tauri exit failed: {}", e);
                }
                // 400ms watchdog：handle.exit(0) 在某些 tao 版本下不真正终止进程
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    eprintln!("[mHost] graceful exit timed out, force-exiting process");
                    std::process::exit(0);
                });
                handle.exit(0);
            });
        }
    });
}
