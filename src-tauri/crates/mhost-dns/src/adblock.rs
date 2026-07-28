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
}

impl AdBlockEngine {
    pub fn new() -> Self {
        Self {
            zero_addr: RwLock::new(HashMap::new()),
            nxdomain: RwLock::new(HashSet::new()),
            whitelist: RwLock::new(HashSet::new()),
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
    }

    /// Decide what to do with a query.
    ///
    /// Returns `None` if the domain is whitelisted (fall through to the
    /// regular rule engine / upstream) or not blocked at all.
    pub fn check(&self, domain: &str) -> Option<AdBlockAction> {
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

    /// Total number of rules loaded across both action sets.
    pub fn rule_count(&self) -> usize {
        let z = self
            .zero_addr
            .read()
            .map(|g| g.len())
            .unwrap_or_else(|p| p.into_inner().len());
        let n = self
            .nxdomain
            .read()
            .map(|g| g.len())
            .unwrap_or_else(|p| p.into_inner().len());
        z + n
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
