//! Conflict detection and analysis
//!
//! Conflicts occur when the same domain is mapped to different IPs
//! across enabled profiles. This module provides utilities for
//! analyzing and reporting conflicts.

use mhost_core::RuleConflict;

/// Check if a domain appears in any conflict.
pub fn is_conflicted(domain: &str, conflicts: &[RuleConflict]) -> bool {
    conflicts.iter().any(|c| c.domain == domain)
}

/// Get all conflicting domains as a sorted list.
pub fn conflict_domains(conflicts: &[RuleConflict]) -> Vec<String> {
    let mut domains: Vec<String> = conflicts.iter().map(|c| c.domain.clone()).collect();
    domains.sort();
    domains.dedup();
    domains
}

/// Count total number of conflicting rules (across all conflicts).
pub fn total_conflicted_rules(conflicts: &[RuleConflict]) -> usize {
    conflicts.iter().map(|c| c.rules.len()).sum()
}

/// Get the IPs involved in a conflict for a given domain.
pub fn conflict_ips(conflicts: &[RuleConflict], domain: &str) -> Vec<String> {
    conflicts
        .iter()
        .find(|c| c.domain == domain)
        .map(|c| {
            let mut ips: Vec<String> = c.rules.iter().map(|r| r.ip.to_string()).collect();
            ips.sort();
            ips.dedup();
            ips
        })
        .unwrap_or_default()
}

/// Get the source profile names involved in a conflict for a given domain.
pub fn conflict_sources(conflicts: &[RuleConflict], domain: &str) -> Vec<String> {
    conflicts
        .iter()
        .find(|c| c.domain == domain)
        .map(|c| {
            let mut sources: Vec<String> = c
                .rules
                .iter()
                .map(|r| r.source_profile_name.clone())
                .collect();
            sources.sort();
            sources.dedup();
            sources
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mhost_core::{ProfileId, ResolvedRule};
    use uuid::Uuid;

    fn make_conflict(domain: &str, ips: Vec<&str>, profiles: Vec<&str>) -> RuleConflict {
        let rules = ips
            .into_iter()
            .zip(profiles.into_iter())
            .map(|(ip, profile)| ResolvedRule {
                ip: ip.parse().unwrap(),
                domain: domain.to_string(),
                source_profile_id: ProfileId(Uuid::new_v4()),
                source_profile_name: profile.to_string(),
            })
            .collect();
        RuleConflict {
            domain: domain.to_string(),
            rules,
        }
    }

    #[test]
    fn test_is_conflicted() {
        let conflicts = vec![make_conflict(
            "x.com",
            vec!["127.0.0.1", "192.168.1.1"],
            vec!["p1", "p2"],
        )];

        let cases = vec![("x.com", true), ("y.com", false), ("x.co", false)];

        for (domain, expected) in cases {
            assert_eq!(
                is_conflicted(domain, &conflicts),
                expected,
                "case: {}",
                domain
            );
        }
    }

    #[test]
    fn test_conflict_domains() {
        let conflicts = vec![
            make_conflict("a.com", vec!["127.0.0.1", "192.168.1.1"], vec!["p1", "p2"]),
            make_conflict("b.com", vec!["10.0.0.1", "10.0.0.2"], vec!["p3", "p4"]),
        ];

        let domains = conflict_domains(&conflicts);
        assert_eq!(domains, vec!["a.com", "b.com"]);
    }

    #[test]
    fn test_total_conflicted_rules() {
        let cases = vec![
            (
                "two_rules",
                vec![make_conflict(
                    "x.com",
                    vec!["127.0.0.1", "192.168.1.1"],
                    vec!["p1", "p2"],
                )],
                2usize,
            ),
            (
                "three_rules",
                vec![make_conflict(
                    "x.com",
                    vec!["127.0.0.1", "192.168.1.1", "10.0.0.1"],
                    vec!["p1", "p2", "p3"],
                )],
                3usize,
            ),
            ("empty", vec![], 0usize),
        ];

        for (name, conflicts, expected) in cases {
            assert_eq!(
                total_conflicted_rules(&conflicts),
                expected,
                "case: {}",
                name
            );
        }
    }

    #[test]
    fn test_conflict_ips() {
        let conflicts = vec![make_conflict(
            "x.com",
            vec!["127.0.0.1", "192.168.1.1"],
            vec!["p1", "p2"],
        )];

        let ips = conflict_ips(&conflicts, "x.com");
        assert_eq!(ips, vec!["127.0.0.1", "192.168.1.1"]);

        let empty = conflict_ips(&conflicts, "y.com");
        assert!(empty.is_empty());
    }

    #[test]
    fn test_conflict_sources() {
        let conflicts = vec![make_conflict(
            "x.com",
            vec!["127.0.0.1", "192.168.1.1"],
            vec!["dev", "staging"],
        )];

        let sources = conflict_sources(&conflicts, "x.com");
        assert_eq!(sources, vec!["dev", "staging"]);

        let empty = conflict_sources(&conflicts, "y.com");
        assert!(empty.is_empty());
    }
}
