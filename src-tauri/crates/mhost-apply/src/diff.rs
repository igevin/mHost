//! Diff generation
//!
//! Calculates the difference between the current hosts file content
//! and the target resolved rules, considering the managed block markers.

use mhost_core::{HostsDiff, ResolvedRule};
use mhost_hosts::Parser;
use std::collections::HashSet;

/// Calculate the diff between current hosts content and target rules.
///
/// The algorithm:
/// 1. Parse current hosts to extract existing rules (ignoring the managed block)
/// 2. Build target lines from resolved rules
/// 3. Compare to produce added/removed/unchanged
pub fn calculate_diff(current_hosts: &str, resolved_rules: &[ResolvedRule]) -> HostsDiff {
    // Extract existing rules from current hosts (outside managed block)
    let existing_lines = extract_existing_lines(current_hosts);

    // Build target lines from resolved rules
    let target_lines: Vec<String> = resolved_rules
        .iter()
        .map(|r| format!("{} {}", r.ip, r.domain))
        .collect();

    // Compute diff using set operations
    let existing_set: HashSet<String> = existing_lines.iter().cloned().collect();
    let target_set: HashSet<String> = target_lines.iter().cloned().collect();

    let mut added: Vec<String> = target_set.difference(&existing_set).cloned().collect();

    let mut removed: Vec<String> = existing_set.difference(&target_set).cloned().collect();

    let mut unchanged: Vec<String> = existing_set.intersection(&target_set).cloned().collect();

    // Sort for deterministic output
    added.sort();
    removed.sort();
    unchanged.sort();

    HostsDiff {
        added,
        removed,
        unchanged,
    }
}

/// Extract existing host lines from the managed block of the current hosts content.
///
/// Only rules inside the `# ---- mHost start/end ----` managed block are
/// considered for diff calculation. Lines outside the managed block are
/// user-managed and preserved as-is.
fn extract_existing_lines(current_hosts: &str) -> Vec<String> {
    let managed_range = Parser::extract_managed_block(current_hosts);

    let content_to_parse = match managed_range {
        Some((start, end)) => {
            let lines: Vec<&str> = current_hosts.lines().collect();
            // Extract lines strictly between start and end markers
            if start < end.saturating_sub(1) {
                lines[start + 1..end].join("\n")
            } else {
                String::new()
            }
        }
        None => String::new(),
    };

    let parse_result = Parser::parse(&content_to_parse);

    // Log parse errors so they are not silently discarded
    for err in &parse_result.errors {
        eprintln!("[mhost-apply] parse error in managed block: {}", err);
    }

    // Convert parsed rules back to lines (one per domain)
    let mut lines = Vec::new();
    for rule in &parse_result.rules {
        for domain in &rule.domains {
            lines.push(format!("{} {}", rule.ip, domain));
        }
    }

    lines
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mhost_core::{ProfileId, ResolvedRule};
    use uuid::Uuid;

    fn make_rule(ip: &str, domain: &str, profile_name: &str) -> ResolvedRule {
        ResolvedRule {
            ip: ip.parse().unwrap(),
            domain: domain.to_string(),
            source_profile_id: ProfileId(Uuid::new_v4()),
            source_profile_name: profile_name.to_string(),
        }
    }

    #[test]
    fn test_diff_all_new() {
        let current = "# Some existing comment\n";
        let rules = vec![
            make_rule("127.0.0.1", "a.com", "p1"),
            make_rule("127.0.0.1", "b.com", "p1"),
        ];

        let diff = calculate_diff(current, &rules);
        assert_eq!(diff.added.len(), 2);
        assert!(diff.added.contains(&"127.0.0.1 a.com".to_string()));
        assert!(diff.added.contains(&"127.0.0.1 b.com".to_string()));
        assert!(diff.removed.is_empty());
        assert!(diff.unchanged.is_empty());
    }

    #[test]
    fn test_diff_no_change_in_managed_block() {
        let current = r#"# ---- mHost start ----
127.0.0.1 a.com
127.0.0.1 b.com
# ---- mHost end ----
"#;
        let rules = vec![
            make_rule("127.0.0.1", "a.com", "p1"),
            make_rule("127.0.0.1", "b.com", "p1"),
        ];

        let diff = calculate_diff(current, &rules);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert_eq!(diff.unchanged.len(), 2);
        assert!(diff.unchanged.contains(&"127.0.0.1 a.com".to_string()));
        assert!(diff.unchanged.contains(&"127.0.0.1 b.com".to_string()));
    }

    #[test]
    fn test_diff_some_added_some_removed_in_managed_block() {
        let current = r#"# ---- mHost start ----
127.0.0.1 a.com
127.0.0.1 old.com
# ---- mHost end ----
"#;
        let rules = vec![
            make_rule("127.0.0.1", "a.com", "p1"),
            make_rule("127.0.0.1", "b.com", "p1"),
        ];

        let diff = calculate_diff(current, &rules);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0], "127.0.0.1 b.com");
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0], "127.0.0.1 old.com");
        assert_eq!(diff.unchanged.len(), 1);
        assert_eq!(diff.unchanged[0], "127.0.0.1 a.com");
    }

    #[test]
    fn test_diff_with_managed_block() {
        let current = r#"# Original content
# ---- mHost start ----
127.0.0.1 a.com
# ---- mHost end ----
# Footer
"#;
        let rules = vec![
            make_rule("127.0.0.1", "a.com", "p1"),
            make_rule("127.0.0.1", "b.com", "p1"),
        ];

        let diff = calculate_diff(current, &rules);
        // a.com exists in managed block, b.com is new
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0], "127.0.0.1 b.com");
        assert!(diff.removed.is_empty());
        assert_eq!(diff.unchanged.len(), 1);
        assert_eq!(diff.unchanged[0], "127.0.0.1 a.com");
    }

    #[test]
    fn test_diff_empty_current() {
        let rules = vec![make_rule("127.0.0.1", "example.com", "p1")];
        let diff = calculate_diff("", &rules);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0], "127.0.0.1 example.com");
        assert!(diff.removed.is_empty());
        assert!(diff.unchanged.is_empty());
    }

    #[test]
    fn test_diff_empty_rules() {
        // Rules outside managed block are ignored for diff calculation
        let current = "127.0.0.1 example.com\n";
        let diff = calculate_diff(current, &[]);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert!(diff.unchanged.is_empty());
    }

    #[test]
    fn test_diff_empty_rules_in_managed_block() {
        let current = r#"# ---- mHost start ----
127.0.0.1 example.com
# ---- mHost end ----
"#;
        let diff = calculate_diff(current, &[]);
        assert!(diff.added.is_empty());
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0], "127.0.0.1 example.com");
        assert!(diff.unchanged.is_empty());
    }

    #[test]
    fn test_diff_both_empty() {
        let diff = calculate_diff("", &[]);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert!(diff.unchanged.is_empty());
    }

    #[test]
    fn test_diff_ip_change_in_managed_block() {
        let current = r#"# ---- mHost start ----
127.0.0.1 example.com
# ---- mHost end ----
"#;
        let rules = vec![make_rule("192.168.1.1", "example.com", "p1")];

        let diff = calculate_diff(current, &rules);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0], "192.168.1.1 example.com");
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0], "127.0.0.1 example.com");
        assert!(diff.unchanged.is_empty());
    }

    #[test]
    fn test_diff_with_comments_and_empty_lines_in_managed_block() {
        let current = r#"# ---- mHost start ----
# Header
127.0.0.1 a.com

# Another comment
::1 localhost
# ---- mHost end ----
"#;
        let rules = vec![
            make_rule("127.0.0.1", "a.com", "p1"),
            make_rule("::1", "localhost", "p1"),
        ];

        let diff = calculate_diff(current, &rules);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert_eq!(diff.unchanged.len(), 2);
    }

    #[test]
    fn test_diff_multi_domain_rule_in_managed_block() {
        // Current hosts has a multi-domain rule inside managed block
        let current = r#"# ---- mHost start ----
127.0.0.1 a.com b.com
# ---- mHost end ----
"#;
        let rules = vec![make_rule("127.0.0.1", "a.com", "p1")];

        let diff = calculate_diff(current, &rules);
        assert!(diff.added.is_empty());
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0], "127.0.0.1 b.com");
        assert_eq!(diff.unchanged.len(), 1);
        assert_eq!(diff.unchanged[0], "127.0.0.1 a.com");
    }

    #[test]
    fn test_diff_outside_managed_block_ignored() {
        // Rules outside managed block should not participate in diff
        let current = r#"# Original content
127.0.0.1 a.com
# ---- mHost start ----
127.0.0.1 b.com
# ---- mHost end ----
# Footer
"#;
        let rules = vec![make_rule("127.0.0.1", "b.com", "p1")];

        let diff = calculate_diff(current, &rules);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert_eq!(diff.unchanged.len(), 1);
        assert_eq!(diff.unchanged[0], "127.0.0.1 b.com");
    }
}
