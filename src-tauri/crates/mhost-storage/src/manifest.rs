//! Manifest management

use chrono::{DateTime, Utc};
use mhost_core::StorageError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

/// 存储格式清单，用于数据版本管理。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    /// 数据格式版本，阶段 0 = 1
    pub version: u32,
    /// 应用版本号
    pub app_version: String,
    /// 最后更新时间戳（UTC）
    pub updated_at: DateTime<Utc>,
}

impl Manifest {
    /// 创建阶段 0 的默认清单（version = 1）。
    pub fn new(app_version: impl Into<String>) -> Self {
        Self {
            version: 1,
            app_version: app_version.into(),
            updated_at: Utc::now(),
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
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.app_version, "0.1.0");
    }

    #[test]
    fn test_manifest_serialization_roundtrip() {
        let cases = vec![
            ("v1_0_1_0", Manifest::new("0.1.0")),
            (
                "v1_1_0_0",
                Manifest {
                    version: 1,
                    app_version: "1.0.0".to_string(),
                    updated_at: "2024-01-01T00:00:00+00:00".parse().unwrap(),
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
        assert!(json.contains("1"));
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
}
