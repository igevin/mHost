pub mod conflict;
pub mod diff;
pub mod merge;
pub mod writer;

#[cfg(test)]
mod integration_tests;

pub use mhost_core::ApplyPlan;

use merge::Merger;
use mhost_core::{MhostError, Profile, ResolvedRule};

/// Generate an apply plan from profiles and current hosts content.
///
/// This function:
/// 1. Merges all enabled profiles' rules
/// 2. Detects conflicts
/// 3. Calculates diff against current hosts
/// 4. Returns a complete ApplyPlan
pub fn generate_plan(profiles: &[Profile], current_hosts: &str) -> Result<ApplyPlan, MhostError> {
    let merge_result = Merger::merge(profiles);
    let diff = diff::calculate_diff(current_hosts, &merge_result.rules);

    Ok(ApplyPlan {
        rules: merge_result.rules,
        conflicts: merge_result.conflicts,
        diff: diff.clone(),
        backup_required: !diff.added.is_empty() || !diff.removed.is_empty(),
    })
}

/// Format resolved rules as a hosts managed block.
///
/// Wraps the rules in `# ---- mHost start ----` / `# ---- mHost end ----` markers.
pub fn format_as_hosts(rules: &[ResolvedRule]) -> String {
    use mhost_hosts::formatter::format_managed_block;

    // Convert ResolvedRule to HostRule for formatting
    let host_rules: Vec<mhost_core::HostRule> = rules
        .iter()
        .map(|r| mhost_core::HostRule::new(r.ip, vec![r.domain.clone()]))
        .collect();

    format_managed_block(&host_rules)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use merge::test_helpers::profile_with_rules;

    #[test]
    fn test_generate_plan_basic() {
        let p1 = profile_with_rules("p1", vec![("127.0.0.1", "a.com")]);
        let current = "# existing hosts\n";

        let plan = generate_plan(&[p1], current).unwrap();
        assert_eq!(plan.rules.len(), 1);
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.diff.added.len(), 1);
        assert_eq!(plan.diff.added[0], "127.0.0.1 a.com");
        assert!(plan.backup_required);
    }

    #[test]
    fn test_generate_plan_with_conflict() {
        let p1 = profile_with_rules("p1", vec![("127.0.0.1", "x.com")]);
        let p2 = profile_with_rules("p2", vec![("192.168.1.1", "x.com")]);

        let plan = generate_plan(&[p1, p2], "# empty\n").unwrap();
        assert!(plan.rules.is_empty());
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].domain, "x.com");
    }

    #[test]
    fn test_generate_plan_empty_profiles() {
        let plan = generate_plan(&[], "# empty\n").unwrap();
        assert!(plan.rules.is_empty());
        assert!(plan.conflicts.is_empty());
        assert!(plan.diff.added.is_empty());
        assert!(plan.diff.removed.is_empty());
        // No changes -> backup not required
        assert!(!plan.backup_required);
    }

    #[test]
    fn test_generate_managed_block() {
        let p1 = profile_with_rules("p1", vec![("127.0.0.1", "x.com")]);
        let plan = generate_plan(&[p1], "# empty\n").unwrap();
        let hosts_text = format_as_hosts(&plan.rules);
        assert!(hosts_text.contains("# ---- mHost start ----"));
        assert!(hosts_text.contains("# ---- mHost end ----"));
        assert!(hosts_text.contains("127.0.0.1 x.com"));
    }

    #[test]
    fn test_generate_managed_block_empty_rules() {
        let hosts_text = format_as_hosts(&[]);
        assert_eq!(hosts_text, "");
    }

    #[test]
    fn test_generate_plan_multiple_rules() {
        let p1 = profile_with_rules(
            "p1",
            vec![
                ("127.0.0.1", "a.com"),
                ("127.0.0.1", "b.com"),
                ("::1", "localhost"),
            ],
        );

        let plan = generate_plan(&[p1], "# empty\n").unwrap();
        assert_eq!(plan.rules.len(), 3);

        let hosts_text = format_as_hosts(&plan.rules);
        assert!(hosts_text.contains("127.0.0.1 a.com"));
        assert!(hosts_text.contains("127.0.0.1 b.com"));
        assert!(hosts_text.contains("::1 localhost"));
    }

    #[test]
    fn test_generate_plan_with_existing_rules() {
        // Current hosts has a.com and localhost (both outside managed block)
        // Target profile has a.com and b.com
        // Since only managed-block rules participate in diff, a.com is added
        // and localhost is ignored (user-managed, preserved as-is)
        let current = "127.0.0.1 a.com\n::1 localhost\n";
        let p1 = profile_with_rules("p1", vec![("127.0.0.1", "a.com"), ("127.0.0.1", "b.com")]);

        let plan = generate_plan(&[p1], current).unwrap();
        assert_eq!(plan.diff.added.len(), 2);
        assert!(plan.diff.added.contains(&"127.0.0.1 a.com".to_string()));
        assert!(plan.diff.added.contains(&"127.0.0.1 b.com".to_string()));
        assert!(plan.diff.removed.is_empty());
        assert!(plan.diff.unchanged.is_empty());
    }

    #[test]
    fn test_generate_plan_with_existing_managed_block() {
        // Current hosts has a managed block with a.com and localhost
        // Target profile has a.com and b.com
        // localhost is in managed block but not in profile -> removed
        let current = r#"# ---- mHost start ----
127.0.0.1 a.com
::1 localhost
# ---- mHost end ----
"#;
        let p1 = profile_with_rules("p1", vec![("127.0.0.1", "a.com"), ("127.0.0.1", "b.com")]);

        let plan = generate_plan(&[p1], current).unwrap();
        assert_eq!(plan.diff.added.len(), 1);
        assert_eq!(plan.diff.added[0], "127.0.0.1 b.com");
        assert_eq!(plan.diff.removed.len(), 1);
        assert_eq!(plan.diff.removed[0], "::1 localhost");
        assert_eq!(plan.diff.unchanged.len(), 1);
        assert_eq!(plan.diff.unchanged[0], "127.0.0.1 a.com");
    }
}
