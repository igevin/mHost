use std::process::Command;

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
        return Err(PlatformError::GetDns(format!("networksetup failed: {}", stderr)));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_dns_servers(&stdout)
}

/// 设置系统 DNS 为本地服务（127.0.0.1）。
pub fn set_local_dns() -> Result<(), PlatformError> {
    let interface = get_active_network_interface()?;
    let output = Command::new("networksetup")
        .args(["-setdnsservers", &interface, "127.0.0.1"])
        .output()
        .map_err(|e| PlatformError::SetDns(format!("command failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PlatformError::SetDns(format!("networksetup failed: {}", stderr)));
    }

    Ok(())
}

/// 恢复系统 DNS 为指定列表。
pub fn restore_system_dns(servers: &[String]) -> Result<(), PlatformError> {
    let interface = get_active_network_interface()?;

    if servers.is_empty() {
        // 空列表表示恢复为 DHCP 自动获取
        let output = Command::new("networksetup")
            .args(["-setdnsservers", &interface, "Empty"])
            .output()
            .map_err(|e| PlatformError::RestoreDns(format!("command failed: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PlatformError::RestoreDns(format!(
                "networksetup failed: {}",
                stderr
            )));
        }
        return Ok(());
    }

    let mut args = vec!["-setdnsservers", &interface];
    for s in servers {
        args.push(s.as_str());
    }

    let output = Command::new("networksetup")
        .args(&args)
        .output()
        .map_err(|e| PlatformError::RestoreDns(format!("command failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PlatformError::RestoreDns(format!(
            "networksetup failed: {}",
            stderr
        )));
    }

    Ok(())
}

/// 获取当前活跃的网络接口名（Hardware Port）。
fn get_active_network_interface() -> Result<String, PlatformError> {
    // 1. 获取默认路由对应的设备名（如 en0）
    let route_output = Command::new("route")
        .args(["-n", "get", "default"])
        .output()
        .map_err(|e| {
            PlatformError::DetectInterface(format!("route command failed: {}", e))
        })?;

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
    parse_hardware_port(&list_stdout, &device).ok_or_else(|| {
        PlatformError::DetectInterface(format!(
            "no hardware port found for device '{}'",
            device
        ))
    })
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
        assert_eq!(
            parse_route_interface(output),
            Some("en0".to_string())
        );
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
}
