//! Domain-centric rule lookup — the backend for the Cmd+K command palette
//! (issue #124).
//!
//! # Aggregation path (deliberately independent of `mhost-apply::Merger`)
//!
//! `Merger` dedupes by `(domain, ip)` and `ResolvedRule` carries no line
//! numbers, so it can't power a lookup UI. This module builds its own
//! read-only path over three sources:
//!
//! 1. **DNS-mode profiles** — `storage.list_profiles_by_mode(Dns)`; the same
//!    list `DnsServer::reload_rules` consumes, so what we show is what the
//!    resolver serves. Only matches from *enabled* profiles with *enabled*
//!    rules are effective, and only while DNS mode is on. Profiles are
//!    sorted by name before collection to mirror `RuleEngine::rebuild`'s
//!    deterministic first-wins order. Matching mirrors `RuleEngine::resolve`
//!    (fix #79): a rule also hits when its domain is a dot-suffix of the
//!    query (`ad.example.com` → rule for `example.com`, `via_suffix=true`);
//!    exact hits outrank suffix hits, longer suffixes outrank shorter ones.
//!
//!    Known pre-existing divergence (not fixed here): the live resolver
//!    compares `Name::to_utf8()` bytes as-is, so a 0x20-case-randomized
//!    DNS query can miss a lowercase rule; the palette matches
//!    case-insensitively (the right call for a human-facing lookup).
//! 2. **Hosts-mode profiles** — `AppState::cached_profiles()` (the #181
//!    profile cache; all mutation IPCs invalidate it). First enabled
//!    profile in list order wins per domain (mirrors the apply merger's
//!    first-wins conflict rule).
//! 3. **System hosts** — `/etc/hosts` with the mHost managed block blanked
//!    out line-for-line before parsing, so `parse_with_lines` reports
//!    line numbers in the *original file*. Managed-block lines are excluded
//!    because those rules already show up as profile matches. If the block
//!    markers are malformed (duplicated/misordered → strict extraction
//!    fails), a lenient first-start-to-last-end span is masked instead;
//!    when no sane span exists at all, system-hosts hits duplicating a
//!    profile match's `(domain, ip)` are dropped as managed-block leaks.
//!
//! # Line numbers for profile rules
//!
//! Profiles are stored as structured JSON (`Profile.rules`), not hosts
//! text — there is no source file to re-parse. The editor
//! (`RuleEditor.rulesToText`) renders exactly one line per rule, so a
//! rule's 1-based index in `Profile.rules` IS its editor line number.
//! The palette's jump feature navigates to that line.

use std::sync::atomic::Ordering;

use mhost_core::{MhostError, Profile, ProfileMode};
use mhost_hosts::parser::Parser;
use serde::Serialize;
use tauri::State;

use crate::state::AppState;

/// Where a matched rule lives. Tagged union for the frontend renderer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MatchSource {
    /// A rule line in `/etc/hosts` outside mHost's managed block.
    SystemHosts { line: usize },
    /// A rule in a hosts-mode profile. `enabled` is the *profile's* flag.
    Profile {
        id: String,
        name: String,
        line: usize,
        enabled: bool,
    },
    /// A rule in a DNS-mode profile (served by the local resolver when
    /// DNS mode is on).
    DnsRule {
        id: String,
        name: String,
        line: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuleMatch {
    pub domain: String,
    pub ip: String,
    pub source: MatchSource,
    /// Whether this rule currently participates in resolution.
    /// `false` → grayed out in the UI; can never be the winner.
    pub effective: bool,
    /// Exactly one match — the one that wins within mHost's precedence
    /// model (DNS > enabled profiles > system hosts). Note this is NOT a
    /// full simulation of OS resolution order: the managed block is
    /// appended at the end of `/etc/hosts`, so a same-name entry the user
    /// hand-wrote *before* the block would actually win `getaddrinfo`.
    /// This mirrors the apply/diff modeling assumption (PR #244 review).
    pub is_winner: bool,
    /// `true` when the match came from the DNS resolver's suffix walk
    /// (fix #79, mirrored — see module doc): the rule's domain is a
    /// dot-suffix of the query, e.g. query `ad.example.com` hits a rule
    /// for `example.com`. Always serialized (issue #202 lesson).
    pub via_suffix: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QueryDomainResult {
    pub query: String,
    pub matches: Vec<RuleMatch>,
}

// ---------------------------------------------------------------------------
// Fuzzy matching (hand-rolled; mirrors src/lib/fuzzy.ts — issue #124 forbids
// a fuzzy library)
// ---------------------------------------------------------------------------

/// fzf-like subsequence matcher, case-insensitive.
///
/// Returns `(score, positions)` when every query char appears in order in
/// `text`, else `None`. Score rewards contiguous runs and matches at
/// segment boundaries (start of text or right after `.` / `-` / `_`) and
/// penalizes a late first match, so `api.local` ranks above
/// `xpi.aocal` for query `apiloc`.
fn fuzzy_score(query: &str, text: &str) -> Option<(i32, Vec<usize>)> {
    if query.is_empty() {
        return None;
    }
    let q: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let t: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();

    let mut positions: Vec<usize> = Vec::with_capacity(q.len());
    let mut cursor = 0usize;
    for qc in &q {
        let hit = t[cursor..].iter().position(|c| c == qc)? + cursor;
        positions.push(hit);
        cursor = hit + 1;
    }

    let mut score = 0i32;
    for (i, &pos) in positions.iter().enumerate() {
        if i > 0 && pos == positions[i - 1] + 1 {
            score += 10; // contiguous run
        }
        if pos == 0 || matches!(t[pos - 1], '.' | '-' | '_') {
            score += 5; // segment boundary
        }
    }
    score -= positions[0] as i32; // earlier start wins ties
    Some((score, positions))
}

/// Mirror of `RuleEngine::resolve`'s suffix walk (fix #79): the live DNS
/// resolver answers `query` with a rule for `domain` iff `domain == query`
/// or `domain` is a dot-suffix of `query` (`ad.example.com` →
/// `example.com`). Returns the specificity: `usize::MAX` for an exact
/// match, otherwise the matched domain's label count (more labels = the
/// resolver walks into it earlier). `None` = the resolver would NOT hit
/// this rule for this query.
///
/// Case-insensitive. Known pre-existing divergence (do NOT "fix" here):
/// the live resolver compares `Name::to_utf8()` bytes as-is
/// (server.rs `resolve` path), so a 0x20-case-randomized query misses a
/// lowercase rule. The palette is case-insensitive by design — documented
/// divergence, not a bug to propagate.
fn resolver_hit_specificity(query: &str, domain: &str) -> Option<usize> {
    let q = query.to_lowercase();
    let d = domain.to_lowercase();
    if q == d {
        return Some(usize::MAX);
    }
    if q.len() > d.len() + 1 && q.ends_with(&d) && q.as_bytes()[q.len() - d.len() - 1] == b'.' {
        // Label count of the matched suffix — more labels = more specific.
        return Some(d.matches('.').count() + 1);
    }
    None
}

// ---------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------

/// Internal sort envelope — keeps sort keys out of the wire format.
struct Scored {
    m: RuleMatch,
    /// 0 = DNS rule, 1 = enabled hosts profile, 2 = system hosts,
    /// 3 = anything non-effective (disabled profile / rule / DNS off).
    rank: u8,
    /// DNS resolver specificity (PR #244 R1): exact > longer suffix > not
    /// a resolver hit (fuzzy-only). 0 for non-DNS rows and fuzzy-only DNS
    /// rows — resolver hits always outrank fuzzy-only within rank 0.
    specificity: usize,
    /// Winner eligibility. Fuzzy-only DNS matches are shown (lookup is a
    /// finding aid) but can never be the WIN — the live resolver would not
    /// answer the query with them. Non-DNS rows keep the pre-existing
    /// fuzzy-winner semantics (`true` here).
    resolver_hit: bool,
    score: i32,
    /// Source-list order: (profile index, rule index) for profile rows,
    /// (line, 0) for system hosts rows.
    order: (usize, usize),
}

/// Collect matches from one profile list (hosts- or DNS-mode).
///
/// PR #244 review (R1): for DNS profiles we ALSO match the query's parent
/// suffixes, mirroring `RuleEngine::resolve` (fix #79) — a rule for
/// `example.com` intercepts `ad.example.com`, so it must appear (and be
/// eligible for WIN) here. Fuzzy-only DNS matches are still shown (lookup
/// is a *finding* aid) but sort behind resolver hits and cannot outrank
/// them for the winner badge.
///
/// PR #244 review (R2): DNS profiles are sorted by name before iteration,
/// mirroring `RuleEngine::rebuild`'s deterministic first-wins order
/// (storage's `read_dir` enumeration order is filesystem-dependent).
fn collect_profile_matches(
    query: &str,
    profiles: &[Profile],
    mode: ProfileMode,
    dns_enabled: bool,
    out: &mut Vec<Scored>,
) {
    let mut ordered: Vec<&Profile> = profiles.iter().collect();
    if mode == ProfileMode::Dns {
        ordered.sort_by(|a, b| a.name.cmp(&b.name));
    }

    for (profile_idx, profile) in ordered.iter().enumerate() {
        for (rule_idx, rule) in profile.rules.iter().enumerate() {
            // Comment-only rules carry no IP and can't match a domain.
            let Some(ip) = rule.ip else { continue };
            // 1-based editor line: RuleEditor renders one line per rule.
            let line = rule_idx + 1;
            for domain in &rule.domains {
                let fuzzy = fuzzy_score(query, domain);
                let resolver_spec = if mode == ProfileMode::Dns {
                    resolver_hit_specificity(query, domain)
                } else {
                    None
                };
                if fuzzy.is_none() && resolver_spec.is_none() {
                    continue;
                }
                let (via_suffix, specificity) = match resolver_spec {
                    Some(usize::MAX) => (false, usize::MAX), // exact
                    Some(spec) => (true, spec),              // suffix hit
                    None => (false, 0),                      // fuzzy-only
                };
                // Hosts/system rows keep pre-existing fuzzy-winner
                // semantics; DNS rows need a resolver hit to be eligible.
                let resolver_hit = mode == ProfileMode::Hosts || resolver_spec.is_some();
                let score = fuzzy.map(|(s, _)| s).unwrap_or(0);
                // A rule is effective only when its whole delivery chain is
                // on: profile enabled AND rule enabled AND (for DNS profiles)
                // DNS mode running.
                let effective =
                    profile.enabled && rule.enabled && (mode == ProfileMode::Hosts || dns_enabled);
                let rank = match (mode, effective) {
                    (ProfileMode::Dns, true) => 0,
                    (ProfileMode::Hosts, true) => 1,
                    _ => 3,
                };
                let source = match mode {
                    ProfileMode::Hosts => MatchSource::Profile {
                        id: profile.id.to_string(),
                        name: profile.name.clone(),
                        line,
                        enabled: profile.enabled,
                    },
                    ProfileMode::Dns => MatchSource::DnsRule {
                        id: profile.id.to_string(),
                        name: profile.name.clone(),
                        line,
                    },
                };
                out.push(Scored {
                    m: RuleMatch {
                        domain: domain.clone(),
                        ip: ip.to_string(),
                        source,
                        effective,
                        is_winner: false,
                        via_suffix,
                    },
                    rank,
                    specificity,
                    resolver_hit,
                    score,
                    order: (profile_idx, rule_idx),
                });
            }
        }
    }
}

/// Mask the managed block out of a hosts file, line-for-line (blanking
/// keeps `parse_with_lines` line numbers aligned with the original file).
///
/// Returns `(masked_text, mask_applied)`. **PR #244 review (R3)**: the
/// strict `extract_managed_block` returns `None` on duplicated or
/// misordered markers, which used to leak managed rules into system-hosts
/// matches. Fallback: mask from the *first* start marker through the *last*
/// end marker; if markers are too broken to locate a sane span, return the
/// input unmasked and let the caller dedupe against profile matches.
fn mask_managed_block(text: &str) -> (String, bool) {
    let range = Parser::extract_managed_block(text).or_else(|| {
        let lines: Vec<&str> = text.lines().collect();
        let start = lines
            .iter()
            .position(|l| l.trim() == mhost_hosts::parser::MANAGED_START);
        let end = lines
            .iter()
            .rposition(|l| l.trim() == mhost_hosts::parser::MANAGED_END);
        match (start, end) {
            (Some(s), Some(e)) if s <= e => Some((s, e)),
            _ => None,
        }
    });

    match range {
        Some((start, end)) => (
            text.lines()
                .enumerate()
                .map(|(i, line)| if (start..=end).contains(&i) { "" } else { line })
                .collect::<Vec<_>>()
                .join("\n"),
            true,
        ),
        None => (text.to_string(), false),
    }
}

/// Collect matches from `/etc/hosts`, excluding mHost's managed block.
///
/// `profile_keys` holds `(domain, ip)` of every profile match; it is only
/// consulted when the managed block could NOT be masked (markers too broken
/// to trust) — then any system-hosts hit duplicating a profile match is a
/// managed-block leak and is dropped (PR #244 R3).
fn collect_system_hosts_matches(
    query: &str,
    hosts_text: &str,
    profile_keys: &std::collections::HashSet<(String, String)>,
    out: &mut Vec<Scored>,
) {
    let (masked, mask_applied) = mask_managed_block(hosts_text);

    let parsed = Parser::parse_with_lines(&masked);
    for rule in &parsed.rules {
        let Some(ip) = rule.ip else { continue };
        // Skip rules without a line number instead of inventing line 0.
        let Some(line) = rule.line_number else {
            continue;
        };
        for domain in &rule.domains {
            let Some((score, _positions)) = fuzzy_score(query, domain) else {
                continue;
            };
            if !mask_applied && profile_keys.contains(&(domain.clone(), ip.to_string())) {
                continue;
            }
            out.push(Scored {
                m: RuleMatch {
                    domain: domain.clone(),
                    ip: ip.to_string(),
                    source: MatchSource::SystemHosts { line },
                    effective: true,
                    is_winner: false,
                    via_suffix: false,
                },
                rank: 2,
                specificity: 0,
                // System hosts keeps pre-existing fuzzy-winner semantics.
                resolver_hit: true,
                score,
                order: (line, 0),
            });
        }
    }
}

/// Aggregate domain matches across DNS profiles, hosts profiles and the
/// system hosts file; sort by source priority and mark the winner.
///
/// Priority (issue #124):
///   1. DNS proxy rules (only while DNS mode is enabled) beat everything
///   2. enabled hosts-mode profiles, first profile in list order wins
///   3. system hosts as fallback
///
/// Non-effective matches (disabled profiles/rules, DNS off) sort last
/// and can never be the winner.
pub(crate) fn aggregate_domain_matches(
    query: &str,
    hosts_profiles: &[Profile],
    dns_profiles: &[Profile],
    system_hosts_text: &str,
    dns_enabled: bool,
) -> Vec<RuleMatch> {
    let mut scored: Vec<Scored> = Vec::new();
    collect_profile_matches(
        query,
        dns_profiles,
        ProfileMode::Dns,
        dns_enabled,
        &mut scored,
    );
    collect_profile_matches(
        query,
        hosts_profiles,
        ProfileMode::Hosts,
        dns_enabled,
        &mut scored,
    );
    // R3 fallback: only consulted when the managed block couldn't be masked.
    let profile_keys: std::collections::HashSet<(String, String)> = scored
        .iter()
        .map(|s| (s.m.domain.clone(), s.m.ip.clone()))
        .collect();
    collect_system_hosts_matches(query, system_hosts_text, &profile_keys, &mut scored);

    scored.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
            .then(b.specificity.cmp(&a.specificity))
            .then(b.score.cmp(&a.score))
            .then(a.order.cmp(&b.order))
    });

    let mut winner_assigned = false;
    scored
        .into_iter()
        .map(|s| {
            let mut m = s.m;
            if !winner_assigned && s.resolver_hit && m.effective {
                m.is_winner = true;
                winner_assigned = true;
            }
            m
        })
        .collect()
}

// ---------------------------------------------------------------------------
// IPC
// ---------------------------------------------------------------------------

/// `query_domain` — domain lookup for the command palette (issue #124).
///
/// Read-only: never mutates rules, only reads storage + `/etc/hosts`.
#[tauri::command]
pub async fn query_domain(
    state: State<'_, AppState>,
    query: String,
) -> Result<QueryDomainResult, MhostError> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Ok(QueryDomainResult {
            query,
            matches: Vec::new(),
        });
    }

    // Hosts-mode profiles come from the #181 cache (every mutation IPC
    // invalidates it, so this is fresh). PR #244 review (R4): the read must
    // happen INSIDE spawn_blocking — on a cache miss it synchronously walks
    // and parses the profiles dir, which must not block a Tokio worker.
    let storage = state.storage.clone();
    let profile_cache = state.cached_profiles.clone();
    let dns_enabled = state.dns_enabled.load(Ordering::Relaxed);

    tauri::async_runtime::spawn_blocking(move || {
        let hosts_profiles = crate::state::cached_profiles_read(&profile_cache, &storage)?;
        let dns_profiles = storage.list_profiles_by_mode(ProfileMode::Dns)?;
        // /etc/hosts unreadable (permissions, missing) is not fatal for a
        // lookup — degrade to profile/DNS matches only.
        let system_hosts = std::fs::read_to_string("/etc/hosts").unwrap_or_default();
        Ok(QueryDomainResult {
            matches: aggregate_domain_matches(
                &query,
                &hosts_profiles,
                &dns_profiles,
                &system_hosts,
                dns_enabled,
            ),
            query,
        })
    })
    .await
    // JoinError = the blocking task panicked/cancelled; MhostError has no
    // better-fitting variant, so InvalidInput carries the message.
    .map_err(|e| MhostError::InvalidInput(e.to_string()))?
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mhost_core::{HostRule, ProfileId};
    use std::net::IpAddr;
    use uuid::Uuid;

    fn make_profile(
        name: &str,
        mode: ProfileMode,
        enabled: bool,
        rules: Vec<(&str, &str)>,
    ) -> Profile {
        let mut p = Profile::new(name);
        p.id = ProfileId(Uuid::new_v4());
        p.mode = mode;
        p.enabled = enabled;
        for (ip, domain) in rules {
            p.rules.push(HostRule::new(
                ip.parse::<IpAddr>().unwrap(),
                vec![domain.to_string()],
            ));
        }
        p
    }

    fn winner(matches: &[RuleMatch]) -> Option<&RuleMatch> {
        matches.iter().find(|m| m.is_winner)
    }

    // ---- fuzzy_score ----

    #[test]
    fn fuzzy_matches_subsequence_case_insensitive() {
        assert!(fuzzy_score("apiloc", "api.local").is_some());
        assert!(fuzzy_score("API", "api.local").is_some());
        assert!(fuzzy_score("zzz", "api.local").is_none());
        assert!(fuzzy_score("", "api.local").is_none());
    }

    #[test]
    fn fuzzy_prefers_contiguous_match() {
        let (_, contiguous) = fuzzy_score("api", "api.local").unwrap();
        let (sparse_score, _) = fuzzy_score("aao", "a.b.c.a.o").unwrap();
        let (contig_score, _) = fuzzy_score("api", "api.local").unwrap();
        assert_eq!(contiguous, vec![0, 1, 2]);
        assert!(contig_score > sparse_score);
    }

    // ---- ranking / winner ----

    #[test]
    fn enabled_profile_wins_over_system_hosts() {
        let hosts_profiles = vec![make_profile(
            "dev",
            ProfileMode::Hosts,
            true,
            vec![("10.0.0.1", "api.local")],
        )];
        let system = "127.0.0.1 localhost\n192.168.1.1 api.local\n";
        let matches = aggregate_domain_matches("api.local", &hosts_profiles, &[], system, false);
        assert_eq!(matches.len(), 2);
        let w = winner(&matches).expect("a winner exists");
        assert_eq!(w.ip, "10.0.0.1");
        assert!(matches!(w.source, MatchSource::Profile { .. }));
        // Sorted by source priority: profile first, system hosts second.
        assert!(matches!(matches[1].source, MatchSource::SystemHosts { .. }));
        assert!(!matches[1].is_winner);
    }

    #[test]
    fn dns_rule_beats_hosts_when_dns_mode_on() {
        let dns_profiles = vec![make_profile(
            "dns-adb",
            ProfileMode::Dns,
            true,
            vec![("0.0.0.0", "api.local")],
        )];
        let hosts_profiles = vec![make_profile(
            "dev",
            ProfileMode::Hosts,
            true,
            vec![("10.0.0.1", "api.local")],
        )];
        let matches =
            aggregate_domain_matches("api.local", &hosts_profiles, &dns_profiles, "", true);
        let w = winner(&matches).unwrap();
        assert!(matches!(w.source, MatchSource::DnsRule { .. }));
        assert_eq!(w.ip, "0.0.0.0");
        // DNS match sorts first.
        assert!(matches!(matches[0].source, MatchSource::DnsRule { .. }));
    }

    #[test]
    fn dns_rule_not_effective_when_dns_mode_off() {
        let dns_profiles = vec![make_profile(
            "dns-adb",
            ProfileMode::Dns,
            true,
            vec![("0.0.0.0", "api.local")],
        )];
        let hosts_profiles = vec![make_profile(
            "dev",
            ProfileMode::Hosts,
            true,
            vec![("10.0.0.1", "api.local")],
        )];
        let matches =
            aggregate_domain_matches("api.local", &hosts_profiles, &dns_profiles, "", false);
        let w = winner(&matches).unwrap();
        // DNS off → DNS rule degrades to non-effective, hosts profile wins.
        assert!(matches!(w.source, MatchSource::Profile { .. }));
        assert_eq!(w.ip, "10.0.0.1");
        let dns_match = matches
            .iter()
            .find(|m| matches!(m.source, MatchSource::DnsRule { .. }))
            .unwrap();
        assert!(!dns_match.effective);
        assert!(!dns_match.is_winner);
    }

    #[test]
    fn disabled_profile_matches_shown_but_cannot_win() {
        let hosts_profiles = vec![
            make_profile(
                "disabled-one",
                ProfileMode::Hosts,
                false,
                vec![("10.0.0.9", "api.local")],
            ),
            make_profile(
                "enabled-one",
                ProfileMode::Hosts,
                true,
                vec![("10.0.0.1", "api.local")],
            ),
        ];
        let matches = aggregate_domain_matches("api.local", &hosts_profiles, &[], "", false);
        assert_eq!(matches.len(), 2);
        let w = winner(&matches).unwrap();
        match &w.source {
            MatchSource::Profile { name, .. } => assert_eq!(name, "enabled-one"),
            _ => panic!("winner must be the enabled profile"),
        }
        let disabled = matches
            .iter()
            .find(|m| !m.effective)
            .expect("disabled match present");
        assert!(!disabled.is_winner);
        // 非有效匹配排在最后（rank 3）。
        assert!(!matches.last().unwrap().effective);
    }

    #[test]
    fn disabled_rule_inside_enabled_profile_cannot_win() {
        let mut p = make_profile(
            "dev",
            ProfileMode::Hosts,
            true,
            vec![("10.0.0.1", "api.local")],
        );
        p.rules[0].enabled = false; // editor 里被注释掉的行
        let system = "192.168.1.1 api.local\n";
        let matches = aggregate_domain_matches("api.local", &[p], &[], system, false);
        let w = winner(&matches).unwrap();
        assert!(matches!(w.source, MatchSource::SystemHosts { .. }));
    }

    #[test]
    fn first_enabled_profile_wins_in_list_order() {
        let hosts_profiles = vec![
            make_profile(
                "a-first",
                ProfileMode::Hosts,
                true,
                vec![("10.0.0.1", "api.local")],
            ),
            make_profile(
                "b-second",
                ProfileMode::Hosts,
                true,
                vec![("10.0.0.2", "api.local")],
            ),
        ];
        let matches = aggregate_domain_matches("api.local", &hosts_profiles, &[], "", false);
        let w = winner(&matches).unwrap();
        match &w.source {
            MatchSource::Profile { name, .. } => assert_eq!(name, "a-first"),
            _ => unreachable!(),
        }
        // Shadowed match follows the winner in list order.
        assert_eq!(matches[1].ip, "10.0.0.2");
    }

    #[test]
    fn system_hosts_is_fallback_winner_when_no_profile_matches() {
        let hosts_profiles = vec![make_profile(
            "dev",
            ProfileMode::Hosts,
            true,
            vec![("10.0.0.1", "other.local")],
        )];
        let system = "127.0.0.1 localhost\n8.8.8.8 dns.google\n";
        let matches = aggregate_domain_matches("dns.google", &hosts_profiles, &[], system, false);
        assert_eq!(matches.len(), 1);
        let w = winner(&matches).unwrap();
        match w.source {
            MatchSource::SystemHosts { line } => assert_eq!(line, 2),
            _ => panic!("system hosts must be the fallback winner"),
        }
    }

    #[test]
    fn managed_block_rules_excluded_from_system_hosts() {
        let system = "127.0.0.1 localhost\n# ---- mHost start ----\n10.0.0.1 api.local\n# ---- mHost end ----\n";
        // /etc/hosts 里 managed block 内的 api.local 来自 profile ——
        // 不能再作为 system hosts 匹配出现一次。
        let matches = aggregate_domain_matches("api.local", &[], &[], system, false);
        assert!(matches.is_empty());
    }

    #[test]
    fn system_hosts_line_numbers_survive_managed_block_masking() {
        let system = "127.0.0.1 localhost\n# ---- mHost start ----\n10.0.0.1 managed.local\n# ---- mHost end ----\n192.168.1.50 nas.local\n";
        let matches = aggregate_domain_matches("nas.local", &[], &[], system, false);
        assert_eq!(matches.len(), 1);
        match matches[0].source {
            // 原文件的物理行号（第 5 行），不是 mask 后的相对行号。
            MatchSource::SystemHosts { line } => assert_eq!(line, 5),
            _ => unreachable!(),
        }
    }

    #[test]
    fn profile_line_is_editor_line_index() {
        let hosts_profiles = vec![make_profile(
            "dev",
            ProfileMode::Hosts,
            true,
            vec![("10.0.0.1", "a.local"), ("10.0.0.2", "api.local")],
        )];
        let matches = aggregate_domain_matches("api.local", &hosts_profiles, &[], "", false);
        match &matches[0].source {
            // RuleEditor 一行一条规则：第二条规则在第 2 行。
            MatchSource::Profile { line, .. } => assert_eq!(*line, 2),
            _ => unreachable!(),
        }
    }

    #[test]
    fn fuzzy_query_matches_domain() {
        let hosts_profiles = vec![make_profile(
            "dev",
            ProfileMode::Hosts,
            true,
            vec![("10.0.0.1", "api.local")],
        )];
        let matches = aggregate_domain_matches("apiloc", &hosts_profiles, &[], "", false);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].domain, "api.local");
    }

    #[test]
    fn no_match_returns_empty() {
        let hosts_profiles = vec![make_profile(
            "dev",
            ProfileMode::Hosts,
            true,
            vec![("10.0.0.1", "api.local")],
        )];
        let matches = aggregate_domain_matches("zzz", &hosts_profiles, &[], "", false);
        assert!(matches.is_empty());
    }

    #[test]
    fn winner_is_none_when_everything_is_disabled() {
        let hosts_profiles = vec![make_profile(
            "off",
            ProfileMode::Hosts,
            false,
            vec![("10.0.0.1", "api.local")],
        )];
        let matches = aggregate_domain_matches("api.local", &hosts_profiles, &[], "", false);
        assert_eq!(matches.len(), 1);
        assert!(winner(&matches).is_none());
    }

    #[test]
    fn match_source_wire_format_is_tagged_snake_case() {
        let m = RuleMatch {
            domain: "api.local".into(),
            ip: "10.0.0.1".into(),
            source: MatchSource::Profile {
                id: "id-1".into(),
                name: "dev".into(),
                line: 3,
                enabled: true,
            },
            effective: true,
            is_winner: true,
            via_suffix: false,
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"type\":\"profile\""), "{}", json);
        assert!(json.contains("\"is_winner\":true"), "{}", json);
        // via_suffix 永远在线上（issue #202 教训：绝不省略字段）。
        assert!(json.contains("\"via_suffix\":false"), "{}", json);
        let sys = RuleMatch {
            domain: "x".into(),
            ip: "1.1.1.1".into(),
            source: MatchSource::SystemHosts { line: 7 },
            effective: true,
            is_winner: false,
            via_suffix: false,
        };
        assert!(serde_json::to_string(&sys)
            .unwrap()
            .contains("\"type\":\"system_hosts\""));
        let dns = RuleMatch {
            domain: "x".into(),
            ip: "0.0.0.0".into(),
            source: MatchSource::DnsRule {
                id: "d".into(),
                name: "ads".into(),
                line: 1,
            },
            effective: true,
            is_winner: false,
            via_suffix: false,
        };
        assert!(serde_json::to_string(&dns)
            .unwrap()
            .contains("\"type\":\"dns_rule\""));
    }

    // ---- PR #244 review follow-ups (R1/R2/R3/R5) ----

    #[test]
    fn resolver_hit_specificity_mirrors_suffix_walk() {
        assert_eq!(
            resolver_hit_specificity("example.com", "example.com"),
            Some(usize::MAX)
        );
        assert_eq!(
            resolver_hit_specificity("ad.example.com", "example.com"),
            Some(2)
        );
        assert_eq!(
            resolver_hit_specificity("a.b.example.com", "example.com"),
            Some(2)
        );
        assert_eq!(
            resolver_hit_specificity("a.b.example.com", "b.example.com"),
            Some(3)
        );
        // 兄弟域名 / 父域不是 query 的后缀 → 不命中
        assert_eq!(
            resolver_hit_specificity("notexample.com", "example.com"),
            None
        );
        assert_eq!(
            resolver_hit_specificity("example.com", "ad.example.com"),
            None
        );
        // 大小写不敏感
        assert!(resolver_hit_specificity("AD.EXAMPLE.COM", "Example.com").is_some());
    }

    /// R1: 子域名查询命中父域 DNS 规则并压过 hosts 条目拿 WIN。
    #[test]
    fn dns_suffix_match_wins_over_hosts_entries() {
        let dns_profiles = vec![make_profile(
            "ads",
            ProfileMode::Dns,
            true,
            vec![("0.0.0.0", "example.com")],
        )];
        let hosts_profiles = vec![make_profile(
            "dev",
            ProfileMode::Hosts,
            true,
            vec![("10.0.0.1", "ad.example.com")],
        )];
        let matches =
            aggregate_domain_matches("ad.example.com", &hosts_profiles, &dns_profiles, "", true);
        let w = winner(&matches).expect("winner exists");
        match &w.source {
            MatchSource::DnsRule { name, .. } => assert_eq!(name, "ads"),
            _ => panic!("suffix-matched DNS rule must win"),
        }
        assert!(w.via_suffix);
        assert_eq!(w.ip, "0.0.0.0");
        // hosts 那条还在，但只能排在后面且不赢
        assert!(matches
            .iter()
            .any(|m| matches!(m.source, MatchSource::Profile { .. }) && !m.is_winner));
    }

    /// R1: 更具体的规则先赢（resolver 从完整域名向上走）。
    #[test]
    fn dns_suffix_more_specific_rule_wins() {
        let dns_profiles = vec![make_profile(
            "ads",
            ProfileMode::Dns,
            true,
            vec![("0.0.0.0", "example.com"), ("127.0.0.1", "sub.example.com")],
        )];
        let matches = aggregate_domain_matches("a.sub.example.com", &[], &dns_profiles, "", true);
        let w = winner(&matches).unwrap();
        assert_eq!(w.ip, "127.0.0.1");
        assert!(w.via_suffix);
        // 排序：更具体的 suffix 排前面
        assert_eq!(matches[0].domain, "sub.example.com");
        assert_eq!(matches[1].domain, "example.com");
    }

    /// R1: fuzzy-only 的 DNS 匹配仍然展示（查找是「找规则」场景），但
    /// 排在 resolver hit 之后，且永远不能成为 WIN —— 真实 resolver 不会
    /// 用它应答这个 query。
    #[test]
    fn fuzzy_only_dns_match_ranks_behind_resolver_hits_and_cannot_win() {
        let dns_profiles = vec![
            // suffix hit：query 的 dot-suffix
            make_profile(
                "a-suffix",
                ProfileMode::Dns,
                true,
                vec![("0.0.0.0", "api.local")],
            ),
            // fuzzy-only：query 的字符是它的子序列，但 resolver 不会命中
            make_profile(
                "b-fuzzy",
                ProfileMode::Dns,
                true,
                vec![("0.0.0.2", "subzero.api.local")],
            ),
        ];
        let matches = aggregate_domain_matches("sub.api.local", &[], &dns_profiles, "", true);
        assert_eq!(matches.len(), 2);
        // resolver hit 排第一且赢
        assert_eq!(matches[0].domain, "api.local");
        assert!(matches[0].via_suffix);
        assert!(matches[0].is_winner);
        // fuzzy-only 排第二、不赢
        assert_eq!(matches[1].domain, "subzero.api.local");
        assert!(!matches[1].is_winner);

        // 只剩 fuzzy-only DNS 匹配时，winner 让给 hosts/system 的精确匹配
        let hosts_profiles = vec![make_profile(
            "dev",
            ProfileMode::Hosts,
            true,
            vec![("10.0.0.1", "sub.api.local")],
        )];
        let matches = aggregate_domain_matches(
            "sub.api.local",
            &hosts_profiles,
            &dns_profiles[1..],
            "",
            true,
        );
        let w = winner(&matches).unwrap();
        assert!(matches!(w.source, MatchSource::Profile { .. }));
    }

    /// R2: 同一域名出现在两个启用的 DNS profile —— winner 与 resolver
    /// 的 by-name first-wins 一致，与输入顺序无关。
    #[test]
    fn dns_winner_follows_resolver_name_order() {
        let zeta = make_profile(
            "zeta",
            ProfileMode::Dns,
            true,
            vec![("10.0.0.1", "shared.com")],
        );
        let alpha = make_profile(
            "alpha",
            ProfileMode::Dns,
            true,
            vec![("10.0.0.2", "shared.com")],
        );
        // 输入顺序 zeta 在前（模拟 read_dir 的任意顺序）
        let matches = aggregate_domain_matches("shared.com", &[], &[zeta, alpha], "", true);
        let w = winner(&matches).unwrap();
        match &w.source {
            MatchSource::DnsRule { name, .. } => assert_eq!(name, "alpha"),
            _ => unreachable!(),
        }
        assert_eq!(w.ip, "10.0.0.2");
    }

    /// R3: 标记损坏（重复 start marker）→ 严格提取失败 → lenient 区间
    /// mask 兜底，managed 规则不会作为 system_hosts 重复出现。
    #[test]
    fn malformed_managed_block_falls_back_to_lenient_mask() {
        let system = "127.0.0.1 localhost\n# ---- mHost start ----\n10.0.0.1 api.local\n# ---- mHost start ----\n10.0.0.2 other.local\n# ---- mHost end ----\n";
        let matches = aggregate_domain_matches("api.local", &[], &[], system, false);
        assert!(matches.is_empty(), "managed leak despite broken markers");
    }

    /// R3: 标记完全不可用（只有 start 没有 end）→ 不做 mask，改为按
    /// (domain, ip) 去重 profile 已有匹配 —— managed 泄漏被去重拦截，
    /// 但同域名不同 IP 的**用户手写**条目（块外）仍保留。
    #[test]
    fn unmaskable_managed_block_dedupes_profile_duplicates() {
        let hosts_profiles = vec![make_profile(
            "dev",
            ProfileMode::Hosts,
            true,
            vec![("10.0.0.1", "api.local")],
        )];
        // 只有一个 start marker → 无有效区间 → dedupe 路径
        let system = "# ---- mHost start ----\n10.0.0.1 api.local\n";
        let matches = aggregate_domain_matches("api.local", &hosts_profiles, &[], system, false);
        assert_eq!(matches.len(), 1, "profile 一条 + system 去重后零条");
        assert!(matches!(matches[0].source, MatchSource::Profile { .. }));
    }

    /// R5: 多域名规则 —— 每个域名独立匹配、各自成行。
    #[test]
    fn multi_domain_rule_matches_each_domain() {
        let mut p = Profile::new("dev");
        p.enabled = true;
        p.rules.push(HostRule::new(
            "10.0.0.1".parse::<IpAddr>().unwrap(),
            vec!["a.local".to_string(), "b.local".to_string()],
        ));
        let a = aggregate_domain_matches("a.local", &[p.clone()], &[], "", false);
        let b = aggregate_domain_matches("b.local", &[p], &[], "", false);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].domain, "a.local");
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].domain, "b.local");
        // 同一条规则 → 同一编辑器行号
        match (&a[0].source, &b[0].source) {
            (MatchSource::Profile { line: la, .. }, MatchSource::Profile { line: lb, .. }) => {
                assert_eq!(la, lb);
            }
            _ => unreachable!(),
        }
    }
}
