//! Manifest management

use chrono::{DateTime, Utc};
use mhost_core::{OriginalDns, StorageError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

/// 存储格式清单，用于数据版本管理。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    /// 数据格式版本，v2 起支持 DNS 模式
    pub version: u32,
    /// 应用版本号
    pub app_version: String,
    /// 最后更新时间戳（UTC）
    pub updated_at: DateTime<Utc>,
    /// DNS 模式全局开关状态（v2 新增，旧版本兼容为 None）
    #[serde(default)]
    pub dns_enabled: Option<bool>,
    /// 启用 DNS 模式前的系统 DNS 配置快照（v2.1 新增，**fix：v2.2 从
    /// `Vec<String>` 改为 `OriginalDns` 语义枚举**，用于崩溃/强杀后恢复）。
    ///
    /// - `Some(Manual(servers))`：用户在 System Settings 里手动配的；
    ///   disable 时回写这些 servers。
    /// - `Some(DhcpEmpty)`：用户没手动配；disable 时回写 `Empty`（DHCP
    ///   default），避免跨网络切换时泄漏上次抓到的 DHCP 推的 IP。
    /// - `None`：全新安装或 DNS mode 尚未启用。
    ///
    /// 反序列化兼容两种磁盘格式（详见 `OriginalDns::deserialize` 的迁移规则）：
    ///   - 新格式 `{"kind":"manual","servers":[...]}` 或 `{"kind":"dhcp_empty"}`
    ///   - 旧格式裸 `Vec<String>`（v2.0 / v2.1）
    #[serde(default)]
    pub original_dns: Option<OriginalDns>,
}

impl Manifest {
    /// 创建 v2 默认清单（version = 2）。
    pub fn new(app_version: impl Into<String>) -> Self {
        Self {
            version: 2,
            app_version: app_version.into(),
            updated_at: Utc::now(),
            dns_enabled: Some(false),
            original_dns: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Migration trait
// ---------------------------------------------------------------------------

// TODO: Migration trait 已预留，待与 Storage 集成以实现自动数据迁移。
/// 数据迁移接口，用于不同存储版本之间的升级/降级。
#[allow(clippy::should_implement_trait, clippy::wrong_self_convention)]
pub trait Migration {
    /// 源版本号
    fn from_version(&self) -> u32;
    /// 目标版本号
    fn to_version(&self) -> u32;
    /// 执行迁移，将源版本数据转换为目标版本数据
    fn migrate(&self, data: Value) -> Result<Value, StorageError>;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_default_version() {
        let manifest = Manifest::new("0.1.0");
        assert_eq!(manifest.version, 2);
        assert_eq!(manifest.app_version, "0.1.0");
        assert_eq!(manifest.dns_enabled, Some(false));
    }

    #[test]
    fn test_manifest_serialization_roundtrip() {
        let cases = vec![
            ("v2_0_1_0", Manifest::new("0.1.0")),
            (
                "v2_explicit_dns_false",
                Manifest {
                    version: 2,
                    app_version: "1.0.0".to_string(),
                    updated_at: "2024-01-01T00:00:00+00:00".parse().unwrap(),
                    dns_enabled: Some(false),
                    original_dns: None,
                },
            ),
            (
                "v2_dns_enabled",
                Manifest {
                    version: 2,
                    app_version: "1.1.0".to_string(),
                    updated_at: "2024-06-01T00:00:00+00:00".parse().unwrap(),
                    dns_enabled: Some(true),
                    original_dns: None,
                },
            ),
            (
                "v2_dns_with_original",
                Manifest {
                    version: 2,
                    app_version: "1.2.0".to_string(),
                    updated_at: "2024-07-01T00:00:00+00:00".parse().unwrap(),
                    dns_enabled: Some(true),
                    original_dns: Some(OriginalDns::Manual(vec![
                        "8.8.8.8".to_string(),
                        "1.1.1.1".to_string(),
                    ])),
                },
            ),
        ];

        for (name, manifest) in cases {
            let json = serde_json::to_string(&manifest).unwrap();
            let restored: Manifest = serde_json::from_str(&json).unwrap();
            assert_eq!(manifest, restored, "case: {}", name);
        }
    }

    #[test]
    fn test_manifest_json_format() {
        let manifest = Manifest::new("0.1.0");
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        assert!(json.contains("\"version\""));
        assert!(json.contains("\"app_version\""));
        assert!(json.contains("\"updated_at\""));
        assert!(json.contains("\"dns_enabled\""));
        assert!(json.contains("2"));
        assert!(json.contains("0.1.0"));
    }

    #[test]
    fn test_manifest_updated_at_is_set() {
        let manifest = Manifest::new("0.1.0");
        let rfc3339 = manifest.updated_at.to_rfc3339();
        // 验证是有效的 RFC 3339 时间戳
        assert!(rfc3339.contains('T'));
        assert!(rfc3339.contains('+') || rfc3339.contains('Z'));
    }

    #[test]
    fn test_manifest_v1_backward_compatibility() {
        // 模拟旧版本 v1 manifest JSON（不含 dns_enabled 字段）
        let v1_json = r#"{
            "version": 1,
            "app_version": "0.1.0",
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let manifest: Manifest = serde_json::from_str(v1_json).unwrap();
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.app_version, "0.1.0");
        assert_eq!(
            manifest.dns_enabled, None,
            "旧版本 manifest 反序列化时 dns_enabled 应为 None"
        );
        assert_eq!(
            manifest.original_dns, None,
            "旧版本 manifest 反序列化时 original_dns 应为 None"
        );
    }

    #[test]
    fn test_manifest_default_has_none_original_dns() {
        // 新建 manifest 应默认 original_dns = None
        let manifest = Manifest::new("1.0.0");
        assert_eq!(manifest.original_dns, None);
    }

    #[test]
    fn test_manifest_original_dns_backward_compat_pre_v2_1() {
        // v2.0 manifest 不包含 original_dns 字段，反序列化时默认为 None
        let v2_pre_2_1 = r#"{
            "version": 2,
            "app_version": "1.0.0",
            "updated_at": "2024-01-01T00:00:00Z",
            "dns_enabled": true
        }"#;

        let manifest: Manifest = serde_json::from_str(v2_pre_2_1).unwrap();
        assert_eq!(manifest.dns_enabled, Some(true));
        assert_eq!(
            manifest.original_dns, None,
            "v2.0 不含 original_dns 字段时应默认为 None"
        );
    }

    // -----------------------------------------------------------------------
    // OriginalDns 迁移测试（fix: disabling-after-network-switch）
    // -----------------------------------------------------------------------

    #[test]
    fn test_manifest_original_dns_legacy_vec_non_empty_migrates_to_manual() {
        // v2.0/v2.1 旧格式：裸 Vec<String> 且非空 → Manual(vec)
        let legacy_json = r#"{
            "version": 2,
            "app_version": "1.0.0",
            "updated_at": "2024-01-01T00:00:00Z",
            "dns_enabled": true,
            "original_dns": ["8.8.8.8", "1.1.1.1"]
        }"#;
        let manifest: Manifest = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(
            manifest.original_dns,
            Some(OriginalDns::Manual(vec![
                "8.8.8.8".to_string(),
                "1.1.1.1".to_string()
            ]))
        );
    }

    #[test]
    fn test_manifest_original_dns_legacy_vec_empty_migrates_to_dhcp_empty() {
        let legacy_json = r#"{
            "version": 2,
            "app_version": "1.0.0",
            "updated_at": "2024-01-01T00:00:00Z",
            "dns_enabled": true,
            "original_dns": []
        }"#;
        let manifest: Manifest = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(manifest.original_dns, Some(OriginalDns::DhcpEmpty));
    }

    #[test]
    fn test_manifest_original_dns_legacy_vec_placeholder_migrates_to_dhcp_empty() {
        // v2.0 的 ["Empty"] 残留 placeholder → DhcpEmpty
        let legacy_json = r#"{
            "version": 2,
            "app_version": "1.0.0",
            "updated_at": "2024-01-01T00:00:00Z",
            "dns_enabled": true,
            "original_dns": ["Empty"]
        }"#;
        let manifest: Manifest = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(manifest.original_dns, Some(OriginalDns::DhcpEmpty));
    }

    #[test]
    fn test_manifest_original_dns_new_tagged_manual_roundtrip() {
        let manifest = Manifest {
            version: 2,
            app_version: "2.2.0".to_string(),
            updated_at: "2024-07-01T00:00:00+00:00".parse().unwrap(),
            dns_enabled: Some(true),
            original_dns: Some(OriginalDns::Manual(vec!["9.9.9.9".to_string()])),
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let restored: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(
            restored.original_dns,
            Some(OriginalDns::Manual(vec!["9.9.9.9".to_string()]))
        );
        // 验证是新 tagged 格式
        assert!(json.contains("\"kind\":\"manual\""));
        assert!(json.contains("\"servers\":[\"9.9.9.9\"]"));
    }

    #[test]
    fn test_manifest_original_dns_new_tagged_dhcp_empty_roundtrip() {
        let manifest = Manifest {
            version: 2,
            app_version: "2.2.0".to_string(),
            updated_at: "2024-07-01T00:00:00+00:00".parse().unwrap(),
            dns_enabled: Some(true),
            original_dns: Some(OriginalDns::DhcpEmpty),
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let restored: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.original_dns, Some(OriginalDns::DhcpEmpty));
        // 验证 DhcpEmpty 序列化不会泄漏空 servers 数组
        assert!(!json.contains("\"servers\""));
        assert!(json.contains("\"kind\":\"dhcp_empty\""));
    }
}
