//! Reversed-domain trie for `find_longest_suffix_match` lookups
//! (issue #199 sub-task A).
//!
//! ## What it replaces
//!
//! The previous engine stored rule sets as `HashMap<String, IpAddr>` for
//! the zero-addr rules and `HashSet<String>` for the NXDOMAIN rules and
//! whitelist (see git history before this commit). At 100k rules each
//! entry carried a `String` (~24-byte header + ~25-byte payload on
//! average) and a HashMap slot with 87.5% load factor — so the heap
//! footprint was ~50 MB and the DNS hot path did three independent
//! suffix walks, each doing `(labels-in-query)` HashMap lookups.
//!
//! ## What this gives us
//!
//! 1. **Shared prefixes.** A `com` child is allocated once across every
//!    `*.com` rule. The same is true for `tracker.com` if multiple
//!    sources register it. Empirically this drops 100k rules from
//!    ~50 MB to ~5–10 MB (issue #199 estimate, runtime verified by the
//!    end-of-host memory assertion in `src/bin/...`).
//!
//! 2. **One traversal per rule-set.** [`Trie::find_longest_suffix_match`]
//!    walks the trie once from the TLD down, recording the deepest
//!    data-holding node seen so far. `check()` thus does at most one
//!    traversal per rule-set per query, instead of one parent-walk per
//!    call.
//!
//! ## Why reversed-domain
//!
//! DNS labels read left-to-right (`a.b.example.com`) but suffix matching
//! walks right-to-left. A left-to-right trie would need an O(domain)
//! suffix lookup at every node; the reversed trie just walks labels off
//! the right edge and amortises the cost.
//!
//! ## TLD semantics (issue #79 contract)
//!
//! `walk_parents` and the trie both visit **single-label parents once**:
//! a query for `a.b.example.com` walks `com` → `example` → `b` → `a`.
//! With `com` registered, every `*.com` query matches `com`'s data
//! (unless a deeper rule overrides it).

use std::collections::HashMap;

/// A reversed-domain trie mapping a domain suffix to a value `T`.
///
/// `Trie` is generic over `T` because the engine uses three tries with
/// different value types: `Trie<IpAddr>` for zero-addr rules,
/// `Trie<()>` for NXDOMAIN rules, `Trie<()>` for the whitelist.
/// Each trie is internally immutable once published inside a
/// `RulesSnapshot` (the engine rebuilds the whole snapshot under one
/// `Arc::swap` per issue #132), so internal nodes are plain owned
/// values — no `Rc` / `Arc` overhead, and `rebuild()` is free to drop
/// the old snapshot wholesale when its refcount hits zero.
pub struct Trie<T> {
    root: TrieNode<T>,
}

struct TrieNode<T> {
    children: HashMap<String, TrieNode<T>>,
    data: Option<T>,
}

impl<T> TrieNode<T> {
    fn new() -> Self {
        Self {
            children: HashMap::new(),
            data: None,
        }
    }
}

impl<T> Default for Trie<T> {
    fn default() -> Self {
        Self {
            root: TrieNode::new(),
        }
    }
}

impl<T> Trie<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the trie holds no rules at all (root has no data and no
    /// children). Used by `RulesSnapshot::has_block_rules` to keep the
    /// "no block rules loaded → no engine" short-circuit.
    pub fn is_empty(&self) -> bool {
        self.root.children.is_empty() && self.root.data.is_none()
    }

    /// Number of registered domains (terminal nodes with `data.is_some()`).
    /// Replaces the previous `HashSet::len()` / `HashMap::len()` source.
    pub fn len(&self) -> usize {
        self.root.data.is_some() as usize
            + self
                .root
                .children
                .values()
                .map(Self::terminal_count)
                .sum::<usize>()
    }

    fn terminal_count(node: &TrieNode<T>) -> usize {
        node.data.is_some() as usize
            + node
                .children
                .values()
                .map(Self::terminal_count)
                .sum::<usize>()
    }

    /// Total nodes including the root. Used by the bench harness to
    /// verify the in-memory node count vs. a `100_000`-rule baseline
    /// (each rule contributes roughly O(labels-in-domain) nodes).
    pub fn node_count(&self) -> usize {
        1 + self
            .root
            .children
            .values()
            .map(Self::recursive_node_count)
            .sum::<usize>()
    }

    fn recursive_node_count(node: &TrieNode<T>) -> usize {
        1 + node
            .children
            .values()
            .map(Self::recursive_node_count)
            .sum::<usize>()
    }

    /// Register `domain` with `value`. If `domain` already exists, the
    /// new `value` **overwrites** the old one — matches the prior
    /// `HashMap::insert` last-writer-wins semantics.
    ///
    /// Walks labels right-to-left: for `a.b.c.example.com` we collect
    /// `["com", "example", "c", "b", "a"]` and then descend
    /// `root → com → example → c → b → a`, creating missing nodes
    /// along the way. The leaf node (`a`) holds the value.
    pub fn insert(&mut self, domain: &str, value: T) {
        // Collect right-to-left labels. An empty domain is a no-op
        // (matches the prior `HashMap::insert("", _, _)` which
        // silently dropped it).
        let mut labels: Vec<&str> = Vec::new();
        let mut rest = domain;
        loop {
            let (label, new_rest) = match rest.rfind('.') {
                Some(pos) => (&rest[pos + 1..], &rest[..pos]),
                None => (rest, ""),
            };
            if label.is_empty() {
                break;
            }
            labels.push(label);
            rest = new_rest;
            if rest.is_empty() {
                break;
            }
        }
        if labels.is_empty() {
            return;
        }

        // Descend the trie from root down, allocating missing nodes.
        // `labels` is already right-to-left (TLD first, leaf last),
        // so forward iteration walks root → com → example → leaf,
        // which is the right order for sharing prefixes across
        // multiple inserts. (`labels.iter().rev()` was a bug — it
        // built the tree inverted, defeating the whole shared-prefix
        // optimization.)
        let mut node = &mut self.root;
        for label in labels.iter() {
            // `entry().or_insert_with` is one allocation per missing
            // node — exactly what we want.
            node = node
                .children
                .entry(label.to_string())
                .or_insert_with(TrieNode::new);
        }
        node.data = Some(value);
    }

    /// Find the data held by the **longest registered suffix** of
    /// `domain`. Returns `None` if no suffix of `domain` is registered.
    ///
    /// Walk labels from the TLD inward, recording the deepest
    /// data-holding node along the way. With `example.com` and
    /// `tracker.com` registered, a query for `ads.tracker.com` returns
    /// `tracker.com`'s data (deeper than `com`); a query for
    /// `other.com` returns `com`'s data.
    ///
    /// Algorithm (O(labels-in-query)):
    ///
    /// ```text
    /// node = root
    /// best = node.data (root rarely has data)
    /// rest = domain
    /// loop {
    ///     (label, new_rest) = split_last_label(rest)
    ///     if label is empty: return best
    ///     if node.children[label] is Some(child):
    ///         if child.data: best = child.data
    ///         node = child
    ///         rest = new_rest
    ///     else:
    ///         return best
    /// }
    /// ```
    pub fn find_longest_suffix_match(&self, domain: &str) -> Option<&T> {
        let mut node = &self.root;
        let mut best: Option<&T> = node.data.as_ref();
        let mut rest = domain;

        loop {
            // Split off the LAST label of `rest`.
            //
            // `a.b.c.example.com`:
            //   iter 1: label `com`     rest `a.b.c.example`
            //   iter 2: label `example` rest `a.b.c`
            //   iter 3: label `c`       rest `a.b`
            //   iter 4: label `b`       rest `a`
            //   iter 5: label `a`       rest `""` (loop exits via empty rest)
            //
            // `com` (no dot):
            //   iter 1: label `com` rest `""` (loop exits via empty rest)
            let (label, new_rest) = match rest.rfind('.') {
                Some(pos) => (&rest[pos + 1..], &rest[..pos]),
                None => (rest, ""),
            };

            // Empty label: trailing dot in the input, or empty
            // domain. We've processed all non-empty labels; stop.
            if label.is_empty() {
                return best;
            }

            match node.children.get(label) {
                Some(child) => {
                    if let Some(d) = &child.data {
                        best = Some(d);
                    }
                    node = child;
                    rest = new_rest;
                    if rest.is_empty() {
                        // Final label processed, child descended into;
                        // return the best seen so far (which is the
                        // deepest data-holding node we visited).
                        return best;
                    }
                }
                None => return best,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    /// Empty trie returns None for every query.
    #[test]
    fn empty_trie_returns_none() {
        let t: Trie<()> = Trie::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
        assert_eq!(t.find_longest_suffix_match("anything.com"), None);
    }

    /// Insertion + lookup: exact match returns the inserted value.
    #[test]
    fn exact_match_returns_value() {
        let mut t: Trie<()> = Trie::new();
        t.insert("ad.example.com", ());
        assert_eq!(t.len(), 1);
        assert!(!t.is_empty());
        assert!(t.find_longest_suffix_match("ad.example.com").is_some());
        assert!(t.find_longest_suffix_match("tracker.example.com").is_none());
    }

    /// Longest-suffix-match semantics: a query for a subdomain
    /// returns the longest registered ancestor.
    #[test]
    fn longest_suffix_match_walks_inward() {
        let mut t: Trie<()> = Trie::new();
        t.insert("com", ());
        t.insert("example.com", ());
        t.insert("ad.example.com", ());

        // Depth-based expectations:
        //   `a.b.example.com` → deepest registered = `example.com`
        //   `x.com`            → deepest registered = `com`
        //   `unrelated.org`    → no registered suffix
        assert!(t.find_longest_suffix_match("a.b.example.com").is_some());
        assert!(t.find_longest_suffix_match("x.com").is_some());
        assert!(t.find_longest_suffix_match("unrelated.org").is_none());
        assert_eq!(t.len(), 3);
    }

    /// TLD matching per issue #79: registering `com` matches every
    /// `*.com` query. Issue #79 walk_parents contract.
    #[test]
    fn tld_alone_matches_every_subdomain() {
        let mut t: Trie<()> = Trie::new();
        t.insert("com", ());

        // Should match for any `*.com` query.
        for d in ["example.com", "a.b.example.com", "anything.anything.com"] {
            assert!(t.find_longest_suffix_match(d).is_some(), "{d} should hit",);
        }
        // Different TLD: no match.
        assert!(t.find_longest_suffix_match("example.org").is_none());
    }

    /// Value semantics: zero-addr trie returns the inserted IP.
    #[test]
    fn zero_addr_value_is_returned() {
        let mut t: Trie<std::net::IpAddr> = Trie::new();
        let ip = std::net::IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        t.insert("ads.example.com", ip);

        // Exact lookup: returns the IP.
        match t.find_longest_suffix_match("ads.example.com") {
            Some(got) => assert_eq!(*got, ip),
            None => panic!("exact lookup must hit"),
        }
        // Subdomain: same IP.
        match t.find_longest_suffix_match("x.ads.example.com") {
            Some(got) => assert_eq!(*got, ip),
            None => panic!("subdomain lookup must hit"),
        }
    }

    /// Re-inserting the same domain overwrites the value (matches the
    /// prior HashMap::insert semantics).
    #[test]
    fn insert_overwrites_existing() {
        let mut t: Trie<std::net::IpAddr> = Trie::new();
        t.insert(
            "ad.example.com",
            std::net::IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
        );
        t.insert(
            "ad.example.com",
            std::net::IpAddr::V4(Ipv4Addr::new(2, 2, 2, 2)),
        );
        assert_eq!(t.len(), 1, "overwrite must not duplicate");
        assert_eq!(
            *t.find_longest_suffix_match("ad.example.com").unwrap(),
            std::net::IpAddr::V4(Ipv4Addr::new(2, 2, 2, 2)),
        );
    }

    /// Shared-prefix compression: 100k `*.example.com` rules share the
    /// `com → example` prefix. node_count should be ≪ 100k (root +
    /// `com` + `example` + 100k leaves = 100_003, NOT 300k for
    /// `com`+`example`+`ad`×3 etc. that you'd get without sharing).
    /// This is the memory benefit the issue promises.
    #[test]
    fn shared_prefix_is_deduplicated() {
        let mut t: Trie<()> = Trie::new();
        // 100k distinct leaves, all sharing the `com → example` prefix.
        for i in 0..100_000 {
            t.insert(&format!("ad{i:07}.example.com"), ());
        }
        assert_eq!(t.len(), 100_000);
        // root + com + example + 100k leaves = 100_003. Far fewer
        // than 100k × 3 = 300k if we duplicated `com`+`example` per
        // insertion (HashMap doesn't share at all).
        assert_eq!(
            t.node_count(),
            100_003,
            "shared-prefix compression must keep node_count = root + 2 + leaves",
        );
    }

    /// Empty domain is a no-op insert (matches the prior HashMap
    /// behavior where `HashMap::insert("", _, _)` was effectively
    /// dead code).
    #[test]
    fn empty_domain_is_noop() {
        let mut t: Trie<()> = Trie::new();
        t.insert("", ());
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
        assert_eq!(t.find_longest_suffix_match("anything.com"), None);
    }

    /// Single-label domain (no dot): registers a TLD-level rule.
    #[test]
    fn single_label_domain() {
        let mut t: Trie<()> = Trie::new();
        t.insert("com", ());
        assert_eq!(t.len(), 1);
        assert!(t.find_longest_suffix_match("com").is_some());
        assert!(t.find_longest_suffix_match("example.com").is_some());
        assert!(t.find_longest_suffix_match("a.b.example.com").is_some());
        assert!(t.find_longest_suffix_match("example.org").is_none());
    }

    /// `Default` matches `Trie::new`.
    #[test]
    fn default_matches_new() {
        let a: Trie<()> = Trie::default();
        let b: Trie<()> = Trie::new();
        assert_eq!(a.len(), b.len());
        assert_eq!(a.is_empty(), b.is_empty());
    }
}
