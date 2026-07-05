//! Rule merger
//!
//! Merges rules from multiple enabled profiles, detecting conflicts
//! where the same domain maps to different IPs.

use mhost_core::{Profile, ResolvedRule, RuleConflict};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;

/// Result of merging profiles
#[derive(Debug, Clone, PartialEq)]
pub struct MergeResult {
    pub rules: Vec<ResolvedRule>,
    pub conflicts: Vec<RuleConflict>,
}

/// Merger combines rules from all enabled profiles
pub struct Merger;

impl Merger {
    /// Merge all enabled profiles' rules into a flat list of resolved rules.
    ///
    /// Rules are expanded so each domain gets its own ResolvedRule.
    /// Conflicts are detected when the same domain maps to different IPs.
    ///
    /// **fix (P-R10, P-R11, issue #90)**:
    ///   - **P-R11**: `domain_to_ips` was `HashMap<String, Vec<String>>` and
    ///     stored IP as `String` (allocated per rule via `IpAddr::to_string`).
    ///     Now `HashMap<String, HashSet<IpAddr>>` — `IpAddr` itself is
    ///     `Hash + Eq`, no string allocation needed.
    ///   - **P-R10**: sort keys were `a.ip.to_string()` (alloc per
    ///     comparison). `IpAddr` implements `Ord` directly, so we sort by
    ///     `a.ip` / `b.ip` — zero allocation.
    pub fn merge(profiles: &[Profile]) -> MergeResult {
        // Collect all resolved rules from enabled profiles
        let mut all_rules: Vec<ResolvedRule> = Vec::new();

        for profile in profiles.iter().filter(|p| p.enabled) {
            for rule in profile.rules.iter().filter(|r| r.enabled) {
                // Skip comment-only rules — they don't contribute to DNS resolution
                let ip = match rule.ip {
                    Some(ip) => ip,
                    None => continue,
                };
                for domain in &rule.domains {
                    all_rules.push(ResolvedRule {
                        ip,
                        domain: domain.clone(),
                        source_profile_id: profile.id.clone(),
                        source_profile_name: profile.name.clone(),
                    });
                }
            }
        }

        // Group by (domain, ip) to deduplicate identical mappings
        // Then detect conflicts: same domain, different ip
        //
        // NOTE(phase-0): When the same (domain, ip) comes from multiple profiles,
        // only one source_profile is retained. This is acceptable for Phase 0
        // because only a single profile is enabled at a time. Multi-profile
        // conflict tracing is reserved for a future phase.
        let mut by_domain_ip: HashMap<(String, IpAddr), ResolvedRule> = HashMap::new();
        let mut domain_to_ips: HashMap<String, HashSet<IpAddr>> = HashMap::new();

        for rule in all_rules {
            let domain = rule.domain.clone();
            let key = (domain.clone(), rule.ip);

            by_domain_ip.entry(key).or_insert_with(|| ResolvedRule {
                ip: rule.ip,
                domain: domain.clone(),
                source_profile_id: rule.source_profile_id,
                source_profile_name: rule.source_profile_name,
            });

            // P-R11: insert IpAddr directly, no to_string() alloc. HashSet
            // auto-dedupes the IPs for "is conflict?" check.
            domain_to_ips.entry(domain).or_default().insert(rule.ip);
        }

        // Build conflicts: same domain with different IPs
        let mut conflicts: Vec<RuleConflict> = Vec::new();
        let mut conflict_domains: Vec<String> = Vec::new();

        for (domain, ips) in &domain_to_ips {
            if ips.len() > 1 {
                conflict_domains.push(domain.clone());
            }
        }

        // Sort for deterministic output
        conflict_domains.sort();

        for domain in &conflict_domains {
            let mut conflict_rules: Vec<ResolvedRule> = Vec::new();
            // P-R11: iterate HashSet<IpAddr> directly, no String clones
            for &ip in &domain_to_ips[domain] {
                if let Some(rule) = by_domain_ip.remove(&(domain.clone(), ip)) {
                    conflict_rules.push(rule);
                }
            }

            // P-R10: sort by IpAddr directly, no to_string() per comparison
            conflict_rules.sort_by_key(|a| a.ip);

            conflicts.push(RuleConflict {
                domain: domain.clone(),
                rules: conflict_rules,
            });
        }

        // Build final rules list: exclude domains that have conflicts
        let mut rules: Vec<ResolvedRule> = Vec::new();
        let conflict_domain_set: HashSet<&String> = conflict_domains.iter().collect();

        for ((domain, _ip), rule) in by_domain_ip {
            if !conflict_domain_set.contains(&domain) {
                rules.push(rule);
            }
        }

        // P-R10: sort by (domain, IpAddr) — both Ord, zero alloc.
        rules.sort_by(|a, b| a.domain.cmp(&b.domain).then_with(|| a.ip.cmp(&b.ip)));

        MergeResult { rules, conflicts }
    }
}

// ---------------------------------------------------------------------------
// Helpers for building test profiles
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod test_helpers {
    use mhost_core::{HostRule, Profile};
    use std::net::IpAddr;

    /// Create a profile with the given name and a set of (ip, domain) rules.
    pub fn profile_with_rules(name: &str, rules: Vec<(&str, &str)>) -> Profile {
        let mut profile = Profile::new(name);
        profile.enabled = true;
        for (ip, domain) in rules {
            profile.rules.push(HostRule::new(
                ip.parse::<IpAddr>().unwrap(),
                vec![domain.to_string()],
            ));
        }
        profile
    }

    /// Create a profile with multiple domains in a single rule.
    pub fn profile_with_multi_domain_rule(name: &str, ip: &str, domains: Vec<&str>) -> Profile {
        let mut profile = Profile::new(name);
        profile.enabled = true;
        profile.rules.push(HostRule::new(
            ip.parse::<IpAddr>().unwrap(),
            domains.into_iter().map(|s| s.to_string()).collect(),
        ));
        profile
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::test_helpers::*;
    use super::*;
    use mhost_core::HostRule;

    #[test]
    fn test_merge_single_profile() {
        let mut profile = Profile::new("dev");
        profile.enabled = true;
        profile.rules.push(HostRule::new(
            "127.0.0.1".parse().unwrap(),
            vec!["a.com".to_string(), "b.com".to_string()],
        ));

        let result = Merger::merge(&[profile]);
        assert_eq!(result.rules.len(), 2, "expected 2 rules (one per domain)");
        assert!(result.conflicts.is_empty());

        // Verify domain expansion
        let domains: Vec<_> = result.rules.iter().map(|r| r.domain.as_str()).collect();
        assert!(domains.contains(&"a.com"));
        assert!(domains.contains(&"b.com"));
    }

    #[test]
    fn test_merge_no_conflict() {
        let cases = vec![
            (
                "different_domains",
                vec![
                    profile_with_rules("p1", vec![("127.0.0.1", "a.com")]),
                    profile_with_rules("p2", vec![("127.0.0.1", "b.com")]),
                ],
                2usize,
                0usize,
            ),
            (
                "different_ips_different_domains",
                vec![
                    profile_with_rules("p1", vec![("127.0.0.1", "a.com")]),
                    profile_with_rules("p2", vec![("192.168.1.1", "b.com")]),
                ],
                2,
                0,
            ),
        ];

        for (name, profiles, expected_rules, expected_conflicts) in cases {
            let result = Merger::merge(&profiles);
            assert_eq!(
                result.rules.len(),
                expected_rules,
                "case: {} — rule count mismatch",
                name
            );
            assert_eq!(
                result.conflicts.len(),
                expected_conflicts,
                "case: {} — conflict count mismatch",
                name
            );
        }
    }

    #[test]
    fn test_merge_same_domain_same_ip() {
        let p1 = profile_with_rules("p1", vec![("127.0.0.1", "x.com")]);
        let p2 = profile_with_rules("p2", vec![("127.0.0.1", "x.com")]);

        let result = Merger::merge(&[p1, p2]);
        assert_eq!(
            result.rules.len(),
            1,
            "same domain+ip should merge into one rule"
        );
        assert!(result.conflicts.is_empty());
        assert_eq!(result.rules[0].domain, "x.com");
        assert_eq!(result.rules[0].ip.to_string(), "127.0.0.1");
    }

    #[test]
    fn test_merge_same_domain_different_ip() {
        let p1 = profile_with_rules("p1", vec![("127.0.0.1", "x.com")]);
        let p2 = profile_with_rules("p2", vec![("192.168.1.1", "x.com")]);

        let result = Merger::merge(&[p1, p2]);
        assert_eq!(
            result.rules.len(),
            0,
            "conflicted domain should not appear in rules"
        );
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].domain, "x.com");
        assert_eq!(result.conflicts[0].rules.len(), 2);
    }

    #[test]
    fn test_merge_disabled_profile_ignored() {
        let mut p1 = profile_with_rules("p1", vec![("127.0.0.1", "a.com")]);
        p1.enabled = false;
        let p2 = profile_with_rules("p2", vec![("127.0.0.1", "b.com")]);

        let result = Merger::merge(&[p1, p2]);
        assert_eq!(result.rules.len(), 1);
        assert_eq!(result.rules[0].domain, "b.com");
    }

    #[test]
    fn test_merge_disabled_rule_ignored() {
        let mut p1 = profile_with_rules("p1", vec![("127.0.0.1", "a.com")]);
        p1.rules[0].enabled = false;
        let p2 = profile_with_rules("p2", vec![("127.0.0.1", "b.com")]);

        let result = Merger::merge(&[p1, p2]);
        assert_eq!(result.rules.len(), 1);
        assert_eq!(result.rules[0].domain, "b.com");
    }

    #[test]
    fn test_merge_multi_domain_rule_expansion() {
        let p1 = profile_with_multi_domain_rule("p1", "127.0.0.1", vec!["a.com", "b.com"]);

        let result = Merger::merge(&[p1]);
        assert_eq!(result.rules.len(), 2);
        let domains: Vec<_> = result.rules.iter().map(|r| r.domain.as_str()).collect();
        assert!(domains.contains(&"a.com"));
        assert!(domains.contains(&"b.com"));
    }

    #[test]
    fn test_merge_conflict_preserves_all_variants() {
        let p1 = profile_with_rules("p1", vec![("127.0.0.1", "x.com")]);
        let p2 = profile_with_rules("p2", vec![("192.168.1.1", "x.com")]);
        let p3 = profile_with_rules("p3", vec![("10.0.0.1", "x.com")]);

        let result = Merger::merge(&[p1, p2, p3]);
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].rules.len(), 3);

        let ips: Vec<_> = result.conflicts[0]
            .rules
            .iter()
            .map(|r| r.ip.to_string())
            .collect();
        assert!(ips.contains(&"127.0.0.1".to_string()));
        assert!(ips.contains(&"192.168.1.1".to_string()));
        assert!(ips.contains(&"10.0.0.1".to_string()));
    }

    #[test]
    fn test_merge_empty_profiles() {
        let result = Merger::merge(&[]);
        assert!(result.rules.is_empty());
        assert!(result.conflicts.is_empty());
    }

    #[test]
    fn test_merge_no_enabled_profiles() {
        let mut p1 = profile_with_rules("p1", vec![("127.0.0.1", "a.com")]);
        p1.enabled = false;

        let result = Merger::merge(&[p1]);
        assert!(result.rules.is_empty());
        assert!(result.conflicts.is_empty());
    }

    #[test]
    fn test_merge_source_profile_info() {
        let p1 = profile_with_rules("dev-profile", vec![("127.0.0.1", "example.com")]);

        let result = Merger::merge(&[p1]);
        assert_eq!(result.rules.len(), 1);
        assert_eq!(result.rules[0].source_profile_name, "dev-profile");
        // Verify profile ID is set (non-nil)
        assert_ne!(result.rules[0].source_profile_id.0, uuid::Uuid::nil());
    }

    #[test]
    fn test_merge_comment_only_rule_ignored() {
        let mut p1 = profile_with_rules("p1", vec![("127.0.0.1", "a.com")]);
        p1.rules.push(HostRule::comment_only("# a comment"));

        let result = Merger::merge(&[p1]);
        // Comment-only rule should not be included in merged rules
        assert_eq!(result.rules.len(), 1);
        assert_eq!(result.rules[0].domain, "a.com");
    }
}
