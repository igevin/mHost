//! Verification for the hosts writer
//!
//! Verifies that the written hosts file content matches the expected
//! apply plan.

use mhost_core::{ApplyError, ApplyPlan, MhostError};
use mhost_hosts::Parser;
use std::collections::HashSet;
use std::fmt::Write as _;

/// Verify that the written content matches the expected plan.
///
/// **fix (P-R9, issue #90)**: previously each rule triggered a
/// `format!("{} {}", rule.ip, rule.domain)` allocation — 1000 rules meant
/// 1000 throwaway `String`s plus 1000 `HashSet::contains` lookups. Now we
/// reuse a single `String` buffer with `write!()`, clearing + refilling per
/// rule. Allocation count drops to 1.
pub fn verify(written: &str, plan: &ApplyPlan) -> Result<(), MhostError> {
    // Basic verification: check that the managed block markers exist
    // if the plan has rules, and that all expected rules are present.
    if plan.rules.is_empty() {
        // If no rules, there should be no managed block
        if Parser::extract_managed_block(written).is_some() {
            return Err(ApplyError::VerificationFailed(
                "expected no managed block but found one".to_string(),
            )
            .into());
        }
        return Ok(());
    }

    let block = Parser::extract_managed_block(written);
    if block.is_none() {
        return Err(ApplyError::VerificationFailed("managed block missing".to_string()).into());
    }

    // Extract managed block lines into a HashSet for O(1) lookup
    let managed_content = Parser::extract_managed_block_content(written).unwrap_or_default();
    let written_lines: HashSet<&str> = managed_content.lines().collect();

    // 64 bytes is enough for most IPv4/IPv6 + domain; we'll grow if needed.
    let mut buf = String::with_capacity(64);
    for rule in &plan.rules {
        buf.clear();
        // write! to String is infallible.
        write!(&mut buf, "{} {}", rule.ip, rule.domain).expect("writing to String never fails");
        if !written_lines.contains(buf.as_str()) {
            // borrow for the error message rather than re-formatting
            return Err(ApplyError::VerificationFailed(format!(
                "expected rule '{}' not found",
                buf
            ))
            .into());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mhost_core::{HostRule, HostsDiff, Profile, ProfileMode, ResolvedRule};
    use std::net::IpAddr;

    fn make_plan(rules: Vec<(IpAddr, &str)>) -> ApplyPlan {
        ApplyPlan {
            rules: rules
                .into_iter()
                .map(|(ip, domain)| ResolvedRule {
                    ip,
                    domain: domain.to_string(),
                    source_profile_id: Profile::new("src").id,
                    source_profile_name: "src".to_string(),
                })
                .collect(),
            conflicts: vec![],
            diff: HostsDiff {
                added: vec![],
                removed: vec![],
                unchanged: vec![],
            },
            backup_required: true,
        }
    }

    /// **fix (P-R9) regression test**: verify must correctly find all rules
    /// in a multi-rule plan. Validates the buffer-reuse rewrite did not
    /// break rule matching.
    #[test]
    fn test_verify_multiple_rules_match() {
        let managed =
            "# ---- mHost start ----\n127.0.0.1 a.com\n192.168.1.1 b.com\n# ---- mHost end ----";
        let plan = make_plan(vec![
            ("127.0.0.1".parse().unwrap(), "a.com"),
            ("192.168.1.1".parse().unwrap(), "b.com"),
        ]);
        assert!(verify(managed, &plan).is_ok());
    }

    #[test]
    fn test_verify_missing_rule_fails() {
        let managed = "# ---- mHost start ----\n127.0.0.1 a.com\n# ---- mHost end ----";
        let plan = make_plan(vec![
            ("127.0.0.1".parse().unwrap(), "a.com"),
            ("192.168.1.1".parse().unwrap(), "missing.com"), // not in managed
        ]);
        let err = verify(managed, &plan).unwrap_err();
        assert!(format!("{}", err).contains("missing.com"));
    }

    #[test]
    fn test_verify_no_rules_no_block() {
        // plan with no rules: managed block must NOT be present
        let no_block = "127.0.0.1 unmanaged.com";
        let empty_plan = ApplyPlan {
            rules: vec![],
            conflicts: vec![],
            diff: HostsDiff {
                added: vec![],
                removed: vec![],
                unchanged: vec![],
            },
            backup_required: false,
        };
        assert!(verify(no_block, &empty_plan).is_ok());

        // ...and must fail if a managed block exists
        let with_block = "# ---- mHost start ----\n# ---- mHost end ----";
        assert!(verify(with_block, &empty_plan).is_err());
    }

    /// IPv6 rule — make sure the buffer-reuse handles v6 display correctly.
    #[test]
    fn test_verify_ipv6_rule() {
        let managed = "# ---- mHost start ----\n::1 localhost\n# ---- mHost end ----";
        let plan = make_plan(vec![("::1".parse::<IpAddr>().unwrap(), "localhost")]);
        assert!(verify(managed, &plan).is_ok());
    }

    /// Suppress unused-import warning when running with `--no-cfg(test)`.
    #[allow(dead_code)]
    fn _suppress_unused(_: HostRule, _: ProfileMode) {}
}
