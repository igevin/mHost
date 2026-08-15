//! Shared suffix-walking helper used by both [`crate::resolver::RuleEngine`]
//! (issue #79) and [`crate::adblock::AdBlockEngine`] (issue #130).
//!
//! Behaviour:
//!
//! ```text
//! walk_parents("a.b.c.example.com", predicate)
//!   checks "a.b.c.example.com" → "b.c.example.com" → "c.example.com"
//!                          → "example.com" → "com" → stops (no dot left)
//! ```
//!
//! First match wins. **Single-label parents are visited once** — so a
//! caller that registers `"com"` will match every `.com` query (Pi-hole
//! semantic). The walk only terminates when the current label has no
//! `.` in it AND predicate did not yield a value.

/// Walk parent domains of `domain`, applying `predicate` to each candidate
/// (including `domain` itself). Returns the first value `predicate` yields
/// via `Some`, or `None` if the walk exhausts without a hit.
///
/// `domain` is treated as already lowercased / canonicalised by the caller.
pub(crate) fn walk_parents<T, F>(domain: &str, predicate: F) -> Option<T>
where
    F: Fn(&str) -> Option<T>,
{
    let mut current = domain;
    loop {
        if let Some(v) = predicate(current) {
            return Some(v);
        }
        let pos = current.find('.')?;
        current = &current[pos + 1..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_parents_first_match_wins() {
        // "hits" exact domain first, then parents
        let r = walk_parents("a.b.example.com", |d| match d {
            "a.b.example.com" => Some(1),
            "b.example.com" => Some(2),
            "example.com" => Some(3),
            _ => None,
        });
        assert_eq!(r, Some(1));
    }

    #[test]
    fn walk_parents_falls_through_to_parent() {
        let r = walk_parents("a.b.example.com", |d| match d {
            "example.com" => Some("hit"),
            _ => None,
        });
        assert_eq!(r, Some("hit"));
    }

    #[test]
    fn walk_parents_visits_single_label_parent_once() {
        // Pi-hole semantic: registering "com" should block every *.com.
        // Walk visits "example.com" once, then "com" once, then stops.
        let r = walk_parents("example.com", |d| match d {
            "com" => Some("hit"),
            _ => None,
        });
        assert_eq!(r, Some("hit"));
    }

    #[test]
    fn walk_parents_no_hit() {
        let r = walk_parents("a.b.example.com", |_| None::<()>);
        assert_eq!(r, None);
    }

    /// **PR #154 review (P3)**: Pi-hole-style TLD blocking semantics.
    /// Registering `"com"` in the engine should match every query ending
    /// in `.com` (single-label ancestor walk is intentional). Verifies
    /// the full `walk_parents` chain, not just the single-step case.
    #[test]
    fn walk_parents_registered_tld_matches_every_subdomain() {
        let r = walk_parents("deeply.nested.subdomain.example.com", |d| match d {
            "com" => Some("TLD-hit"),
            _ => None,
        });
        assert_eq!(r, Some("TLD-hit"));

        // And also the trivial single-label form.
        let r = walk_parents("example.com", |d| match d {
            "com" => Some("TLD-hit"),
            _ => None,
        });
        assert_eq!(r, Some("TLD-hit"));

        // But a query NOT under .com should miss (proves the hit
        // wasn't a false positive from walk mechanics).
        let r = walk_parents("example.org", |d| match d {
            "com" => Some("TLD-hit"),
            _ => None,
        });
        assert_eq!(r, None);
    }
}
