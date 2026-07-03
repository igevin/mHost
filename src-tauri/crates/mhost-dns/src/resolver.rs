use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::RwLock;

use mhost_core::{Profile, ProfileMode};
use tracing::warn;

/// 规则引擎：维护域名到 IP 的映射表。
pub struct RuleEngine {
    /// 域名 -> IP 映射（所有启用的 DNS 模式 Profile 规则的并集）
    rules: RwLock<HashMap<String, IpAddr>>,
}

impl RuleEngine {
    pub fn new() -> Self {
        Self {
            rules: RwLock::new(HashMap::new()),
        }
    }

    /// 从 Profile 列表重建规则表。
    ///
    /// 只处理 `mode == ProfileMode::Dns` 且 `enabled == true` 的 Profile，
    /// 每个 Profile 中只处理 `enabled == true` 且 `ip` 不为 `None` 的 HostRule。
    /// 多个 Profile 的相同域名保留第一个，后续冲突记录警告。
    pub fn rebuild(&self, profiles: &[Profile]) {
        let mut new_rules = HashMap::new();

        for profile in profiles {
            if profile.mode != ProfileMode::Dns || !profile.enabled {
                continue;
            }
            for rule in &profile.rules {
                if !rule.enabled {
                    continue;
                }
                let Some(ip) = rule.ip else {
                    continue;
                };
                for domain in &rule.domains {
                    if new_rules.contains_key(domain) {
                        warn!(
                            domain = domain,
                            "Domain conflict detected in DNS rules, keeping first entry"
                        );
                        continue;
                    }
                    new_rules.insert(domain.clone(), ip);
                }
            }
        }

        match self.rules.write() {
            Ok(mut guard) => {
                *guard = new_rules;
            }
            Err(poisoned) => {
                // 锁中毒时尝试恢复数据
                let mut guard = poisoned.into_inner();
                *guard = new_rules;
            }
        }
    }

    /// 查询域名对应的 IP。
    pub fn resolve(&self, domain: &str) -> Option<IpAddr> {
        match self.rules.read() {
            Ok(guard) => guard.get(domain).copied(),
            Err(poisoned) => {
                let guard = poisoned.into_inner();
                guard.get(domain).copied()
            }
        }
    }

    /// 获取当前规则数量。
    pub fn rule_count(&self) -> usize {
        match self.rules.read() {
            Ok(guard) => guard.len(),
            Err(poisoned) => {
                let guard = poisoned.into_inner();
                guard.len()
            }
        }
    }
}

impl Default for RuleEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use mhost_core::{HostRule, Profile, ProfileMode, RuleId};
    use uuid::Uuid;

    use super::*;

    fn make_profile(name: &str, mode: ProfileMode, enabled: bool, rules: Vec<HostRule>) -> Profile {
        Profile {
            id: mhost_core::ProfileId(Uuid::new_v4()),
            name: name.to_string(),
            description: None,
            enabled,
            protected: false,
            tags: vec![],
            rules,
            mode,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn make_rule(ip: Option<&str>, domains: Vec<&str>, enabled: bool) -> HostRule {
        HostRule {
            id: RuleId(Uuid::new_v4()),
            ip: ip.map(|s| s.parse().unwrap()),
            domains: domains.iter().map(|d| d.to_string()).collect(),
            enabled,
            comment: None,
            source: mhost_core::RuleSource::Manual,
            line_number: None,
        }
    }

    #[test]
    fn test_empty_rules() {
        let engine = RuleEngine::new();
        engine.rebuild(&[]);
        assert_eq!(engine.rule_count(), 0);
        assert_eq!(engine.resolve("example.com"), None);
    }

    #[test]
    fn test_single_profile_single_rule() {
        let engine = RuleEngine::new();
        let profile = make_profile(
            "p1",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("127.0.0.1"), vec!["example.com"], true)],
        );
        engine.rebuild(&[profile]);

        assert_eq!(engine.rule_count(), 1);
        assert_eq!(
            engine.resolve("example.com"),
            Some("127.0.0.1".parse::<IpAddr>().unwrap())
        );
        assert_eq!(engine.resolve("notfound.com"), None);
    }

    #[test]
    fn test_multi_profile_union() {
        let engine = RuleEngine::new();
        let p1 = make_profile(
            "p1",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("127.0.0.1"), vec!["a.com"], true)],
        );
        let p2 = make_profile(
            "p2",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("192.168.1.1"), vec!["b.com"], true)],
        );
        engine.rebuild(&[p1, p2]);

        assert_eq!(engine.rule_count(), 2);
        assert_eq!(
            engine.resolve("a.com"),
            Some("127.0.0.1".parse::<IpAddr>().unwrap())
        );
        assert_eq!(
            engine.resolve("b.com"),
            Some("192.168.1.1".parse::<IpAddr>().unwrap())
        );
    }

    #[test]
    fn test_domain_conflict_keep_first() {
        let engine = RuleEngine::new();
        let p1 = make_profile(
            "p1",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("127.0.0.1"), vec!["example.com"], true)],
        );
        let p2 = make_profile(
            "p2",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("192.168.1.1"), vec!["example.com"], true)],
        );
        engine.rebuild(&[p1, p2]);

        assert_eq!(engine.rule_count(), 1);
        assert_eq!(
            engine.resolve("example.com"),
            Some("127.0.0.1".parse::<IpAddr>().unwrap())
        );
    }

    #[test]
    fn test_comment_line_skipped() {
        let engine = RuleEngine::new();
        let profile = make_profile(
            "p1",
            ProfileMode::Dns,
            true,
            vec![
                make_rule(Some("127.0.0.1"), vec!["a.com"], true),
                make_rule(None, vec![], true), // 注释行，ip 为 None
            ],
        );
        engine.rebuild(&[profile]);

        assert_eq!(engine.rule_count(), 1);
        assert_eq!(
            engine.resolve("a.com"),
            Some("127.0.0.1".parse::<IpAddr>().unwrap())
        );
    }

    #[test]
    fn test_disabled_rule_skipped() {
        let engine = RuleEngine::new();
        let profile = make_profile(
            "p1",
            ProfileMode::Dns,
            true,
            vec![
                make_rule(Some("127.0.0.1"), vec!["a.com"], true),
                make_rule(Some("192.168.1.1"), vec!["b.com"], false), // 禁用
            ],
        );
        engine.rebuild(&[profile]);

        assert_eq!(engine.rule_count(), 1);
        assert_eq!(engine.resolve("a.com"), Some("127.0.0.1".parse().unwrap()));
        assert_eq!(engine.resolve("b.com"), None);
    }

    #[test]
    fn test_disabled_profile_skipped() {
        let engine = RuleEngine::new();
        let p1 = make_profile(
            "p1",
            ProfileMode::Dns,
            false, // 禁用
            vec![make_rule(Some("127.0.0.1"), vec!["a.com"], true)],
        );
        engine.rebuild(&[p1]);

        assert_eq!(engine.rule_count(), 0);
        assert_eq!(engine.resolve("a.com"), None);
    }

    #[test]
    fn test_hosts_mode_profile_skipped() {
        let engine = RuleEngine::new();
        let p1 = make_profile(
            "p1",
            ProfileMode::Hosts, // hosts 模式
            true,
            vec![make_rule(Some("127.0.0.1"), vec!["a.com"], true)],
        );
        engine.rebuild(&[p1]);

        assert_eq!(engine.rule_count(), 0);
        assert_eq!(engine.resolve("a.com"), None);
    }

    #[test]
    fn test_multi_domain_per_rule() {
        let engine = RuleEngine::new();
        let profile = make_profile(
            "p1",
            ProfileMode::Dns,
            true,
            vec![make_rule(
                Some("127.0.0.1"),
                vec!["a.com", "b.com", "c.com"],
                true,
            )],
        );
        engine.rebuild(&[profile]);

        assert_eq!(engine.rule_count(), 3);
        assert_eq!(engine.resolve("a.com"), Some("127.0.0.1".parse().unwrap()));
        assert_eq!(engine.resolve("b.com"), Some("127.0.0.1".parse().unwrap()));
        assert_eq!(engine.resolve("c.com"), Some("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn test_rebuild_replaces_old_rules() {
        let engine = RuleEngine::new();
        let p1 = make_profile(
            "p1",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("127.0.0.1"), vec!["a.com"], true)],
        );
        engine.rebuild(&[p1]);
        assert_eq!(engine.rule_count(), 1);

        let p2 = make_profile(
            "p2",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("192.168.1.1"), vec!["b.com"], true)],
        );
        engine.rebuild(&[p2]);
        assert_eq!(engine.rule_count(), 1);
        assert_eq!(engine.resolve("a.com"), None);
        assert_eq!(engine.resolve("b.com"), Some("192.168.1.1".parse().unwrap()));
    }
}
