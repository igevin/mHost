//! Hosts file parser

use crate::validator;
use mhost_core::{HostRule, ParseError};
use std::net::IpAddr;
use std::str::FromStr;

/// Managed block markers
const MANAGED_START: &str = "# ---- mHost start ----";
const MANAGED_END: &str = "# ---- mHost end ----";

/// Result of parsing a hosts file
#[derive(Debug, PartialEq)]
pub struct ParseResult {
    pub rules: Vec<HostRule>,
    pub errors: Vec<ParseError>,
}

/// Hosts file parser
pub struct Parser;

impl Parser {
    /// Parse hosts text into rules and errors
    pub fn parse(input: &str) -> ParseResult {
        let mut rules = Vec::new();
        let mut errors = Vec::new();

        for line in input.lines() {
            match Self::parse_line(line) {
                Ok(Some(rule)) => rules.push(rule),
                Ok(None) => {}
                Err(err) => errors.push(err),
            }
        }

        ParseResult { rules, errors }
    }

    /// Format a slice of HostRule back into hosts text.
    /// Multi-domain rules are expanded to one line per domain.
    ///
    /// Delegates to [`crate::formatter::format_rules`].
    pub fn format(rules: &[HostRule]) -> String {
        crate::formatter::format_rules(rules)
    }

    /// Extract the line range (0-based, inclusive) of the managed block.
    /// Returns Some((start_line, end_line)) if exactly one pair of markers
    /// is found and start_line <= end_line.
    /// Returns None if markers are missing or malformed.
    pub fn extract_managed_block(input: &str) -> Option<(usize, usize)> {
        let mut start = None;
        let mut end = None;

        for (idx, line) in input.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed == MANAGED_START {
                // Multiple start markers -> malformed
                if start.is_some() {
                    return None;
                }
                start = Some(idx);
            } else if trimmed == MANAGED_END {
                // Multiple end markers -> malformed
                if end.is_some() {
                    return None;
                }
                end = Some(idx);
            }
        }

        match (start, end) {
            (Some(s), Some(e)) if s <= e => Some((s, e)),
            _ => None,
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn parse_line(line: &str) -> Result<Option<HostRule>, ParseError> {
        let trimmed = line.trim();

        // Empty or comment line -> no rule
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return Ok(None);
        }

        // Strip inline comment (everything after the first '#')
        let (content, _comment) = match trimmed.split_once('#') {
            Some((c, comment_text)) => (c.trim(), Some(comment_text.trim())),
            None => (trimmed, None),
        };

        if content.is_empty() {
            return Ok(None);
        }

        // Split by whitespace
        let tokens: Vec<&str> = content.split_whitespace().collect();
        if tokens.is_empty() {
            return Ok(None);
        }

        // First token must be an IP address
        let ip_str = tokens[0];
        let ip = match IpAddr::from_str(ip_str) {
            Ok(ip) => ip,
            Err(_) => {
                // Distinguish between "looks like an IP attempt" and "completely unrelated"
                if validator::looks_like_ip(ip_str) {
                    return Err(ParseError::InvalidIp(ip_str.to_string()));
                } else {
                    return Err(ParseError::MalformedLine(line.to_string()));
                }
            }
        };

        // Remaining tokens are domains
        if tokens.len() < 2 {
            return Err(ParseError::MalformedLine(line.to_string()));
        }

        let mut domains = Vec::new();
        for domain in &tokens[1..] {
            if !validator::is_valid_domain(domain) {
                return Err(ParseError::InvalidDomain(domain.to_string()));
            }
            domains.push(domain.to_string());
        }

        let mut rule = HostRule::new(ip, domains);
        if let Some(c) = _comment {
            if !c.is_empty() {
                rule.comment = Some(c.to_string());
            }
        }
        Ok(Some(rule))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    // Helper: build expected rule without caring about generated RuleId
    fn make_rule(ip: &str, domains: Vec<&str>) -> HostRule {
        HostRule::new(
            ip.parse::<IpAddr>().unwrap(),
            domains.into_iter().map(|s| s.to_string()).collect(),
        )
    }

    fn assert_rules_eq(actual: &[HostRule], expected: &[HostRule]) {
        assert_eq!(actual.len(), expected.len(), "rule count mismatch");
        for (a, e) in actual.iter().zip(expected.iter()) {
            assert_eq!(a.ip, e.ip, "IP mismatch");
            assert_eq!(a.domains, e.domains, "domains mismatch");
            assert_eq!(a.enabled, e.enabled, "enabled mismatch");
            assert_eq!(a.comment, e.comment, "comment mismatch");
            assert_eq!(a.source, e.source, "source mismatch");
        }
    }

    #[test]
    fn test_parse_standard_line() {
        let cases = vec![
            (
                "ipv4_single",
                "127.0.0.1 example.com",
                vec![make_rule("127.0.0.1", vec!["example.com"])],
            ),
            (
                "ipv6_single",
                "::1 localhost",
                vec![make_rule("::1", vec!["localhost"])],
            ),
            (
                "ipv6_full",
                "2001:db8::1 example.com",
                vec![make_rule("2001:db8::1", vec!["example.com"])],
            ),
            (
                "multi_domain",
                "127.0.0.1 a.com b.com",
                vec![make_rule("127.0.0.1", vec!["a.com", "b.com"])],
            ),
            (
                "with_comment",
                "127.0.0.1 example.com # dev",
                vec![{
                    let mut r = make_rule("127.0.0.1", vec!["example.com"]);
                    r.comment = Some("dev".to_string());
                    r
                }],
            ),
        ];

        for (name, input, expected) in cases {
            let result = Parser::parse(input);
            assert!(
                result.errors.is_empty(),
                "case: {} — unexpected errors: {:?}",
                name,
                result.errors
            );
            assert_rules_eq(&result.rules, &expected);
        }
    }

    #[test]
    fn test_parse_errors() {
        let cases = vec![
            (
                "invalid_ip",
                "999.999.999.999 x.com",
                ParseError::InvalidIp("999.999.999.999".to_string()),
            ),
            (
                "invalid_domain",
                "127.0.0.1 -bad.com",
                ParseError::InvalidDomain("-bad.com".to_string()),
            ),
            (
                "malformed",
                "example.com 127.0.0.1",
                ParseError::MalformedLine("example.com 127.0.0.1".to_string()),
            ),
        ];

        for (name, input, expected_err) in cases {
            let result = Parser::parse(input);
            assert!(
                result.rules.is_empty(),
                "case: {} — expected no rules, got {:?}",
                name,
                result.rules
            );
            assert_eq!(
                result.errors.len(),
                1,
                "case: {} — expected exactly one error",
                name
            );
            assert_eq!(result.errors[0], expected_err, "case: {}", name);
        }
    }

    #[test]
    fn test_parse_comment_and_empty() {
        let cases = vec![
            ("comment", "# this is a comment", 0),
            ("empty", "", 0),
            ("whitespace", "   ", 0),
        ];

        for (name, input, expected_rules) in cases {
            let result = Parser::parse(input);
            assert_eq!(
                result.rules.len(),
                expected_rules,
                "case: {} — expected {} rules",
                name,
                expected_rules
            );
            assert!(
                result.errors.is_empty(),
                "case: {} — unexpected errors: {:?}",
                name,
                result.errors
            );
        }
    }

    #[test]
    fn test_extract_managed_block() {
        let input =
            "# line 1\n# ---- mHost start ----\n127.0.0.1 x.com\n# ---- mHost end ----\n# line 5";
        assert_eq!(Parser::extract_managed_block(input), Some((1, 3)));
    }

    #[test]
    fn test_extract_managed_block_missing() {
        assert_eq!(
            Parser::extract_managed_block("# no markers\n127.0.0.1 a.com"),
            None
        );
        assert_eq!(
            Parser::extract_managed_block("# ---- mHost start ----\n127.0.0.1 a.com"),
            None
        );
        assert_eq!(
            Parser::extract_managed_block("127.0.0.1 a.com\n# ---- mHost end ----"),
            None
        );
    }

    #[test]
    fn test_extract_managed_block_reversed() {
        let input = "# ---- mHost end ----\n127.0.0.1 a.com\n# ---- mHost start ----";
        assert_eq!(Parser::extract_managed_block(input), None);
    }

    #[test]
    fn test_extract_managed_block_multiple_starts() {
        let input = "# ---- mHost start ----\n127.0.0.1 a.com\n# ---- mHost start ----\n127.0.0.1 b.com\n# ---- mHost end ----";
        assert_eq!(Parser::extract_managed_block(input), None);
    }

    #[test]
    fn test_extract_managed_block_multiple_ends() {
        let input = "# ---- mHost start ----\n127.0.0.1 a.com\n# ---- mHost end ----\n127.0.0.1 b.com\n# ---- mHost end ----";
        assert_eq!(Parser::extract_managed_block(input), None);
    }

    #[test]
    fn test_extract_managed_block_multiple_blocks() {
        let input = "# ---- mHost start ----\n127.0.0.1 a.com\n# ---- mHost end ----\n# ---- mHost start ----\n127.0.0.1 b.com\n# ---- mHost end ----";
        assert_eq!(Parser::extract_managed_block(input), None);
    }

    #[test]
    fn test_format_roundtrip() {
        let input = "127.0.0.1 example.com\n::1 localhost\n";
        let result = Parser::parse(input);
        let formatted = Parser::format(&result.rules);
        let reparsed = Parser::parse(&formatted);
        assert_rules_eq(&result.rules, &reparsed.rules);
    }

    #[test]
    fn test_format_multi_domain_expansion() {
        let rules = vec![make_rule("127.0.0.1", vec!["a.com", "b.com"])];
        let formatted = Parser::format(&rules);
        let expected = "127.0.0.1 a.com\n127.0.0.1 b.com\n";
        assert_eq!(formatted, expected);
    }

    #[test]
    fn test_format_with_comment() {
        let mut rule = make_rule("127.0.0.1", vec!["example.com"]);
        rule.comment = Some("dev".to_string());
        let formatted = Parser::format(&[rule]);
        assert_eq!(formatted, "127.0.0.1 example.com # dev\n");
    }

    #[test]
    fn test_parse_inline_comment_only() {
        let input = "127.0.0.1 example.com #";
        let result = Parser::parse(input);
        assert!(result.errors.is_empty());
        assert_eq!(result.rules.len(), 1);
        assert_eq!(result.rules[0].ip, "127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(result.rules[0].domains, vec!["example.com"]);
        // empty comment after # is stored as None because we trim and check empty
        assert_eq!(result.rules[0].comment, None);
    }

    #[test]
    fn test_parse_leading_trailing_whitespace() {
        let input = "  127.0.0.1   example.com  ";
        let result = Parser::parse(input);
        assert!(result.errors.is_empty());
        assert_eq!(result.rules.len(), 1);
        assert_eq!(result.rules[0].domains, vec!["example.com"]);
    }

    #[test]
    fn test_parse_multiple_lines() {
        let input = "# header\n127.0.0.1 a.com\n\n::1 localhost\n# footer\n";
        let result = Parser::parse(input);
        assert!(result.errors.is_empty());
        assert_eq!(result.rules.len(), 2);
        assert_eq!(result.rules[0].ip, "127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(result.rules[0].domains, vec!["a.com"]);
        assert_eq!(result.rules[1].ip, "::1".parse::<IpAddr>().unwrap());
        assert_eq!(result.rules[1].domains, vec!["localhost"]);
    }

    #[test]
    fn test_parse_invalid_ip_returns_invalid_ip_not_malformed() {
        // IP-like but invalid address should produce InvalidIp, not MalformedLine
        let result = Parser::parse("999.999.999.999 x.com");
        assert_eq!(result.errors.len(), 1);
        assert_eq!(
            result.errors[0],
            ParseError::InvalidIp("999.999.999.999".to_string())
        );
    }

    #[test]
    fn test_parse_default_enabled_and_source() {
        let input = "127.0.0.1 example.com\n::1 localhost # dev\n";
        let result = Parser::parse(input);
        assert!(result.errors.is_empty());
        assert_eq!(result.rules.len(), 2);

        // Default enabled should be true
        assert!(
            result.rules[0].enabled,
            "rule 0 should be enabled by default"
        );
        assert!(
            result.rules[1].enabled,
            "rule 1 should be enabled by default"
        );

        // Default source should be Manual
        assert_eq!(
            result.rules[0].source,
            mhost_core::RuleSource::Manual,
            "rule 0 source should be Manual by default"
        );
        assert_eq!(
            result.rules[1].source,
            mhost_core::RuleSource::Manual,
            "rule 1 source should be Manual by default"
        );

        // Format should not include enabled/source (they are metadata, not hosts syntax)
        let formatted = Parser::format(&result.rules);
        assert!(!formatted.contains("enabled"));
        assert!(!formatted.contains("Manual"));
        assert!(!formatted.contains("Remote"));
        assert!(!formatted.contains("AdBlock"));
    }
}
