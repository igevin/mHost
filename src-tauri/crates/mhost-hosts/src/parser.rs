//! Hosts file parser

use crate::validator;
use mhost_core::{HostRule, ParseError};
use serde::{Deserialize, Serialize};
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

/// A parse error annotated with its 1-based line number
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseErrorAtLine {
    pub line_number: usize,
    pub error: ParseError,
}

/// Validation result suitable for frontend consumption (serializable)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateResult {
    pub rules: Vec<HostRule>,
    pub errors: Vec<ParseErrorAtLine>,
    pub duplicates: Vec<mhost_core::DuplicateRule>,
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

    /// Parse hosts text into a serializable validation result with line numbers.
    /// Each error is annotated with its 1-based line number in the input text.
    pub fn parse_with_lines(text: &str) -> ValidateResult {
        let mut rules = Vec::new();
        let mut errors = Vec::new();

        for (idx, line) in text.lines().enumerate() {
            match Self::parse_line(line) {
                Ok(Some(mut rule)) => {
                    rule.line_number = Some(idx + 1);
                    rules.push(rule);
                }
                Ok(None) => {}
                Err(error) => errors.push(ParseErrorAtLine {
                    line_number: idx + 1,
                    error,
                }),
            }
        }

        let duplicates = validator::check_duplicates(&rules);
        ValidateResult { rules, errors, duplicates }
    }

    /// Parse hosts text and collect only errors with their line numbers.
    pub fn parse_errors_only(text: &str) -> Vec<ParseErrorAtLine> {
        let mut errors = Vec::new();

        for (idx, line) in text.lines().enumerate() {
            if let Err(error) = Self::parse_line(line) {
                errors.push(ParseErrorAtLine {
                    line_number: idx + 1,
                    error,
                });
            }
        }

        errors
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

    /// Extract the content between managed block markers (exclusive of markers).
    /// Returns `Some(content)` if a valid managed block exists, `None` otherwise.
    /// An empty block (start immediately followed by end) returns `Some("")`.
    pub fn extract_managed_block_content(input: &str) -> Option<String> {
        let (start, end) = Self::extract_managed_block(input)?;
        let lines: Vec<&str> = input.lines().collect();
        if end <= start + 1 {
            return Some(String::new());
        }
        let content_lines = &lines[start + 1..end];
        Some(content_lines.join("\n"))
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn parse_line(line: &str) -> Result<Option<HostRule>, ParseError> {
        let trimmed = line.trim();

        // Empty line -> no rule
        if trimmed.is_empty() {
            return Ok(None);
        }

        // Standalone comment line -> comment-only rule
        if trimmed.starts_with('#') {
            return Ok(Some(HostRule::comment_only(trimmed)));
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
            ("comment", "# this is a comment", 1),
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

        // Verify the comment rule has correct content
        let comment_result = Parser::parse("# this is a comment");
        assert!(comment_result.rules[0].is_comment_only());
        assert_eq!(comment_result.rules[0].comment, Some("# this is a comment".to_string()));
        assert_eq!(comment_result.rules[0].ip, None);
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
    fn test_format_roundtrip_with_comments() {
        let input = "# header comment\n127.0.0.1 example.com\n# footer comment\n";
        let result = Parser::parse(input);
        assert_eq!(result.rules.len(), 3); // 2 comment-only + 1 host rule
        let formatted = Parser::format(&result.rules);
        assert_eq!(formatted, "# header comment\n127.0.0.1 example.com\n# footer comment\n");
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
        assert_eq!(result.rules[0].ip, Some("127.0.0.1".parse::<IpAddr>().unwrap()));
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
        // 2 comment-only rules + 2 host rules = 4
        assert_eq!(result.rules.len(), 4);
        assert!(result.rules[0].is_comment_only());
        assert_eq!(result.rules[0].comment, Some("# header".to_string()));
        assert_eq!(result.rules[1].ip, Some("127.0.0.1".parse::<IpAddr>().unwrap()));
        assert_eq!(result.rules[1].domains, vec!["a.com"]);
        assert_eq!(result.rules[2].ip, Some("::1".parse::<IpAddr>().unwrap()));
        assert_eq!(result.rules[2].domains, vec!["localhost"]);
        assert!(result.rules[3].is_comment_only());
        assert_eq!(result.rules[3].comment, Some("# footer".to_string()));
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

    // -------------------------------------------------------------------
    // extract_managed_block_content tests
    // -------------------------------------------------------------------

    #[test]
    fn test_extract_managed_block_content_with_block() {
        let input = "# some comment\n# ---- mHost start ----\n127.0.0.1 example.com\n# ---- mHost end ----\n# tail";
        let result = Parser::extract_managed_block_content(input);
        assert_eq!(result, Some("127.0.0.1 example.com".to_string()));
    }

    #[test]
    fn test_extract_managed_block_content_without_block() {
        let input = "# no block here\n127.0.0.1 example.com";
        let result = Parser::extract_managed_block_content(input);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_managed_block_content_empty_block() {
        let input = "# ---- mHost start ----\n# ---- mHost end ----";
        let result = Parser::extract_managed_block_content(input);
        assert_eq!(result, Some(String::new()));
    }

    #[test]
    fn test_extract_managed_block_content_multi_line() {
        let input = "# ---- mHost start ----\n127.0.0.1 a.com\n192.168.1.1 b.com\n# ---- mHost end ----";
        let result = Parser::extract_managed_block_content(input);
        assert_eq!(result, Some("127.0.0.1 a.com\n192.168.1.1 b.com".to_string()));
    }

    #[test]
    fn test_extract_managed_block_content_with_surrounding_content() {
        let input = "127.0.0.1 pre-existing.com\n# ---- mHost start ----\n::1 managed.local\n# ---- mHost end ----\n192.168.1.1 post-existing.com";
        let result = Parser::extract_managed_block_content(input);
        assert_eq!(result, Some("::1 managed.local".to_string()));
    }

    // -----------------------------------------------------------------------
    // parse_with_lines tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_with_lines_valid() {
        let cases = vec![
            ("single_ipv4", "127.0.0.1 example.com",
             vec![("127.0.0.1", vec!["example.com"])], 0),
            ("ipv6", "::1 localhost",
             vec![("::1", vec!["localhost"])], 0),
            ("multi_domain", "127.0.0.1 a.com b.com",
             vec![("127.0.0.1", vec!["a.com", "b.com"])], 0),
            ("with_comment", "127.0.0.1 x.com # dev",
             vec![("127.0.0.1", vec!["x.com"])], 0),
            ("empty_lines", "\n\n127.0.0.1 x.com\n\n",
             vec![("127.0.0.1", vec!["x.com"])], 0),
        ];
        for (name, input, expected_rules, expected_errors) in cases {
            let result = Parser::parse_with_lines(input);
            assert_eq!(result.rules.len(), expected_rules.len(), "case: {}", name);
            for (i, (expected_ip, expected_domains)) in expected_rules.iter().enumerate() {
                assert_eq!(result.rules[i].ip, Some(expected_ip.parse::<IpAddr>().unwrap()), "case: {} rule {} ip", name, i);
                assert_eq!(result.rules[i].domains, *expected_domains, "case: {} rule {} domains", name, i);
            }
            assert_eq!(result.errors.len(), expected_errors, "case: {}", name);
        }

        // Comment-only case: produces 1 comment-only rule
        let comment_result = Parser::parse_with_lines("# this is a comment");
        assert_eq!(comment_result.rules.len(), 1);
        assert!(comment_result.rules[0].is_comment_only());
        assert_eq!(comment_result.rules[0].comment, Some("# this is a comment".to_string()));
        assert_eq!(comment_result.errors.len(), 0);
    }

    #[test]
    fn test_parse_with_lines_invalid() {
        let cases = vec![
            ("bad_ip", "999.999.999.999 x.com", "invalid ip"),
            ("bad_domain", "127.0.0.1 -bad", "invalid domain"),
            ("no_ip", "example.com 127.0.0.1", "malformed"),
        ];
        for (name, input, expected_msg_contains) in cases {
            let result = Parser::parse_with_lines(input);
            assert!(!result.errors.is_empty(), "case: {} should have errors", name);
            let msg = result.errors[0].error.to_string().to_lowercase();
            assert!(msg.contains(expected_msg_contains),
                "case: {} error '{}' should contain '{}'", name, msg, expected_msg_contains);
        }
    }

    #[test]
    fn test_parse_with_lines_error_line_numbers() {
        let result = Parser::parse_with_lines("127.0.0.1 valid.com\n999.999.999.999 bad.com");
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].line_number, 2);
    }

    #[test]
    fn test_parse_with_lines_multiple_errors() {
        let result = Parser::parse_with_lines(
            "127.0.0.1 ok.com\nbad_line\n127.0.0.1 ok2.com\n999.999.999.999 bad.com"
        );
        assert_eq!(result.errors.len(), 2);
        assert_eq!(result.errors[0].line_number, 2);
        assert_eq!(result.errors[1].line_number, 4);
    }

    #[test]
    fn test_validate_result_serialization() {
        let result = Parser::parse_with_lines("127.0.0.1 example.com");
        let json = serde_json::to_string(&result).unwrap();
        let parsed: ValidateResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.rules.len(), result.rules.len());
    }

    // -----------------------------------------------------------------------
    // Duplicate detection tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_no_duplicates_for_unique_domains() {
        let input = "127.0.0.1 a.com\n127.0.0.1 b.com";
        let result = Parser::parse_with_lines(input);
        assert!(result.duplicates.is_empty());
    }

    #[test]
    fn test_same_ip_duplicates_detected() {
        let input = "127.0.0.1 a.com\n127.0.0.1 a.com";
        let result = Parser::parse_with_lines(input);
        assert_eq!(result.duplicates.len(), 1);
        assert_eq!(result.duplicates[0].domain, "a.com");
        assert_eq!(result.duplicates[0].lines, vec![1, 2]);
        assert!(matches!(result.duplicates[0].kind, mhost_core::DuplicateKind::SameIp));
    }

    #[test]
    fn test_different_ip_duplicates_detected() {
        let input = "127.0.0.1 a.com\n192.168.1.1 a.com";
        let result = Parser::parse_with_lines(input);
        assert_eq!(result.duplicates.len(), 1);
        assert!(matches!(result.duplicates[0].kind, mhost_core::DuplicateKind::DifferentIp));
    }

    #[test]
    fn test_disabled_rules_not_checked() {
        let input = "127.0.0.1 a.com\n# 127.0.0.1 a.com";
        let result = Parser::parse_with_lines(input);
        assert!(result.duplicates.is_empty());
    }

    #[test]
    fn test_comment_only_lines_not_checked() {
        let input = "127.0.0.1 a.com\n# just a comment";
        let result = Parser::parse_with_lines(input);
        assert!(result.duplicates.is_empty());
    }

    #[test]
    fn test_duplicate_across_multiple_rules() {
        let input = "127.0.0.1 a.com b.com\n127.0.0.1 a.com";
        let result = Parser::parse_with_lines(input);
        assert_eq!(result.duplicates.len(), 1);
        assert_eq!(result.duplicates[0].domain, "a.com");
    }
}
