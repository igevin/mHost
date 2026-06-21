//! Hosts file formatter

use mhost_core::HostRule;
use std::fmt::Write;

/// Re-export the format function from the parser module.
/// Multi-domain rules are expanded to one line per domain.
pub fn format_rules(rules: &[HostRule]) -> String {
    let mut out = String::new();
    for rule in rules {
        for domain in &rule.domains {
            if let Some(ref c) = rule.comment {
                writeln!(out, "{} {} # {}", rule.ip, domain, c).unwrap();
            } else {
                writeln!(out, "{} {}", rule.ip, domain).unwrap();
            }
        }
    }
    out
}

/// Format rules wrapped in a managed block.
pub fn format_managed_block(rules: &[HostRule]) -> String {
    if rules.is_empty() {
        return String::new();
    }
    let mut output = String::new();
    output.push_str("# ---- mHost start ----\n");
    output.push_str(&format_rules(rules));
    output.push_str("# ---- mHost end ----\n");
    output
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mhost_core::HostRule;
    use std::net::IpAddr;

    fn make_rule(ip: &str, domains: Vec<&str>) -> HostRule {
        HostRule::new(
            ip.parse::<IpAddr>().unwrap(),
            domains.into_iter().map(|s| s.to_string()).collect(),
        )
    }

    #[test]
    fn test_format_rules_single_domain() {
        let rules = vec![make_rule("127.0.0.1", vec!["example.com"])];
        assert_eq!(format_rules(&rules), "127.0.0.1 example.com\n");
    }

    #[test]
    fn test_format_rules_multi_domain() {
        let rules = vec![make_rule("127.0.0.1", vec!["a.com", "b.com"])];
        assert_eq!(format_rules(&rules), "127.0.0.1 a.com\n127.0.0.1 b.com\n");
    }

    #[test]
    fn test_format_rules_with_comment() {
        let mut rule = make_rule("127.0.0.1", vec!["example.com"]);
        rule.comment = Some("dev".to_string());
        assert_eq!(format_rules(&[rule]), "127.0.0.1 example.com # dev\n");
    }

    #[test]
    fn test_format_rules_empty() {
        assert_eq!(format_rules(&[]), "");
    }

    #[test]
    fn test_format_managed_block() {
        let rules = vec![make_rule("127.0.0.1", vec!["x.com"])];
        let output = format_managed_block(&rules);
        assert!(output.contains("# ---- mHost start ----"));
        assert!(output.contains("127.0.0.1 x.com"));
        assert!(output.contains("# ---- mHost end ----"));
    }

    #[test]
    fn test_format_managed_block_empty() {
        assert_eq!(format_managed_block(&[]), "");
    }
}
