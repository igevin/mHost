use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::IpAddr;
use std::path::PathBuf;
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
// ProfileMode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProfileMode {
    #[default]
    Hosts,
    Dns,
}

// ---------------------------------------------------------------------------
// Profile
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub enabled: bool,
    pub protected: bool,
    pub tags: Vec<String>,
    pub rules: Vec<HostRule>,
    #[serde(default)]
    pub mode: ProfileMode,
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
            mode: ProfileMode::Hosts,
            created_at: now,
            updated_at: now,
        }
    }
}

// ---------------------------------------------------------------------------
// DuplicateRule / DuplicateKind
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateRule {
    pub domain: String,
    pub lines: Vec<usize>,
    pub kind: DuplicateKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DuplicateKind {
    #[serde(rename = "same_ip")]
    SameIp,
    #[serde(rename = "different_ip")]
    DifferentIp,
}

// ---------------------------------------------------------------------------
// HostRule
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostRule {
    pub id: RuleId,
    /// For comment-only lines, this is `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<IpAddr>,
    pub domains: Vec<String>,
    pub enabled: bool,
    /// For comment-only lines, stores the full comment text (e.g. "# this is a comment").
    /// For inline comments on rule lines, stores the comment after `#`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub source: RuleSource,
    /// 1-based line number in the original hosts text (set by `parse_with_lines`).
    #[serde(skip)]
    pub line_number: Option<usize>,
}

impl HostRule {
    pub fn new(ip: IpAddr, domains: Vec<String>) -> Self {
        Self {
            id: RuleId(Uuid::new_v4()),
            ip: Some(ip),
            domains,
            enabled: true,
            comment: None,
            source: RuleSource::Manual,
            line_number: None,
        }
    }

    /// Create a standalone comment-only rule.
    pub fn comment_only(text: impl Into<String>) -> Self {
        Self {
            id: RuleId(Uuid::new_v4()),
            ip: None,
            domains: Vec::new(),
            enabled: false,
            comment: Some(text.into()),
            source: RuleSource::Manual,
            line_number: None,
        }
    }

    /// Returns `true` if this rule represents a standalone comment line.
    pub fn is_comment_only(&self) -> bool {
        self.ip.is_none()
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
// ApplyOutcome / ApplyMode  (issue #127)
// ---------------------------------------------------------------------------

/// Structured result of an apply (or a previewed apply).
///
/// Returned by both `preview_apply_outcome` (read-only) and `enable_and_apply`
/// (after writing). The shape is identical so the frontend can hand the same
/// value to `decide_apply_mode` regardless of whether the write happened.
///
/// `plan` is the hosts-mode `ApplyPlan` that was applied (or previewed).
/// For DNS-mode profiles, `plan` is an empty `ApplyPlan` because DNS apply
/// does not touch `/etc/hosts`.
///
/// `disabled_profile_ids`:
/// - Preview path: profile IDs that **will be** auto-disabled if the toggle
///   is applied (because the target was being enabled with another hosts
///   profile already enabled).
/// - Apply path: profile IDs that **were** auto-disabled by the write.
///
/// `snapshot_id` / `backup_path` are always `None` for preview and for
/// DNS-mode apply — neither runs `auto_snapshot_logic` nor writes a
/// `.bak` file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplyOutcome {
    pub plan: ApplyPlan,
    pub added_count: usize,
    pub removed_count: usize,
    pub unchanged_count: usize,
    pub disabled_profile_ids: Vec<String>,
    pub has_conflicts: bool,
    pub snapshot_id: Option<String>,
    pub backup_path: Option<String>,
}

/// Whether a toggle should apply directly or open the preview dialog.
///
/// `QuickApply` — the change is safe enough to apply without preview.
/// `RequirePreview` — the user must see the diff and confirm explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyMode {
    QuickApply,
    RequirePreview,
}

impl ApplyOutcome {
    /// Construct an empty outcome (DNS-mode apply or uninitialized state).
    pub fn empty() -> Self {
        Self {
            plan: ApplyPlan {
                rules: vec![],
                conflicts: vec![],
                diff: HostsDiff {
                    added: vec![],
                    removed: vec![],
                    unchanged: vec![],
                },
                backup_required: false,
            },
            added_count: 0,
            removed_count: 0,
            unchanged_count: 0,
            disabled_profile_ids: vec![],
            has_conflicts: false,
            snapshot_id: None,
            backup_path: None,
        }
    }

    /// Build an outcome from a (preview or applied) plan plus apply-side
    /// context. Centralizes the "compute counts from diff" rule so the
    /// `decide_apply_mode` policy can rely on stable invariants.
    pub fn from_parts(
        plan: ApplyPlan,
        disabled_profile_ids: Vec<String>,
        snapshot_id: Option<String>,
        backup_path: Option<PathBuf>,
    ) -> Self {
        let has_conflicts = !plan.conflicts.is_empty();
        Self {
            added_count: plan.diff.added.len(),
            removed_count: plan.diff.removed.len(),
            unchanged_count: plan.diff.unchanged.len(),
            plan,
            disabled_profile_ids,
            has_conflicts,
            snapshot_id,
            backup_path: backup_path.map(|p| p.to_string_lossy().into_owned()),
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub profiles: Vec<Profile>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotMeta {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub profile_count: usize,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// OriginalDns
// ---------------------------------------------------------------------------

/// Snapshot of the user's original DNS configuration captured at enable time.
///
/// Distinguishes *user-managed* DNS from *DHCP/empty* so that disable restores
/// exactly what the user had — not values that DHCP happens to have pushed at
/// the moment of capture. Concretely:
///
/// - `Manual(servers)` — user had DNS set in *System Settings* (`networksetup`
///   returned a non-empty list). Restore writes those servers back.
/// - `DhcpEmpty`       — user had nothing manually configured (Tier 1 empty);
///   the system was relying on DHCP defaults or had no DNS at all. Restore
///   writes `Empty` (DHCP default) to avoid leaking a captured DHCP-pushed
///   value across a network switch.
///
/// Tier 3 (`[8.8.8.8, 1.1.1.1]`) — the last-resort public-DNS fallback used
/// exclusively as the `DnsServer` upstream resolver — is *never* represented
/// here. The separation between this type and `get_upstream_resolvers()`
/// makes that enforceable by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginalDns {
    Manual(Vec<String>),
    DhcpEmpty,
}

impl OriginalDns {
    /// Args to pass to `networksetup -setdnsservers <iface> ...` on restore.
    /// DhcpEmpty → `["Empty"]` (= DHCP default).
    pub fn restore_argv(&self) -> Vec<String> {
        match self {
            Self::Manual(s) => s.clone(),
            Self::DhcpEmpty => vec!["Empty".to_string()],
        }
    }

    /// Was this captured state a user-managed DNS config (vs DHCP/empty)?
    pub fn is_manual(&self) -> bool {
        matches!(self, Self::Manual(_))
    }
}

/// Wire format:
///   `Manual(s)`  → `{"kind":"manual","servers":[...]}`
///   `DhcpEmpty`  → `{"kind":"dhcp_empty"}`
///
/// Hand-written (not `#[derive(Serialize)]`) so DhcpEmpty has no `servers` field.
impl Serialize for OriginalDns {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        match self {
            Self::Manual(servers) => {
                let mut s = ser.serialize_struct("OriginalDns", 2)?;
                s.serialize_field("kind", "manual")?;
                s.serialize_field("servers", servers)?;
                s.end()
            }
            Self::DhcpEmpty => {
                let mut s = ser.serialize_struct("OriginalDns", 1)?;
                s.serialize_field("kind", "dhcp_empty")?;
                s.end()
            }
        }
    }
}

/// Accepts BOTH the new tagged form AND the legacy bare `Vec<String>`
/// (used in pre-v2.1 manifests). Migration rules:
///   - `{"kind":"manual","servers":[...]}` → Manual
///   - `{"kind":"dhcp_empty"}`               → DhcpEmpty
///   - `[]`                                  → DhcpEmpty
///   - `["Empty"]`                           → DhcpEmpty (v2.0 placeholder)
///   - `["1.1.1.1", ...]`                    → Manual(vec)
impl<'de> Deserialize<'de> for OriginalDns {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Tagged(Tagged),
            Legacy(Vec<String>),
        }
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum Tagged {
            Manual { servers: Vec<String> },
            DhcpEmpty,
        }
        match Repr::deserialize(de)? {
            Repr::Tagged(Tagged::Manual { servers }) => Ok(OriginalDns::Manual(servers)),
            Repr::Tagged(Tagged::DhcpEmpty) => Ok(OriginalDns::DhcpEmpty),
            Repr::Legacy(vec) => {
                if vec.is_empty() || vec.iter().any(|s| s == "Empty") {
                    Ok(OriginalDns::DhcpEmpty)
                } else {
                    Ok(OriginalDns::Manual(vec))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// DnsStatus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DnsStatus {
    pub running: bool,
    pub port: u16,
    pub upstream: Vec<String>,
    /// Enable 时捕获的系统 DNS 快照（disable 时按语义还原）。
    /// 详见 `OriginalDns`；`Manual(servers)` 回写 server 列表，
    /// `DhcpEmpty` 回写 `Empty`（DHCP 默认）。
    pub original_dns: OriginalDns,
    pub rule_count: usize,
    pub cache_capacity: usize,
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
        assert_eq!(p.mode, ProfileMode::Hosts);
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
            assert_eq!(profile.mode, restored.mode, "case: {}", name);
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

    #[test]
    fn test_profile_mode_backward_compatibility() {
        // 模拟旧版本序列化的 Profile JSON（不含 mode 字段）
        let old_json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "name": "legacy",
            "enabled": false,
            "protected": false,
            "tags": [],
            "rules": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let profile: Profile = serde_json::from_str(old_json).unwrap();
        assert_eq!(
            profile.mode,
            ProfileMode::Hosts,
            "旧数据反序列化时 mode 应默认为 Hosts"
        );
    }

    #[test]
    fn test_profile_mode_serde_roundtrip() {
        let cases = vec![("hosts", ProfileMode::Hosts), ("dns", ProfileMode::Dns)];

        for (name, mode) in cases {
            let json = serde_json::to_string(&mode).unwrap();
            let restored: ProfileMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, restored, "case: {}", name);
        }
    }

    #[test]
    fn test_profile_mode_json_format() {
        assert_eq!(
            serde_json::to_string(&ProfileMode::Hosts).unwrap(),
            "\"hosts\""
        );
        assert_eq!(serde_json::to_string(&ProfileMode::Dns).unwrap(), "\"dns\"");
    }

    #[test]
    fn test_profile_dns_mode_serialization() {
        let mut profile = Profile::new("dns_profile");
        profile.mode = ProfileMode::Dns;

        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("\"mode\":\"dns\""));

        let restored: Profile = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.mode, ProfileMode::Dns);
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

    // -----------------------------------------------------------------------
    // OriginalDns tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_original_dns_manual_serde_roundtrip() {
        let orig = OriginalDns::Manual(vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()]);
        let json = serde_json::to_string(&orig).unwrap();
        assert!(json.contains("\"kind\":\"manual\""));
        assert!(json.contains("\"servers\":[\"8.8.8.8\",\"1.1.1.1\"]"));
        let restored: OriginalDns = serde_json::from_str(&json).unwrap();
        assert_eq!(orig, restored);
    }

    #[test]
    fn test_original_dns_dhcp_empty_serde_roundtrip() {
        let orig = OriginalDns::DhcpEmpty;
        let json = serde_json::to_string(&orig).unwrap();
        assert_eq!(json, r#"{"kind":"dhcp_empty"}"#);
        let restored: OriginalDns = serde_json::from_str(&json).unwrap();
        assert_eq!(orig, restored);
    }

    #[test]
    fn test_original_dns_dhcp_empty_has_no_servers_field() {
        // 反向断言：wire 格式不能泄漏空的 servers 数组。
        let json = serde_json::to_string(&OriginalDns::DhcpEmpty).unwrap();
        assert!(
            !json.contains("servers"),
            "DhcpEmpty 序列化不应出现 servers 字段，得到: {json}"
        );
    }

    #[test]
    fn test_original_dns_restore_argv() {
        assert_eq!(
            OriginalDns::Manual(vec!["8.8.8.8".to_string()]).restore_argv(),
            vec!["8.8.8.8".to_string()]
        );
        assert_eq!(
            OriginalDns::DhcpEmpty.restore_argv(),
            vec!["Empty".to_string()]
        );
    }

    #[test]
    fn test_original_dns_is_manual() {
        assert!(OriginalDns::Manual(vec!["1.1.1.1".to_string()]).is_manual());
        assert!(!OriginalDns::DhcpEmpty.is_manual());
    }

    #[test]
    fn test_original_dns_deserialize_legacy_vec_non_empty() {
        // 旧 manifest 形态：裸 Vec<String>。
        let legacy_json = r#"["192.168.1.1", "8.8.8.8"]"#;
        let restored: OriginalDns = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(
            restored,
            OriginalDns::Manual(vec!["192.168.1.1".to_string(), "8.8.8.8".to_string()])
        );
    }

    #[test]
    fn test_original_dns_deserialize_legacy_vec_empty() {
        let legacy_json = r#"[]"#;
        let restored: OriginalDns = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(restored, OriginalDns::DhcpEmpty);
    }

    #[test]
    fn test_original_dns_deserialize_legacy_vec_placeholder() {
        // v2.0 的 `["Empty"]` 占位符 → DhcpEmpty。
        let legacy_json = r#"["Empty"]"#;
        let restored: OriginalDns = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(restored, OriginalDns::DhcpEmpty);
    }

    #[test]
    fn test_original_dns_deserialize_tagged_manual() {
        let json = r#"{"kind":"manual","servers":["1.1.1.1"]}"#;
        let restored: OriginalDns = serde_json::from_str(json).unwrap();
        assert_eq!(restored, OriginalDns::Manual(vec!["1.1.1.1".to_string()]));
    }

    #[test]
    fn test_original_dns_deserialize_tagged_dhcp_empty() {
        let json = r#"{"kind":"dhcp_empty"}"#;
        let restored: OriginalDns = serde_json::from_str(json).unwrap();
        assert_eq!(restored, OriginalDns::DhcpEmpty);
    }

    // ---- ApplyOutcome (issue #127) ----

    fn fixture_plan_with_diff_and_conflict() -> ApplyPlan {
        let conflict_rule_a = ResolvedRule {
            ip: "127.0.0.1".parse().unwrap(),
            domain: "conflict.example".into(),
            source_profile_id: ProfileId(Uuid::new_v4()),
            source_profile_name: "profile-a".into(),
        };
        let conflict_rule_b = ResolvedRule {
            ip: "10.0.0.1".parse().unwrap(),
            domain: "conflict.example".into(),
            source_profile_id: ProfileId(Uuid::new_v4()),
            source_profile_name: "profile-b".into(),
        };
        ApplyPlan {
            rules: vec![],
            conflicts: vec![RuleConflict {
                domain: "conflict.example".into(),
                rules: vec![conflict_rule_a, conflict_rule_b],
            }],
            diff: HostsDiff {
                added: vec![
                    "1.1.1.1 a.example".into(),
                    "2.2.2.2 b.example".into(),
                    "3.3.3.3 c.example".into(),
                ],
                removed: vec!["4.4.4.4 old.example".into(), "5.5.5.5 older.example".into()],
                unchanged: (0..5)
                    .map(|i| format!("10.0.0.{} stable.example", i))
                    .collect(),
            },
            backup_required: true,
        }
    }

    #[test]
    fn apply_outcome_empty_has_zero_counts_and_no_conflicts() {
        let outcome = ApplyOutcome::empty();
        assert_eq!(outcome.added_count, 0);
        assert_eq!(outcome.removed_count, 0);
        assert_eq!(outcome.unchanged_count, 0);
        assert!(outcome.disabled_profile_ids.is_empty());
        assert!(!outcome.has_conflicts);
        assert!(outcome.snapshot_id.is_none());
        assert!(outcome.backup_path.is_none());
        assert!(outcome.plan.rules.is_empty());
        assert!(outcome.plan.conflicts.is_empty());
        assert!(!outcome.plan.backup_required);
    }

    #[test]
    fn apply_outcome_from_parts_computes_counts_and_has_conflicts() {
        let plan = fixture_plan_with_diff_and_conflict();
        let outcome = ApplyOutcome::from_parts(plan, vec![], Some("snap-1".into()), None);
        assert_eq!(outcome.added_count, 3);
        assert_eq!(outcome.removed_count, 2);
        assert_eq!(outcome.unchanged_count, 5);
        assert!(
            outcome.has_conflicts,
            "should derive true from non-empty conflicts"
        );
        assert_eq!(outcome.snapshot_id.as_deref(), Some("snap-1"));
        assert!(outcome.backup_path.is_none());
    }

    #[test]
    fn apply_outcome_from_parts_handles_backup_path() {
        let plan = ApplyPlan {
            rules: vec![],
            conflicts: vec![],
            diff: HostsDiff {
                added: vec!["1.1.1.1 x".into()],
                removed: vec![],
                unchanged: vec![],
            },
            backup_required: true,
        };
        let path = PathBuf::from("/tmp/mhost-test/backups/hosts-20260101_120000.bak");
        let outcome = ApplyOutcome::from_parts(plan, vec![], None, Some(path.clone()));
        assert_eq!(
            outcome.backup_path.as_deref(),
            Some(path.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn apply_outcome_serde_roundtrip() {
        let plan = fixture_plan_with_diff_and_conflict();
        let original = ApplyOutcome::from_parts(
            plan,
            vec!["disabled-1".into(), "disabled-2".into()],
            Some("snap-xyz".into()),
            Some(PathBuf::from("/var/backups/hosts.bak")),
        );
        let json = serde_json::to_string(&original).unwrap();
        let restored: ApplyOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn apply_mode_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&ApplyMode::QuickApply).unwrap(),
            "\"quick_apply\""
        );
        assert_eq!(
            serde_json::to_string(&ApplyMode::RequirePreview).unwrap(),
            "\"require_preview\""
        );
        let q: ApplyMode = serde_json::from_str("\"quick_apply\"").unwrap();
        let r: ApplyMode = serde_json::from_str("\"require_preview\"").unwrap();
        assert_eq!(q, ApplyMode::QuickApply);
        assert_eq!(r, ApplyMode::RequirePreview);
    }
}
