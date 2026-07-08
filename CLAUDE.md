# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this project is

mHost — a lightweight cross-platform Hosts manager desktop app (Tauri 2). Two modes that can coexist:

- **Hosts mode** (default): directly edits `/etc/hosts` inside a managed block (`# ---- mHost start ---- ... end ----`). First run requires one admin authorization; macOS caches it.
- **DNS mode** (v0.2+): local DNS server on `127.0.0.1:53`. Traffic is intercepted by a separate root helper `mhost-dns-proxy` and forwarded to `127.0.0.1:1053` where the Rust DNS server runs. macOS-only currently; Windows/Linux are tracked under #67.

Design principles (from `readme.md`): 快速 / 轻量 / 安全 / 不打扰 / 可理解. The product is explicitly **not** a persistent network daemon — Hosts writes happen at apply-time only, DNS mode is opt-in.

## Common commands

Package manager is **pnpm** (not npm/yarn). Node 22, Rust stable.

```bash
# install deps
pnpm install --frozen-lockfile

# frontend-only dev (Vite on :1420)
pnpm dev

# full app dev (frontend + Rust, hot reload)
pnpm tauri dev

# production build (macOS bundle; CI matrix runs aarch64 + x86_64)
pnpm tauri build

# frontend tests (Vitest)
pnpm test                    # one-shot run
pnpm test:watch              # watch mode

# frontend type-check + bundle
pnpm build                   # tsc && vite build

# Rust tests / lint / fmt (always run from src-tauri/)
cd src-tauri
cargo test --all-features -- --nocapture
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

CI runs on **macos-latest only** (`.github/workflows/ci.yml`) and requires `cargo fmt`, `cargo clippy -D warnings`, full `cargo test`, frontend `pnpm test`, `pnpm build`, and `pnpm tauri build` smoke. Push to `main` or `develop`, or open a PR against them.

## Repository layout

```txt
mHost/
├── src/                       React/TS frontend
│   ├── App.tsx, main.tsx      entry + router
│   ├── components/            Layout, ProfileCard, ManagementDrawer, RuleEditor, ApplyConfirmDialog, SnapshotPanel, ApplyStatus
│   ├── pages/                 ProfileList, ProfileView, ProfileEdit
│   ├── stores/                Jotai atoms (profiles, dns, snapshots)
│   ├── hooks/                 shared hooks
│   ├── lib/                   Tauri IPC bindings (invoke wrappers)
│   └── types/                 shared TS types
├── src-tauri/                 Rust backend (Tauri 2)
│   ├── src/
│   │   ├── lib.rs             Tauri builder + ~30 IPC commands + tray setup + double-protected exit cleanup
│   │   ├── commands/          apply.rs, dns.rs, profile.rs, profile_io.rs, snapshot.rs, validate.rs, integration_tests.rs
│   │   ├── platform/          macOS-specific (tray activation policy, etc.)
│   │   └── state/             AppState (storage, original_dns, ApplyLock, last_profile_ids)
│   └── crates/                workspace crates — pure logic, no Tauri deps
│       ├── mhost-core/        domain models, error types
│       ├── mhost-hosts/       parser / formatter / validator for hosts syntax
│       ├── mhost-storage/     FileStorage + manifest + v1→v2 migration + atomic_write
│       ├── mhost-apply/       merge / conflict / diff / writer (backup, content, verification)
│       └── mhost-dns/         DNS server, proxy (mhost-dns-proxy bin), resolver, config, platform (macOS networksetup/osascript)
├── doc/tech/                  dns-mode-tech-design.md, rust-tauri-hosts-tech-route.md, dns-mode-development-plan.md
├── doc/requirements/          feature plans and breakdowns
├── spec/                      phased delivery plans
├── perf_review_report.md      prior perf audit (15 issues #23-#37, all merged on branch fix/perf-issues-audit-2026-06)
└── PR5_CODE_REVIEW.md         review of the enable_and_apply PR
```

The architecture rule from `doc/tech/rust-tauri-hosts-tech-route.md` is enforced: **the frontend does not host core rule logic**. Parsing, merging, validation, writing, rollback all live in Rust crates; the frontend receives structured results from `commands::*` IPC handlers.

## Tauri command surface

All IPC handlers are registered in `src-tauri/src/lib.rs::run()`. Roughly six groups:

- **Profile CRUD**: `list_profiles`, `get_profile`, `create_profile`, `update_profile`, `delete_profile`, `set_profile_enabled`, `duplicate_profile`
- **Apply / rollback**: `enable_and_apply`, `generate_preview_plan`, `generate_apply_plan`, `apply_hosts`, `rollback_hosts`, `read_system_hosts`
- **Validation / introspection**: `validate_hosts_text`, `validate_hosts_errors`, `get_managed_block_content`, `get_last_applied`
- **Import / export**: `import_profile`, `export_profile`, `import_profile_from_file`, `export_profile_to_file`
- **Snapshots**: `save_snapshot`, `list_snapshots`, `load_snapshot`, `delete_snapshot`
- **DNS mode**: `set_dns_mode`, `get_dns_mode`, `reload_dns_rules`, `get_dns_status`, `list_dns_profiles`

`enable_and_apply` is the atomic primitive for "enable profile + write hosts in one transaction". `apply_lock` (in `state/mod.rs`) serializes Hosts writes. After the 2026-06 perf audit it is a `tokio::sync::Mutex` — note that it does **not** implement poison recovery (called out in `perf_review_report.md` "Problem 1"); do not panic while holding it.

## Exit / DNS cleanup contract

`src-tauri/src/lib.rs` defends DNS cleanup with **three independent paths**:

- **Tray Quit** (`src-tauri/src/tray.rs`): direct synchronous call to `commands::dns::cleanup_dns_on_exit(state, interactive=true)` before `app.exit(0)`. The `interactive=true` lets the proxy-self-restore timeout branch pop sudo when the privileged proxy is dead.
- **macOS Cmd-Q / NSApplication terminate** (`src-tauri/src/platform/macos.rs::install_quit_handler`): a runtime-built `MhostQuitHandlerDelegate` (via `objc2::declare::ClassBuilder`) overrides `applicationShouldTerminate:` to run cleanup synchronously and then return `NSTerminateNow`. Tauri's `RunEvent::ExitRequested` is **not** fired on macOS Cmd-Q (tao's NSApplicationDelegate returns NSTerminateNow directly), so this OS-level hook is the only way to catch Cmd-Q. The handle is leaked to the heap via `Box::leak(Box::new(app.handle().clone()))` and the raw `*mut AppHandle<Wry>` is stored in a global `AtomicPtr` so the `extern "C" fn` callback (which can't capture) can reach it. **Don't** store the AppHandle as `*const Box<AppHandle>` — `Box::into_raw` returns `*mut T` directly; the extra layer reads the Arc pointer as a Box pointer and panics with `misaligned_pointer_dereference`.
- **SIGINT / SIGTERM** (`src-tauri/src/lib.rs::setup`): tokio signal handlers in a `tauri::async_runtime::spawn(async move { … })` task; awaits `cleanup_dns_on_exit(state, interactive=true)` directly. Interactive sudo is still preferred here for Ctrl+C in dev (`pnpm tauri dev`), at the cost of a brief no-op sudo prompt during OS shutdown if no user is present.

All three call `commands::dns::cleanup_dns_on_exit(state, interactive)` (idempotent — `dns_enabled=false` is a no-op). A force-exit fallback kills the process after 400 ms if `handle.exit(0)` doesn't terminate (known issue in some tao versions). Any new code that may terminate the process must keep this DNS-cleanup guarantee — leaks leave the system pointing at `127.0.0.1` with no way to recover the user's original DNS.

> **Do NOT** try to handle Cmd-Q by hooking Tauri `RunEvent::ExitRequested` — Tauri 2 on macOS does not fire it for `applicationShouldTerminate:`. If you add a new exit path, hook the right OS-level event for the platform.

## Security boundaries already enforced

These are project invariants — do not weaken them:

- **Interface name whitelist** (`src-tauri/crates/mhost-dns/src/platform.rs::validate_interface_name`) — prevents osascript injection (issue #77). Both proxy.rs and platform.rs validate before any shell invocation.
- **Path validation** in `src-tauri/src/commands/profile_io.rs` constrains file I/O to the user's home directory (issue #17).
- **Atomic writes** via `tempfile::NamedTempFile::persist` (`storage::atomic_write`, `mhost-apply/writer`). Never replace with a naive `fs::write` + `rename` — orphans `.tmp` files on partial failure.
- **`ensure_regular_file`** before `/etc/hosts` writes prevents symlink-following attacks.
- **Strongly-typed IPC structs** (e.g. `LastApplied` in `apply.rs`) — never accept `serde_json::Value` from the frontend.
- **Capability manifest** at `src-tauri/capabilities/default.json` is the only place to widen Tauri permissions.
- **CSP** is configured in `src-tauri/tauri.conf.json::app.security.csp` — preserve the `connect-src ipc: http://ipc.localhost` directive.
- **DNS server bound to `127.0.0.1` only** — never expose to the network.

## Performance review history

`perf_review_report.md` (branch `fix/perf-issues-audit-2026-06`, merged) closed 15 issues (#23-#37). A follow-up review was published as GitHub issue **#90** (2026-07). Before opening a new perf audit, check #90 — its Critical/High findings (DNS hot-path clones, apply path full-file scans, sidebar atom subscriptions) are still open.

## Release flow

`.github/workflows/release.yml` triggers on `v*` tags. The matrix builds both `aarch64-apple-darwin` and `x86_64-apple-darwin`. `tauri-action@v0.6` produces a draft release (`prerelease: false`). To ship a release: tag, push, then publish the draft from the GitHub UI.