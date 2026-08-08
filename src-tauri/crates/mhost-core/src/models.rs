use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::{IpAddr, SocketAddr};
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
// AdBlock (issue #130)
// ---------------------------------------------------------------------------
//
// 广告屏蔽状态与配置。**仅在 DNS 模式下生效** —— hosts 模式不再承担
// 广告屏蔽职责（早期尝试因 `/etc/hosts` 膨胀失败）。
//
// 数据布局（mhost-storage）：
//   {root}/adblock.json         # AdBlockState 整体序列化
//   {root}/adblock-cache/{id}.txt # 每源原始 hosts 内容

/// Per-source response when a domain hits an ad block rule.
///
/// `ZeroAddress` returns a 0.0.0.0 A record (clients typically retry-then-fail
/// after a timeout). `NxDomain` returns NXDOMAIN immediately (more aggressive
/// but some clients surface it as an error).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AdBlockResponse {
    #[default]
    ZeroAddress,
    NxDomain,
}

/// A remote ad block subscription source.
///
/// One source = one URL of hosts-format blocklist. Persisted as part of
/// `AdBlockState`. The cached fetched content lives at
/// `{root}/adblock-cache/{source_id}.txt` so that DNS mode can keep serving
/// blocklist hits even when the remote URL is temporarily unreachable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdBlockSource {
    pub source_id: SourceId,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub response: AdBlockResponse,
    /// RFC 3339 timestamp of the last successful fetch. `None` if never fetched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fetched_at: Option<DateTime<Utc>>,
    /// Last fetch error message (transport or non-2xx). Cleared on next success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Number of rules parsed from the last successful fetch.
    pub rule_count: usize,
    /// HTTP ETag from the last successful fetch (reserved for future
    /// conditional GETs — unused in v1 but persisted so we don't need a
    /// migration when conditional fetch lands).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
}

/// Persistent state for the DNS-mode ad block subsystem.
///
/// Stored as a single JSON document (`{root}/adblock.json`). All fields are
/// mutable from the IPC surface; the Rust side owns the on-disk write path
/// (`mhost-storage/src/adblock.rs`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdBlockState {
    pub enabled: bool,
    pub sources: Vec<AdBlockSource>,
    pub whitelist: Vec<String>,
    pub auto_refresh_enabled: bool,
    /// Hours between background refreshes. `0` disables background refresh
    /// regardless of `auto_refresh_enabled`. Clamped to a sane range at the
    /// IPC boundary.
    pub refresh_interval_hours: u32,
}

impl Default for AdBlockState {
    fn default() -> Self {
        Self {
            enabled: false,
            sources: Vec::new(),
            whitelist: Vec::new(),
            auto_refresh_enabled: true,
            refresh_interval_hours: 24,
        }
    }
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

/// **fix (issue #152 hardening)**：判定一个 DNS server 字符串是否指向
/// 本地 loopback / unspecified。和 `mhost_dns::platform::is_local_resolver`
/// 语义一致，但放在 `mhost-core` 是为了避免 `mhost-core ← mhost-dns`
/// 反向依赖（mhost-dns 已经依赖 mhost-core）。
///
/// 同时容忍 `host:port` / `[host]:port`（v6 bracketed）形式；解析不出来
/// 按「非本地」处理，留给上层校验兜底。
fn is_local_resolver(server: &str) -> bool {
    let host = server
        .parse::<SocketAddr>()
        .map(|sa| sa.ip())
        .or_else(|_| server.parse::<IpAddr>());
    matches!(host, Ok(ip) if ip.is_loopback() || ip.is_unspecified())
}

impl OriginalDns {
    /// Args to pass to `networksetup -setdnsservers <iface> ...` on restore.
    /// DhcpEmpty → `["Empty"]` (= DHCP default).
    ///
    /// **fix (issue #152 hardening)**：`Manual(s)` 在返回前过滤掉
    /// `127.0.0.1` / `::1` / unspecified，避免 legacy on-disk 污染（早期
    /// 版本 capture 没过滤）被再次写回系统 DNS。如果过滤后列表为空，
    /// 退回 DhcpEmpty 语义（返回 `["Empty"]`），永远不向 networksetup
    /// 传 `127.0.0.1`。
    pub fn restore_argv(&self) -> Vec<String> {
        match self {
            Self::Manual(s) => {
                let filtered: Vec<String> = s
                    .iter()
                    .filter(|x| !is_local_resolver(x))
                    .cloned()
                    .collect();
                if filtered.is_empty() {
                    vec!["Empty".to_string()]
                } else {
                    filtered
                }
            }
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
///   - `{"kind":"manual","servers":[...]}` → Manual (loopback-filtered)
///   - `{"kind":"dhcp_empty"}`               → DhcpEmpty
///   - `[]`                                  → DhcpEmpty
///   - `["Empty"]`                           → DhcpEmpty (v2.0 placeholder)
///   - `["1.1.1.1", ...]`                    → Manual(vec) (loopback-filtered)
///
/// **fix (issue #152 hardening)**：所有路径在构造 `Manual` 前过滤掉
/// `127.0.0.1` / `::1` / unspecified，防止 pre-fix manifest 的污染数据
/// 被 migrate 后再次写回系统 DNS。
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
            Repr::Tagged(Tagged::Manual { servers }) => {
                // **fix (issue #152 hardening)**：legacy bare Vec<String>
                // 也可能在 capture 没过滤 loopback 的早期版本里被写过
                // `["127.0.0.1", "1.1.1.1"]`。反序列化时同样过滤，
                // 否则 mhost 把污染数据重新写回系统 DNS，链路 2 复发。
                let filtered: Vec<String> = servers
                    .into_iter()
                    .filter(|x| !is_local_resolver(x))
                    .collect();
                Ok(OriginalDns::Manual(filtered))
            }
            Repr::Tagged(Tagged::DhcpEmpty) => Ok(OriginalDns::DhcpEmpty),
            Repr::Legacy(vec) => {
                // **fix (issue #152 hardening)**：legacy bare Vec<String>
                // 在做 `is_empty()` / `"Empty"` 判定前先过滤 loopback。
                // - `["127.0.0.1", "1.1.1.1"]` → `["1.1.1.1"]` → Manual
                // - `["127.0.0.1"]`             → `[]`          → DhcpEmpty
                let filtered: Vec<String> =
                    vec.into_iter().filter(|x| !is_local_resolver(x)).collect();
                if filtered.is_empty() || filtered.iter().any(|s| s == "Empty") {
                    Ok(OriginalDns::DhcpEmpty)
                } else {
                    Ok(OriginalDns::Manual(filtered))
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
    // AdBlock tests (issue #130)
    // -----------------------------------------------------------------------

    #[test]
    fn test_ad_block_response_default_is_zero_address() {
        assert_eq!(AdBlockResponse::default(), AdBlockResponse::ZeroAddress);
    }

    #[test]
    fn test_ad_block_response_serde_roundtrip() {
        for variant in [AdBlockResponse::ZeroAddress, AdBlockResponse::NxDomain] {
            let json = serde_json::to_string(&variant).unwrap();
            let restored: AdBlockResponse = serde_json::from_str(&json).unwrap();
            assert_eq!(variant, restored);
        }
        // snake_case wire format (serde `rename_all = "snake_case"` treats the
        // capital N in `NxDomain` as a word boundary → `nx_domain`)
        assert_eq!(
            serde_json::to_string(&AdBlockResponse::ZeroAddress).unwrap(),
            "\"zero_address\""
        );
        assert_eq!(
            serde_json::to_string(&AdBlockResponse::NxDomain).unwrap(),
            "\"nx_domain\""
        );
    }

    #[test]
    fn test_ad_block_source_serde_skips_none_optionals() {
        let source = AdBlockSource {
            source_id: SourceId(Uuid::new_v4()),
            name: "StevenBlack".to_string(),
            url: "https://example.com/list.txt".to_string(),
            enabled: true,
            response: AdBlockResponse::NxDomain,
            last_fetched_at: None,
            last_error: None,
            rule_count: 0,
            etag: None,
        };
        let json = serde_json::to_string(&source).unwrap();
        assert!(!json.contains("last_fetched_at"));
        assert!(!json.contains("last_error"));
        assert!(!json.contains("etag"));
        let restored: AdBlockSource = serde_json::from_str(&json).unwrap();
        assert_eq!(source, restored);
    }

    #[test]
    fn test_ad_block_source_serde_includes_some_optionals() {
        let source = AdBlockSource {
            source_id: SourceId(Uuid::new_v4()),
            name: "Test".to_string(),
            url: "https://example.com/list.txt".to_string(),
            enabled: false,
            response: AdBlockResponse::ZeroAddress,
            last_fetched_at: Some("2026-07-28T00:00:00Z".parse().unwrap()),
            last_error: Some("timeout".to_string()),
            rule_count: 42,
            etag: Some("W/\"abc\"".to_string()),
        };
        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("last_fetched_at"));
        assert!(json.contains("last_error"));
        assert!(json.contains("etag"));
        let restored: AdBlockSource = serde_json::from_str(&json).unwrap();
        assert_eq!(source, restored);
    }

    #[test]
    fn test_ad_block_state_default() {
        let state = AdBlockState::default();
        assert!(!state.enabled);
        assert!(state.sources.is_empty());
        assert!(state.whitelist.is_empty());
        assert!(state.auto_refresh_enabled);
        assert_eq!(state.refresh_interval_hours, 24);
    }

    #[test]
    fn test_ad_block_state_serde_roundtrip() {
        let state = AdBlockState {
            enabled: true,
            sources: vec![AdBlockSource {
                source_id: SourceId(Uuid::new_v4()),
                name: "S1".to_string(),
                url: "https://example.com/s1".to_string(),
                enabled: true,
                response: AdBlockResponse::NxDomain,
                last_fetched_at: None,
                last_error: None,
                rule_count: 100,
                etag: None,
            }],
            whitelist: vec!["trusted.example.com".to_string()],
            auto_refresh_enabled: true,
            refresh_interval_hours: 12,
        };
        let json = serde_json::to_string(&state).unwrap();
        let restored: AdBlockState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, restored);
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

    /// **fix (issue #152 hardening)**：`restore_argv` 必须过滤 loopback，
    /// 防止 legacy on-disk 污染数据被写回系统 DNS。
    #[test]
    fn test_original_dns_restore_argv_strips_loopback() {
        // 混合：保留非 loopback，过滤掉 127.0.0.1 和 ::1
        assert_eq!(
            OriginalDns::Manual(vec![
                "127.0.0.1".to_string(),
                "8.8.8.8".to_string(),
                "::1".to_string(),
                "1.1.1.1".to_string(),
            ])
            .restore_argv(),
            vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()]
        );

        // 全 loopback → 退回 DhcpEmpty 语义（["Empty"]），绝不传 127.0.0.1
        assert_eq!(
            OriginalDns::Manual(vec!["127.0.0.1".to_string(), "::1".to_string()]).restore_argv(),
            vec!["Empty".to_string()]
        );

        // unspecified 也算「本地」（0.0.0.0 是某些 OS 的 placeholder）
        assert_eq!(
            OriginalDns::Manual(vec!["0.0.0.0".to_string(), "8.8.8.8".to_string()]).restore_argv(),
            vec!["8.8.8.8".to_string()]
        );

        // host:port 形式也能识别
        assert_eq!(
            OriginalDns::Manual(vec!["127.0.0.1:53".to_string(), "8.8.8.8".to_string()])
                .restore_argv(),
            vec!["8.8.8.8".to_string()]
        );

        // DhcpEmpty 路径不受影响
        assert_eq!(
            OriginalDns::DhcpEmpty.restore_argv(),
            vec!["Empty".to_string()]
        );
    }

    /// **fix (issue #152 hardening)**：legacy bare Vec<String> 反序列化
    /// 时同样过滤 loopback；legacy `["127.0.0.1"]` 必须迁移成 DhcpEmpty。
    #[test]
    fn test_original_dns_deserialize_legacy_vec_with_loopback_filters() {
        // 混合：保留非 loopback
        let legacy = r#"["127.0.0.1", "8.8.8.8"]"#;
        let restored: OriginalDns = serde_json::from_str(legacy).unwrap();
        assert_eq!(restored, OriginalDns::Manual(vec!["8.8.8.8".to_string()]));

        // 全 loopback → DhcpEmpty
        let legacy = r#"["127.0.0.1"]"#;
        let restored: OriginalDns = serde_json::from_str(legacy).unwrap();
        assert_eq!(restored, OriginalDns::DhcpEmpty);

        // 全 unspecified → DhcpEmpty
        let legacy = r#"["0.0.0.0"]"#;
        let restored: OriginalDns = serde_json::from_str(legacy).unwrap();
        assert_eq!(restored, OriginalDns::DhcpEmpty);

        // 全 loopback + "Empty" placeholder → DhcpEmpty（filter 不该破坏
        // "Empty" 占位符的语义）
        let legacy = r#"["Empty", "127.0.0.1"]"#;
        let restored: OriginalDns = serde_json::from_str(legacy).unwrap();
        assert_eq!(restored, OriginalDns::DhcpEmpty);
    }

    /// **fix (issue #152 hardening)**：tagged `Manual{servers}` 反序列化
    /// 也过滤 loopback。覆盖 pre-fix manifest 写过的
    /// `{"kind":"manual","servers":["127.0.0.1","8.8.8.8"]}` 这种污染数据。
    #[test]
    fn test_original_dns_deserialize_tagged_manual_with_loopback_filters() {
        let json = r#"{"kind":"manual","servers":["127.0.0.1", "8.8.8.8"]}"#;
        let restored: OriginalDns = serde_json::from_str(json).unwrap();
        assert_eq!(restored, OriginalDns::Manual(vec!["8.8.8.8".to_string()]));

        // 全 loopback → 过滤后 vec 为空 → Manual(vec![]) 保留，调用方
        // restore_argv() 会进一步退回 ["Empty"]。
        let json = r#"{"kind":"manual","servers":["127.0.0.1"]}"#;
        let restored: OriginalDns = serde_json::from_str(json).unwrap();
        assert_eq!(restored, OriginalDns::Manual(vec![]));
        assert_eq!(restored.restore_argv(), vec!["Empty".to_string()]);
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
}
