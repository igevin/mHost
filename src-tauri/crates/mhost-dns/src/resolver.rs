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
    ///
    /// **冲突顺序确定性（issue #67 DNS 多 Profile）**：启用 profile 按
    /// `name` 字典序排序后再迭代，因此 first-wins 与文件系统读取顺序无关，
    /// 同一组 profile 在不同启动中结果一致。
    pub fn rebuild(&self, profiles: &[Profile]) {
        // 收集 DNS 模式且启用的 profile，按 name 排序保证 first-wins 确定性。
        let mut active: Vec<&Profile> = profiles
            .iter()
            .filter(|p| p.mode == ProfileMode::Dns && p.enabled)
            .collect();
        active.sort_by(|a, b| a.name.cmp(&b.name));

        let mut new_rules = HashMap::new();

        for profile in active {
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
    ///
    /// **支持 suffix 匹配（fix #79）**：从完整域名开始，逐级向上找父域，
    /// 第一个匹配胜出。
    /// 例如：注册 `example.com → 0.0.0.0` 后，
    /// `ad.example.com` / `tracker.example.com` / `example.com` 都会命中。
    /// `something.com` 不会命中（除非显式注册 `com`）。
    pub fn resolve(&self, domain: &str) -> Option<IpAddr> {
        let lookup = |map: &HashMap<String, IpAddr>| -> Option<IpAddr> {
            let mut current = domain;
            loop {
                if let Some(ip) = map.get(current) {
                    return Some(*ip);
                }
                let pos = current.find('.')?;
                current = &current[pos + 1..];
            }
        };
        match self.rules.read() {
            Ok(guard) => lookup(&guard),
            Err(poisoned) => lookup(&poisoned.into_inner()),
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

    /// issue #67：first-wins 必须按 `name` 字典序决定，
    /// 与输入顺序无关（之前依赖 `fs::read_dir` 系统调用顺序，不确定）。
    #[test]
    fn test_rebuild_sort_by_name_deterministic() {
        // zeta 在前 / alpha 在后：alpha 应胜出（name 字典序在前）
        let zeta = make_profile(
            "zeta",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("10.0.0.1"), vec!["shared.com"], true)],
        );
        let alpha = make_profile(
            "alpha",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("10.0.0.2"), vec!["shared.com"], true)],
        );
        // 输入顺序：zeta 在前（按 slice 顺序）
        let engine = RuleEngine::new();
        engine.rebuild(&[zeta, alpha]);
        // alpha 按 name 排序在前，应胜出
        assert_eq!(
            engine.resolve("shared.com"),
            Some("10.0.0.2".parse::<IpAddr>().unwrap()),
            "alpha should win by name order, not input order"
        );

        // 反向输入：alpha 在前 / zeta 在后；结果必须一致（仍是 alpha 胜）
        let alpha2 = make_profile(
            "alpha",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("10.0.0.3"), vec!["shared.com"], true)],
        );
        let zeta2 = make_profile(
            "zeta",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("10.0.0.4"), vec!["shared.com"], true)],
        );
        let engine2 = RuleEngine::new();
        engine2.rebuild(&[alpha2, zeta2]);
        assert_eq!(
            engine2.resolve("shared.com"),
            Some("10.0.0.3".parse::<IpAddr>().unwrap()),
            "reversed input order must yield same winner (alpha)"
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
        assert_eq!(
            engine.resolve("b.com"),
            Some("192.168.1.1".parse().unwrap())
        );
    }

    // -----------------------------------------------------------------------
    // Suffix 匹配测试（fix #79）
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_suffix_match_basic() {
        // 注册 example.com → 0.0.0.0，ad.example.com 应该通过 suffix 匹配命中
        let engine = RuleEngine::new();
        let profile = make_profile(
            "p1",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("0.0.0.0"), vec!["example.com"], true)],
        );
        engine.rebuild(&[profile]);

        // 完整域名本身命中
        assert_eq!(
            engine.resolve("example.com"),
            Some("0.0.0.0".parse().unwrap())
        );
        // 一级子域名命中
        assert_eq!(
            engine.resolve("ad.example.com"),
            Some("0.0.0.0".parse().unwrap())
        );
        // 多级子域名命中
        assert_eq!(
            engine.resolve("a.b.c.example.com"),
            Some("0.0.0.0".parse().unwrap())
        );
        // 兄弟域名（共享后缀但不是子域名）不命中
        assert_eq!(engine.resolve("notexample.com"), None);
        assert_eq!(engine.resolve("example.org"), None);
        // 完全不同域名不命中
        assert_eq!(engine.resolve("google.com"), None);
    }

    #[test]
    fn test_resolve_specificity_more_specific_wins() {
        // 同时注册 example.com 和 sub.example.com，a.sub.example.com
        // 应优先匹配 sub.example.com（更具体的）
        let engine = RuleEngine::new();
        let profile = make_profile(
            "p1",
            ProfileMode::Dns,
            true,
            vec![
                make_rule(Some("0.0.0.0"), vec!["example.com"], true),
                make_rule(Some("127.0.0.1"), vec!["sub.example.com"], true),
            ],
        );
        engine.rebuild(&[profile]);

        // a.sub.example.com → 127.0.0.1（sub.example.com 更具体）
        assert_eq!(
            engine.resolve("a.sub.example.com"),
            Some("127.0.0.1".parse().unwrap())
        );
        // b.example.com → 0.0.0.0（只有 example.com 匹配）
        assert_eq!(
            engine.resolve("b.example.com"),
            Some("0.0.0.0".parse().unwrap())
        );
        // example.com 本身 → 0.0.0.0
        assert_eq!(
            engine.resolve("example.com"),
            Some("0.0.0.0".parse().unwrap())
        );
    }

    #[test]
    fn test_resolve_no_match_unrelated_tld() {
        // 注册 example.com 后查 com.example.org —— 后缀链找不到共同祖先，不命中
        let engine = RuleEngine::new();
        let profile = make_profile(
            "p1",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("0.0.0.0"), vec!["example.com"], true)],
        );
        engine.rebuild(&[profile]);

        assert_eq!(engine.resolve("com.example.org"), None);
        assert_eq!(engine.resolve("example.org"), None);
        // 只有 example.com 链下的才命中
        assert_eq!(
            engine.resolve("sub.example.com"),
            Some("0.0.0.0".parse().unwrap())
        );
    }

    #[test]
    fn test_resolve_tld_alone_requires_explicit_rule() {
        // TLD 单独（如 com）必须显式注册才会命中
        // 注册 example.com 不会让 com 单独命中
        let engine = RuleEngine::new();
        let profile = make_profile(
            "p1",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("0.0.0.0"), vec!["example.com"], true)],
        );
        engine.rebuild(&[profile]);

        // com 单独不命中（避免误伤所有 .com 域名）
        assert_eq!(engine.resolve("com"), None);
        // 除非显式注册 com
        let profile2 = make_profile(
            "p2",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("0.0.0.0"), vec!["com"], true)],
        );
        engine.rebuild(&[profile2]);
        // 显式注册 com 后才匹配
        assert_eq!(engine.resolve("com"), Some("0.0.0.0".parse().unwrap()));
        assert_eq!(
            engine.resolve("anything.com"),
            Some("0.0.0.0".parse().unwrap())
        );
    }
}
