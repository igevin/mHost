//! Hosts syntax validator

/// Validates a domain string according to the following rules:
/// - must not be empty
/// - only letters, digits, hyphens, and dots are allowed
/// - each label must not start or end with '-'
/// - each label must not be all digits (to avoid accepting IP-like tokens as domains)
pub fn is_valid_domain(domain: &str) -> bool {
    if domain.is_empty() {
        return false;
    }
    if domain.starts_with('-') || domain.ends_with('-') || domain.ends_with('.') {
        return false;
    }
    if !domain
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        return false;
    }
    for label in domain.split('.') {
        if label.is_empty() {
            return false;
        }
        if label.starts_with('-') || label.ends_with('-') {
            return false;
        }
        if label.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }
    true
}

/// Heuristic check: does the token look like an IP address attempt?
///
/// IPv4-like: only digits and dots, contains at least one dot, at least one digit,
///            and does not start or end with a dot.
/// IPv6-like: contains ':' (hex digits are optional since :: is valid).
///            Must not contain URL-like patterns (e.g., "http://", "example.com:8080").
pub fn looks_like_ip(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    if token.contains(':') {
        // Reject URL schemes like "http://", "https://"
        if token.contains("//") {
            return false;
        }
        // Reject if it looks like a URL with port (e.g., example.com:8080)
        // Valid IPv6 addresses contain only hex digits, colons, and optionally dots (IPv4-mapped)
        // A simple heuristic: if the token before the first colon contains a dot, it's likely a URL
        let before_first_colon = token.split(':').next().unwrap_or("");
        if before_first_colon.contains('.') {
            return false;
        }
        return true;
    }
    token.chars().all(|c| c.is_ascii_digit() || c == '.')
        && token.contains('.')
        && token.chars().any(|c| c.is_ascii_digit())
        && !token.starts_with('.')
        && !token.ends_with('.')
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_domain() {
        let cases = vec![
            ("simple", "example.com", true),
            ("subdomain", "sub.example.com", true),
            ("hyphen", "my-site.com", true),
            ("localhost", "localhost", true),
            ("starts_with_hyphen", "-bad.com", false),
            ("ends_with_hyphen_label", "bad-.com", false),
            ("empty", "", false),
            ("invalid_char", "bad@com", false),
            ("underscore", "bad_com", false),
            ("ends_with_dot", "example.com.", false),
            ("all_digits_label", "123.456", false),
            ("all_digits", "12345", false),
            ("mixed_label", "123.example.com", false),
        ];

        for (name, domain, expected) in cases {
            assert_eq!(
                is_valid_domain(domain),
                expected,
                "case: {} — domain: {}",
                name,
                domain
            );
        }
    }

    #[test]
    fn test_looks_like_ip() {
        let cases = vec![
            ("ipv4", "127.0.0.1", true),
            ("invalid_ipv4", "999.999.999.999", true),
            ("ipv6", "::1", true),
            ("ipv6_full", "2001:db8::1", true),
            ("ipv6_mapped", "::ffff:192.0.2.1", true),
            ("domain", "example.com", false),
            ("text", "abc", false),
            ("empty", "", false),
            ("dot_only", ".", false),
            ("trailing_dot", "1.", false),
            ("double_colon", "::", true),
            ("hex_colon", "abc:def", true),
            // IPv6 enhancement: reject URL-like tokens with port
            ("url_with_port", "example.com:8080", false),
            ("url_with_scheme", "http://example.com", false),
            ("ipv6_loopback", "::1", true),
            ("ipv6_full_expanded", "2001:0db8:85a3:0000:0000:8a2e:0370:7334", true),
        ];

        for (name, token, expected) in cases {
            assert_eq!(
                looks_like_ip(token),
                expected,
                "case: {} — token: {}",
                name,
                token
            );
        }
    }
}
