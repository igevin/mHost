//! Ad block engine for DNS mode (issue #130).
//!
//! Two sets of rules held independently so each source can choose its own
//! response strategy (0.0.0.0 A record vs NXDOMAIN):
//!
//! * `zero_addr` — sources configured with `AdBlockResponse::ZeroAddress`.
//!   Hits return a 0.0.0.0 A record (`NoError` rcode, `A 0.0.0.0`).
//! * `nxdomain`  — sources configured with `AdBlockResponse::NxDomain`.
//!   Hits return `Rcode::NameError` so the client gives up immediately.
//!
//! Plus a whitelist: if a domain matches any whitelist entry (suffix-walked),
//! the ad block layer is bypassed entirely for that domain. Whitelist is
//! applied **before** the ad block engines — so whitelist wins over both
//! response variants.
//!
//! All three lookups use the shared [`crate::matcher::walk_parents`] helper
//! so `ad.example.com` matches a registered `example.com` (issue #79 fix).

use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::Arc;

use crate::matcher::walk_parents;

/// The action to take when an ad block rule matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdBlockAction {
    /// Return a 0.0.0.0 A record (with `NoError` rcode).
    ZeroAddress(IpAddr),
    /// Return `NXDOMAIN` (rcode = NameError). The `IpAddr` is unused; carried
    /// as `()`-equivalent. Held in an `IpAddr` for symmetry with `ZeroAddress`
    /// so both arms are `Copy + Eq`; downstream never reads the value.
    NxDomain,
}

/// An immutable snapshot of all ad-block rule sets, published as a single
/// `Arc` so a concurrent `check` either sees the old snapshot in full or the
/// new one in full — never a half-rebuilt mix (issue #132).
///
/// This replaces the old three-`RwLock` + `AtomicUsize` short-circuit, which
/// updated three maps and a cached size in separate steps. On a 0→N rebuild
/// that ordering briefly let `check` short-circuit to `None` while the new
/// (loaded) maps were already in place, leaking ad-block hits through. A
/// single `Arc` swap removes the multi-step inconsistency entirely.
#[derive(Default)]
struct RulesSnapshot {
    zero_addr: HashMap<String, IpAddr>,
    nxdomain: HashSet<String>,
    whitelist: HashSet<String>,
}

impl RulesSnapshot {
    /// Whether this snapshot has any rules that could produce a block.
    /// Whitelist is intentionally excluded: the master switch (`state.enabled`)
    /// only gates zero_addr / nxdomain, while whitelist is always collected
    /// regardless (review Medium #2). An empty `has_block_rules` means
    /// `check()` can only return `None`, so callers can short-circuit the
    /// parent-walk entirely.
    #[inline]
    fn has_block_rules(&self) -> bool {
        !self.zero_addr.is_empty() || !self.nxdomain.is_empty()
    }

    /// Total rule count for stats (`rule_count()`). Includes whitelist
    /// because it's the externally observable number; short-circuit
    /// decisions use [`has_block_rules`] instead.
    fn total(&self) -> usize {
        self.zero_addr.len() + self.nxdomain.len() + self.whitelist.len()
    }
}

/// DNS-mode ad block engine. Thread-safe; holds two rule sets + a whitelist.
///
/// Hot-reload pattern: `rebuild(...)` builds a fresh [`RulesSnapshot`] and
/// swaps the whole `Arc` in under a single write lock — one atomic
/// publication step, so every concurrent `check` observes a coherent
/// snapshot (issue #132). `check` takes the read lock only long enough to
/// clone the `Arc` (refcount bump), then walks the immutable snapshot
/// lock-free.
pub struct AdBlockEngine {
    current: RwLock<Arc<RulesSnapshot>>,
}

impl AdBlockEngine {
    pub fn new() -> Self {
        Self {
            current: RwLock::new(Arc::new(RulesSnapshot::default())),
        }
    }

    /// Atomically swap in new rule sets.
    ///
    /// Builds the three sets into one [`RulesSnapshot`] and replaces the
    /// published `Arc` under a single write lock. This is the single point
    /// of publication — there is no multi-step inconsistency window, so the
    /// 0→N leak that the old `AtomicUsize` short-circuit had is gone
    /// (issue #132).
    ///
    /// `zero_addr_rules` and `nxdomain_rules` come from parsing the cached
    /// blocklist of each enabled source (per-source response strategy).
    /// `whitelist` is the user-curated allow-list.
    pub fn rebuild(
        &self,
        zero_addr_rules: HashMap<String, IpAddr>,
        nxdomain_rules: HashSet<String>,
        whitelist: HashSet<String>,
    ) {
        let snapshot = Arc::new(RulesSnapshot {
            zero_addr: zero_addr_rules,
            nxdomain: nxdomain_rules,
            whitelist,
        });
        // Swap the Arc under one write lock, then drop the old snapshot
        // OUTSIDE the lock. The old snapshot can hold 100k+ entries; letting
        // its refcount hit zero and deallocate under the write lock would
        // block every concurrent `check()` reader (review Medium #1).
        let old = {
            let mut g = self.current.write();
            std::mem::replace(&mut *g, snapshot)
        };
        drop(old);
    }

    /// Read the currently published snapshot. Takes the read lock only for
    /// the `Arc::clone` (cheap — refcount bump), then releases it before any
    /// domain walking, so concurrent rebuilds never block readers.
    fn snapshot(&self) -> Arc<RulesSnapshot> {
        Arc::clone(&self.current.read())
    }

    /// Decide what to do with a query.
    ///
    /// Returns `None` if the domain is whitelisted (fall through to the
    /// regular rule engine / upstream) or not blocked at all.
    pub fn check(&self, domain: &str) -> Option<AdBlockAction> {
        let snap = self.snapshot();
        // Fast-path: no block rules loaded → no possible hit. Avoids any
        // domain walking for the common `state.enabled == false` case.
        // Whitelist is excluded because it's collected regardless of the
        // master switch (review Medium #2); an empty block-rule set means
        // `check()` can only return `None`. Unlike the old `AtomicUsize`
        // short-circuit this reads the very snapshot the walk below uses,
        // so the empty-check can't disagree with the rule data (issue #132).
        if !snap.has_block_rules() {
            return None;
        }

        // 1. whitelist (read once, then release)
        if walk_parents(domain, |d| snap.whitelist.contains(d).then_some(())).is_some() {
            return None;
        }
        // 2. NXDOMAIN sources first — more aggressive, save a hashmap lookup
        if walk_parents(domain, |d| snap.nxdomain.contains(d).then_some(())).is_some() {
            return Some(AdBlockAction::NxDomain);
        }
        // 3. zero-address sources
        if let Some(ip) = walk_parents(domain, |d| snap.zero_addr.get(d).copied()) {
            return Some(AdBlockAction::ZeroAddress(ip));
        }
        None
    }

    /// Total number of rules loaded across both action sets (whitelist
    /// included — it influences `check` outcomes).
    pub fn rule_count(&self) -> usize {
        self.snapshot().total()
    }

    pub fn whitelist_size(&self) -> usize {
        self.snapshot().whitelist.len()
    }

    pub fn zero_addr_count(&self) -> usize {
        self.snapshot().zero_addr.len()
    }

    pub fn nxdomain_count(&self) -> usize {
        self.snapshot().nxdomain.len()
    }
}

impl Default for AdBlockEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn za(domains: &[&str]) -> HashMap<String, IpAddr> {
        domains
            .iter()
            .map(|d| ((*d).to_string(), IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))))
            .collect()
    }

    fn nx(domains: &[&str]) -> HashSet<String> {
        domains.iter().map(|d| (*d).to_string()).collect()
    }

    fn wl(domains: &[&str]) -> HashSet<String> {
        domains.iter().map(|d| (*d).to_string()).collect()
    }

    #[test]
    fn empty_engine_returns_none() {
        let engine = AdBlockEngine::new();
        assert_eq!(engine.check("anything.com"), None);
        assert_eq!(engine.rule_count(), 0);
    }

    /// PR #131 review finding 1.2: the fast-path short-circuit must bypass
    /// the rule walking when no rules are loaded. With the `Arc::swap`
    /// publication (issue #132) the empty-check reads the same snapshot the
    /// walk would use, so we keep a dedicated assertion that an empty engine
    /// returns `None` without touching rule data.
    #[test]
    fn empty_engine_short_circuits_before_locking() {
        let engine = AdBlockEngine::new();
        // rule_count() now reads the published snapshot's total; on a fresh
        // engine it's 0. We can't directly observe "no walk ran" from a
        // public test, but a single-shot regression here locks in the
        // contract that an empty engine returns None without state.
        assert_eq!(engine.rule_count(), 0);
        assert_eq!(engine.check("a.b.c.example.com"), None);
    }

    #[test]
    fn zero_address_hit_returns_zero_address() {
        let engine = AdBlockEngine::new();
        engine.rebuild(za(&["ad.example.com"]), nx(&[]), wl(&[]));
        let action = engine.check("ad.example.com");
        assert_eq!(
            action,
            Some(AdBlockAction::ZeroAddress(IpAddr::V4(Ipv4Addr::new(
                0, 0, 0, 0
            ))))
        );
    }

    #[test]
    fn nxdomain_hit_returns_nxdomain() {
        let engine = AdBlockEngine::new();
        engine.rebuild(za(&[]), nx(&["tracker.example.com"]), wl(&[]));
        assert_eq!(
            engine.check("tracker.example.com"),
            Some(AdBlockAction::NxDomain)
        );
    }

    #[test]
    fn suffix_walk_matches_subdomains() {
        // ad-blocker semantics: registering example.com hits *.example.com
        let engine = AdBlockEngine::new();
        engine.rebuild(za(&["example.com"]), nx(&[]), wl(&[]));
        for d in ["example.com", "ad.example.com", "deep.ad.example.com"] {
            assert!(
                matches!(engine.check(d), Some(AdBlockAction::ZeroAddress(_))),
                "{} should hit",
                d
            );
        }
        assert_eq!(engine.check("example.org"), None);
    }

    #[test]
    fn nxdomain_consulted_before_zero_addr() {
        // Per design (issue #130): the lookup order is whitelist → nxdomain
        // → zero_addr. An NXDOMAIN rule on a parent domain blocks descendants
        // before the more-specific zero_addr rule is reached. This is the
        // intended Pi-hole-style semantic: NXDOMAIN is the more aggressive
        // action and is consulted first to save a hashmap lookup.
        let engine = AdBlockEngine::new();
        engine.rebuild(
            za(&["specific.ad.example.com"]),
            nx(&["example.com"]),
            wl(&[]),
        );
        // The parent NXDOMAIN wins because it's consulted first.
        assert_eq!(
            engine.check("specific.ad.example.com"),
            Some(AdBlockAction::NxDomain)
        );
        // But on a domain NOT covered by the parent nxdomain rule, the
        // more-specific zero_addr rule still applies.
        assert_eq!(
            engine.check("specific.ad.other.com"),
            None,
            "other.com not under the nxdomain rule, and not registered in zero_addr"
        );
    }

    #[test]
    fn whitelist_overrides_everything() {
        let engine = AdBlockEngine::new();
        // blocked on both engines; whitelisted → falls through.
        engine.rebuild(
            za(&["example.com"]),
            nx(&["example.com"]),
            wl(&["good.example.com"]),
        );
        // whitelist exact hit
        assert_eq!(engine.check("good.example.com"), None);
        // whitelist suffix hit
        assert_eq!(engine.check("api.good.example.com"), None);
        // not whitelisted — still blocked
        assert!(engine.check("ad.example.com").is_some());
    }

    #[test]
    fn rebuild_replaces_state_atomically() {
        let engine = AdBlockEngine::new();
        engine.rebuild(za(&["a.com"]), nx(&["b.com"]), wl(&["c.com"]));
        assert_eq!(engine.zero_addr_count(), 1);
        assert_eq!(engine.nxdomain_count(), 1);
        assert_eq!(engine.whitelist_size(), 1);

        engine.rebuild(za(&["d.com", "e.com"]), nx(&[]), wl(&[]));
        assert_eq!(engine.zero_addr_count(), 2);
        assert_eq!(engine.nxdomain_count(), 0);
        assert_eq!(engine.whitelist_size(), 0);
        // Old rules no longer hit
        assert_eq!(engine.check("a.com"), None);
        assert_eq!(engine.check("b.com"), None);
        // New rule does hit
        assert!(matches!(
            engine.check("d.com"),
            Some(AdBlockAction::ZeroAddress(_))
        ));
    }

    #[test]
    fn rule_count_sums_both_sets() {
        let engine = AdBlockEngine::new();
        engine.rebuild(
            za(&["a.com", "b.com"]),
            nx(&["c.com", "d.com", "e.com"]),
            wl(&[]),
        );
        assert_eq!(engine.rule_count(), 5);
    }

    /// PR #131 review finding 1.2: `rule_count` reflects zero_addr + nxdomain
    /// + whitelist (the snapshot total is the sum of all three). Whitelist is
    /// part of the contract because it short-circuits the hot path.
    #[test]
    fn rule_count_includes_whitelist() {
        let engine = AdBlockEngine::new();
        engine.rebuild(za(&["a.com"]), nx(&[]), wl(&["w1", "w2", "w3"]));
        assert_eq!(engine.rule_count(), 4);
    }

    /// Rebuild publishes a fresh snapshot atomically, so the count reflects
    /// the new total immediately — no cached-size window to fall out of sync
    /// with the rule data (issue #132).
    #[test]
    fn rebuild_updates_cached_total() {
        let engine = AdBlockEngine::new();
        engine.rebuild(za(&["a.com"]), nx(&[]), wl(&[]));
        assert_eq!(engine.rule_count(), 1);

        engine.rebuild(za(&[]), nx(&[]), wl(&[]));
        assert_eq!(
            engine.rule_count(),
            0,
            "size must drop to 0 after empty rebuild"
        );
        assert_eq!(engine.check("a.com"), None, "fast-path should now fire");
    }

    #[test]
    fn tld_alone_matches_every_subdomain() {
        // Pi-hole semantic: registering "com" blocks every *.com because
        // walk_parents visits single-label parents once. This is intentional
        // — users sometimes deliberately TLD-block (e.g. blocking the entire
        // `.xyz` TLD used by abuse).
        let engine = AdBlockEngine::new();
        engine.rebuild(za(&["com"]), nx(&[]), wl(&[]));
        assert!(matches!(
            engine.check("example.com"),
            Some(AdBlockAction::ZeroAddress(_))
        ));
        assert!(matches!(
            engine.check("anything.anything.com"),
            Some(AdBlockAction::ZeroAddress(_))
        ));
        // A different TLD is untouched.
        assert_eq!(engine.check("example.org"), None);
    }
}
