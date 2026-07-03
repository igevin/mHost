use serde::{Deserialize, Serialize};

/// DNS 服务配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DnsConfig {
    /// 监听端口（默认 53）。
    pub port: u16,
    /// 上游 DNS 服务器地址列表。
    pub upstream: Vec<String>,
    /// 缓存大小（默认 1000）。
    pub cache_size: usize,
    /// 上游查询超时毫秒数（默认 3000）。
    pub timeout_ms: u64,
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            port: 53,
            upstream: vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()],
            cache_size: 1000,
            timeout_ms: 3000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dns_config_default() {
        let config = DnsConfig::default();
        assert_eq!(config.port, 53);
        assert_eq!(config.upstream, vec!["8.8.8.8", "1.1.1.1"]);
        assert_eq!(config.cache_size, 1000);
    }

    #[test]
    fn test_dns_config_serde_roundtrip() {
        let cases = vec![
            (
                "default",
                DnsConfig::default(),
            ),
            (
                "custom",
                DnsConfig {
                    port: 1053,
                    upstream: vec!["9.9.9.9".to_string()],
                    cache_size: 500,
                    timeout_ms: 3000,
                },
            ),
            (
                "empty_upstream",
                DnsConfig {
                    port: 53,
                    upstream: vec![],
                    cache_size: 0,
                    timeout_ms: 3000,
                },
            ),
        ];

        for (name, config) in cases {
            let json = serde_json::to_string(&config).unwrap();
            let restored: DnsConfig = serde_json::from_str(&json).unwrap();
            assert_eq!(config, restored, "case: {}", name);
        }
    }

    #[test]
    fn test_dns_config_json_format() {
        let config = DnsConfig::default();
        let json = serde_json::to_string_pretty(&config).unwrap();
        assert!(json.contains("\"port\""));
        assert!(json.contains("\"upstream\""));
        assert!(json.contains("\"cache_size\""));
        assert!(json.contains("8.8.8.8"));
    }
}
