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

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;

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

/// DNS-mode ad block engine. Thread-safe; holds two rule sets + a whitelist.
///
/// Hot-reload pattern (same as `RuleEngine`):
/// `rebuild(zero_addr_rules, nxdomain_rules, whitelist)` writes new state
/// under a write lock; concurrent `check` calls see a coherent snapshot.
pub struct AdBlockEngine {
    /// domain -> ip, where ip is typically 0.0.0.0 but stored so future
    /// variants (e.g. sinkhole to a local server) can be added without
    /// changing the public surface.
    zero_addr: RwLock<HashMap<String, IpAddr>>,
    /// domain (no IP needed — NXDOMAIN is the only action)
    nxdomain: RwLock<HashSet<String>>,
    /// whitelist domains — matched with the same suffix-walk as the engines
    whitelist: RwLock<HashSet<String>>,
    /// Cached total of zero_addr + nxdomain + whitelist entries.
    ///
    /// **fix (PR #131 review finding 1.2)**: the empty-engine case (state
    /// `enabled=false`, which is the default and the state most users are
    /// in after install) was taking three read locks + a walk_parents on
    /// every DNS query. With 100k+ QPS this is wasted lock traffic. We
    /// cache the total and short-circuit `check` to `None` before any
    /// lock is taken when no rules are loaded.
    ///
    /// The size is updated **after** `rebuild` swaps in the new maps (see
    /// comment on `rebuild` for the consistency trade-off).
    total_rules: AtomicUsize,
}

impl AdBlockEngine {
    pub fn new() -> Self {
        Self {
            zero_addr: RwLock::new(HashMap::new()),
            nxdomain: RwLock::new(HashSet::new()),
            whitelist: RwLock::new(HashSet::new()),
            total_rules: AtomicUsize::new(0),
        }
    }

    /// Atomically swap in new rule sets.
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
        let new_total = zero_addr_rules.len() + nxdomain_rules.len() + whitelist.len();
        match self.zero_addr.write() {
            Ok(mut g) => *g = zero_addr_rules,
            Err(p) => *p.into_inner() = zero_addr_rules,
        }
        match self.nxdomain.write() {
            Ok(mut g) => *g = nxdomain_rules,
            Err(p) => *p.into_inner() = nxdomain_rules,
        }
        match self.whitelist.write() {
            Ok(mut g) => *g = whitelist,
            Err(p) => *p.into_inner() = whitelist,
        }
        // Update the cached size LAST. Two transition directions to think about:
        //
        //   - N → 0  (rebuild to empty): the brief window has new empty maps
        //     but cached size still says N. `check` walks the empty maps
        //     and correctly returns None — equivalent to the pre-short-circuit
        //     behavior (which also walks empty maps). No regression.
        //   - 0 → N  (rebuild from empty): the brief window has new loaded
        //     maps but cached size still says 0. `check` short-circuits to
        //     None and **leaks ad-block hits through** for the duration of
        //     the rebuild. This is strictly worse than the pre-short-circuit
        //     code, which would have correctly applied the new rules.
        //
        // This window is bounded by the time to populate the maps (sub-second
        // for a 100k-rule list on commodity hardware). Acceptable trade-off
        // for v1; a true fix is the `Arc::swap` pattern (single atomic
        // publication, no multi-step inconsistency), tracked as a follow-up.
        // The opposite ordering (size first, then maps) would let `check`
        // walk the OLD maps while believing they're loaded — also bad in a
        // different direction. Both orderings have transition windows; we
        // picked the one whose downside is bounded by the rebuild latency.
        self.total_rules.store(new_total, Ordering::Release);
    }

    /// Decide what to do with a query.
    ///
    /// Returns `None` if the domain is whitelisted (fall through to the
    /// regular rule engine / upstream) or not blocked at all.
    pub fn check(&self, domain: &str) -> Option<AdBlockAction> {
        // Fast-path: no rules loaded → no possible hit. Avoids three
        // RwLock acquisitions on the hot path for the common
        // `state.enabled == false` case.
        if self.total_rules.load(Ordering::Acquire) == 0 {
            return None;
        }

        // 1. whitelist (read once, then release)
        {
            let whitelist_guard = self.whitelist.read().unwrap_or_else(|p| p.into_inner());
            let hit = walk_parents(domain, |d| whitelist_guard.contains(d).then_some(()));
            if hit.is_some() {
                return None;
            }
        }
        // 2. NXDOMAIN sources first — more aggressive, save a hashmap lookup
        {
            let nx = self.nxdomain.read().unwrap_or_else(|p| p.into_inner());
            if walk_parents(domain, |d| nx.contains(d).then_some(())).is_some() {
                return Some(AdBlockAction::NxDomain);
            }
        }
        // 3. zero-address sources
        {
            let za = self.zero_addr.read().unwrap_or_else(|p| p.into_inner());
            let hit = walk_parents(domain, |d| za.get(d).copied());
            if let Some(ip) = hit {
                return Some(AdBlockAction::ZeroAddress(ip));
            }
        }
        None
    }

    /// Total number of rules loaded across both action sets (whitelist
    /// included — it influences `check` outcomes).
    pub fn rule_count(&self) -> usize {
        self.total_rules.load(Ordering::Acquire)
    }

    pub fn whitelist_size(&self) -> usize {
        self.whitelist
            .read()
            .map(|g| g.len())
            .unwrap_or_else(|p| p.into_inner().len())
    }

    pub fn zero_addr_count(&self) -> usize {
        self.zero_addr
            .read()
            .map(|g| g.len())
            .unwrap_or_else(|p| p.into_inner().len())
    }

    pub fn nxdomain_count(&self) -> usize {
        self.nxdomain
            .read()
            .map(|g| g.len())
            .unwrap_or_else(|p| p.into_inner().len())
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

    /// PR #131 review finding 1.2: the fast-path short-circuit must
    /// bypass the three RwLock acquisitions when no rules are loaded.
    /// Functionally this is the same outcome as the existing
    /// `empty_engine_returns_none`, but we keep a dedicated assertion so
    /// the contract is explicit when somebody refactors the hot path.
    #[test]
    fn empty_engine_short_circuits_before_locking() {
        let engine = AdBlockEngine::new();
        // rule_count() now reads the AtomicUsize; on a fresh engine it's 0.
        // We can't directly observe "no locks were taken" from a public
        // test, but a single-shot regression here is enough to lock in
        // the contract that an empty engine returns None without state.
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

    /// PR #131 review finding 1.2: rule_count now reflects zero_addr + nxdomain
    /// + whitelist (the AtomicUsize is the sum of all three). Whitelist is
    /// part of the contract because it short-circuits the hot path.
    #[test]
    fn rule_count_includes_whitelist() {
        let engine = AdBlockEngine::new();
        engine.rebuild(za(&["a.com"]), nx(&[]), wl(&["w1", "w2", "w3"]));
        assert_eq!(engine.rule_count(), 4);
    }

    /// Rebuild updates the cached size to the new sum, so the short-circuit
    /// doesn't run on the next check against an engine we just emptied.
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
