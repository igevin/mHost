use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::IpAddr;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// ID types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfileId(pub Uuid);

impl fmt::Display for ProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for ProfileId {
    type Err = crate::MhostError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uuid = Uuid::parse_str(s)
            .map_err(|e| crate::MhostError::InvalidInput(format!("invalid profile id: {}", e)))?;
        Ok(ProfileId(uuid))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuleId(pub Uuid);

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceId(pub Uuid);

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

// ---------------------------------------------------------------------------
// Profile
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub protected: bool,
    pub tags: Vec<String>,
    pub rules: Vec<HostRule>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Profile {
    pub fn new(name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: ProfileId(Uuid::new_v4()),
            name: name.into(),
            description: None,
            enabled: false,
            protected: false,
            tags: Vec::new(),
            rules: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}

// ---------------------------------------------------------------------------
// HostRule
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostRule {
    pub id: RuleId,
    pub ip: IpAddr,
    pub domains: Vec<String>,
    pub enabled: bool,
    pub comment: Option<String>,
    pub source: RuleSource,
}

impl HostRule {
    pub fn new(ip: IpAddr, domains: Vec<String>) -> Self {
        Self {
            id: RuleId(Uuid::new_v4()),
            ip,
            domains,
            enabled: true,
            comment: None,
            source: RuleSource::Manual,
        }
    }
}

// ---------------------------------------------------------------------------
// ExternalSource
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalSource {
    pub source_id: SourceId,
    pub source_name: String,
}

// ---------------------------------------------------------------------------
// RuleSource
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum RuleSource {
    Manual,
    Remote(ExternalSource),
    AdBlock(ExternalSource),
}

// ---------------------------------------------------------------------------
// ExportFormat
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Hosts,
    Json,
}

// ---------------------------------------------------------------------------
// ApplyPlan
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplyPlan {
    pub rules: Vec<ResolvedRule>,
    pub conflicts: Vec<RuleConflict>,
    pub diff: HostsDiff,
    pub backup_required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedRule {
    pub ip: IpAddr,
    pub domain: String,
    pub source_profile_id: ProfileId,
    pub source_profile_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleConflict {
    pub domain: String,
    pub rules: Vec<ResolvedRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostsDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: Vec<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Profile tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_profile_default_values() {
        let p = Profile::new("dev");
        assert!(!p.enabled);
        assert!(!p.protected);
        assert!(p.tags.is_empty());
        assert!(p.rules.is_empty());
        assert!(p.description.is_none());
    }

    #[test]
    fn test_profile_serialization() {
        let mut profile_with_rules = Profile::new("with_rules");
        profile_with_rules.description = Some("desc".to_string());
        profile_with_rules.enabled = true;
        profile_with_rules.protected = true;
        profile_with_rules.tags = vec!["tag1".to_string(), "tag2".to_string()];
        profile_with_rules.rules.push(HostRule::new(
            "127.0.0.1".parse().unwrap(),
            vec!["a.com".to_string()],
        ));

        let cases = vec![
            ("minimal", Profile::new("test")),
            ("with_rules", profile_with_rules),
        ];

        for (name, profile) in cases {
            let json = serde_json::to_string(&profile).unwrap();
            let restored: Profile = serde_json::from_str(&json).unwrap();
            assert_eq!(profile.id, restored.id, "case: {}", name);
            assert_eq!(profile.name, restored.name, "case: {}", name);
            assert_eq!(profile.description, restored.description, "case: {}", name);
            assert_eq!(profile.enabled, restored.enabled, "case: {}", name);
            assert_eq!(profile.protected, restored.protected, "case: {}", name);
            assert_eq!(profile.tags, restored.tags, "case: {}", name);
            assert_eq!(profile.rules, restored.rules, "case: {}", name);
            assert_eq!(profile.created_at, restored.created_at, "case: {}", name);
            assert_eq!(profile.updated_at, restored.updated_at, "case: {}", name);
        }
    }

    #[test]
    fn test_profile_json_format() {
        let p = Profile::new("dev");
        let json = serde_json::to_string_pretty(&p).unwrap();
        // Verify that the JSON contains expected keys
        assert!(json.contains("\"id\""));
        assert!(json.contains("\"name\""));
        assert!(json.contains("\"enabled\""));
    }

    // -----------------------------------------------------------------------
    // HostRule tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_host_rule_new_defaults() {
        let rule = HostRule::new(
            "192.168.1.1".parse().unwrap(),
            vec!["example.com".to_string()],
        );
        assert!(rule.enabled);
        assert!(rule.comment.is_none());
        assert_eq!(rule.source, RuleSource::Manual);
        assert_eq!(rule.domains.len(), 1);
    }

    #[test]
    fn test_host_rule_multi_domain() {
        let rule = HostRule::new(
            "127.0.0.1".parse().unwrap(),
            vec!["a.com".to_string(), "b.com".to_string()],
        );
        assert_eq!(rule.domains.len(), 2);
        assert_eq!(rule.domains[0], "a.com");
        assert_eq!(rule.domains[1], "b.com");
    }

    #[test]
    fn test_host_rule_serialization_roundtrip() {
        let rule = HostRule::new(
            "::1".parse().unwrap(),
            vec!["localhost".to_string(), "local".to_string()],
        );
        let json = serde_json::to_string(&rule).unwrap();
        let restored: HostRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule.id, restored.id);
        assert_eq!(rule.ip, restored.ip);
        assert_eq!(rule.domains, restored.domains);
        assert_eq!(rule.enabled, restored.enabled);
        assert_eq!(rule.comment, restored.comment);
        assert_eq!(rule.source, restored.source);
    }

    // -----------------------------------------------------------------------
    // RuleSource tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_rule_source_manual_serde() {
        let source = RuleSource::Manual;
        let json = serde_json::to_string(&source).unwrap();
        assert_eq!(json, r#"{"type":"Manual"}"#);
        let restored: RuleSource = serde_json::from_str(&json).unwrap();
        assert_eq!(source, restored);
    }

    #[test]
    fn test_rule_source_remote_serde() {
        let source = RuleSource::Remote(ExternalSource {
            source_id: SourceId(Uuid::new_v4()),
            source_name: "My Remote".to_string(),
        });
        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("\"type\":\"Remote\""));
        assert!(json.contains("\"source_name\":\"My Remote\""));
        let restored: RuleSource = serde_json::from_str(&json).unwrap();
        assert_eq!(source, restored);
    }

    #[test]
    fn test_rule_source_adblock_serde() {
        let source = RuleSource::AdBlock(ExternalSource {
            source_id: SourceId(Uuid::new_v4()),
            source_name: "AdGuard".to_string(),
        });
        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("\"type\":\"AdBlock\""));
        assert!(json.contains("\"source_name\":\"AdGuard\""));
        let restored: RuleSource = serde_json::from_str(&json).unwrap();
        assert_eq!(source, restored);
    }

    // -----------------------------------------------------------------------
    // ID type tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_id_display() {
        let uuid = Uuid::new_v4();
        let pid = ProfileId(uuid);
        let rid = RuleId(uuid);
        let sid = SourceId(uuid);

        assert_eq!(pid.to_string(), uuid.to_string());
        assert_eq!(rid.to_string(), uuid.to_string());
        assert_eq!(sid.to_string(), uuid.to_string());
    }

    #[test]
    fn test_id_serde_roundtrip() {
        let uuid = Uuid::new_v4();
        let pid = ProfileId(uuid);
        let json = serde_json::to_string(&pid).unwrap();
        let restored: ProfileId = serde_json::from_str(&json).unwrap();
        assert_eq!(pid, restored);
    }

    // -----------------------------------------------------------------------
    // ApplyPlan / ResolvedRule / RuleConflict / HostsDiff tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_apply_plan_serialization_roundtrip() {
        let plan = ApplyPlan {
            rules: vec![ResolvedRule {
                ip: "127.0.0.1".parse().unwrap(),
                domain: "example.com".to_string(),
                source_profile_id: ProfileId(Uuid::new_v4()),
                source_profile_name: "dev".to_string(),
            }],
            conflicts: vec![RuleConflict {
                domain: "conflict.com".to_string(),
                rules: vec![
                    ResolvedRule {
                        ip: "127.0.0.1".parse().unwrap(),
                        domain: "conflict.com".to_string(),
                        source_profile_id: ProfileId(Uuid::new_v4()),
                        source_profile_name: "p1".to_string(),
                    },
                    ResolvedRule {
                        ip: "192.168.1.1".parse().unwrap(),
                        domain: "conflict.com".to_string(),
                        source_profile_id: ProfileId(Uuid::new_v4()),
                        source_profile_name: "p2".to_string(),
                    },
                ],
            }],
            diff: HostsDiff {
                added: vec!["127.0.0.1 example.com".to_string()],
                removed: vec!["127.0.0.1 old.com".to_string()],
                unchanged: vec!["::1 localhost".to_string()],
            },
            backup_required: true,
        };

        let json = serde_json::to_string(&plan).unwrap();
        let restored: ApplyPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(plan.rules.len(), restored.rules.len());
        assert_eq!(plan.conflicts.len(), restored.conflicts.len());
        assert_eq!(plan.diff.added, restored.diff.added);
        assert_eq!(plan.diff.removed, restored.diff.removed);
        assert_eq!(plan.diff.unchanged, restored.diff.unchanged);
        assert_eq!(plan.backup_required, restored.backup_required);
    }

    #[test]
    fn test_resolved_rule_serialization_roundtrip() {
        let rule = ResolvedRule {
            ip: "2001:db8::1".parse().unwrap(),
            domain: "ipv6.example.com".to_string(),
            source_profile_id: ProfileId(Uuid::new_v4()),
            source_profile_name: "test".to_string(),
        };
        let json = serde_json::to_string(&rule).unwrap();
        let restored: ResolvedRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule.ip, restored.ip);
        assert_eq!(rule.domain, restored.domain);
        assert_eq!(rule.source_profile_id, restored.source_profile_id);
        assert_eq!(rule.source_profile_name, restored.source_profile_name);
    }

    #[test]
    fn test_hosts_diff_empty() {
        let diff = HostsDiff {
            added: vec![],
            removed: vec![],
            unchanged: vec![],
        };
        let json = serde_json::to_string(&diff).unwrap();
        let restored: HostsDiff = serde_json::from_str(&json).unwrap();
        assert!(restored.added.is_empty());
        assert!(restored.removed.is_empty());
        assert!(restored.unchanged.is_empty());
    }
}
