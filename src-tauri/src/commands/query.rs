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
//!    rules are effective, and only while DNS mode is on.
//! 2. **Hosts-mode profiles** — `AppState::cached_profiles()` (the #181
//!    profile cache; all mutation IPCs invalidate it). First enabled
//!    profile in list order wins per domain (mirrors the apply merger's
//!    first-wins conflict rule).
//! 3. **System hosts** — `/etc/hosts` with the mHost managed block blanked
//!    out line-for-line before parsing, so `parse_with_lines` reports
//!    line numbers in the *original file*. Managed-block lines are excluded
//!    because those rules already show up as profile matches.
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
    /// Exactly one match — the one that actually wins resolution.
    pub is_winner: bool,
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

// ---------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------

/// Internal sort envelope — keeps sort keys out of the wire format.
struct Scored {
    m: RuleMatch,
    /// 0 = DNS rule, 1 = enabled hosts profile, 2 = system hosts,
    /// 3 = anything non-effective (disabled profile / rule / DNS off).
    rank: u8,
    score: i32,
    /// Source-list order (profile index; line order for system hosts).
    order: usize,
}

/// Collect matches from one profile list (hosts- or DNS-mode).
fn collect_profile_matches(
    query: &str,
    profiles: &[Profile],
    mode: ProfileMode,
    dns_enabled: bool,
    out: &mut Vec<Scored>,
) {
    for (profile_idx, profile) in profiles.iter().enumerate() {
        for (rule_idx, rule) in profile.rules.iter().enumerate() {
            // Comment-only rules carry no IP and can't match a domain.
            let Some(ip) = rule.ip else { continue };
            // 1-based editor line: RuleEditor renders one line per rule.
            let line = rule_idx + 1;
            for domain in &rule.domains {
                let Some((score, _positions)) = fuzzy_score(query, domain) else {
                    continue;
                };
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
                    },
                    rank,
                    score,
                    order: profile_idx * 1_000_000 + rule_idx,
                });
            }
        }
    }
}

/// Collect matches from `/etc/hosts`, excluding mHost's managed block.
///
/// The managed block is blanked line-for-line (not removed) so that
/// `parse_with_lines` line numbers stay aligned with the original file.
fn collect_system_hosts_matches(query: &str, hosts_text: &str, out: &mut Vec<Scored>) {
    let masked = match Parser::extract_managed_block(hosts_text) {
        Some((start, end)) => hosts_text
            .lines()
            .enumerate()
            .map(|(i, line)| if (start..=end).contains(&i) { "" } else { line })
            .collect::<Vec<_>>()
            .join("\n"),
        None => hosts_text.to_string(),
    };

    let parsed = Parser::parse_with_lines(&masked);
    for rule in &parsed.rules {
        let Some(ip) = rule.ip else { continue };
        // parse_with_lines always sets line_number; fallback is unreachable.
        let line = rule.line_number.unwrap_or(0);
        for domain in &rule.domains {
            let Some((score, _positions)) = fuzzy_score(query, domain) else {
                continue;
            };
            out.push(Scored {
                m: RuleMatch {
                    domain: domain.clone(),
                    ip: ip.to_string(),
                    source: MatchSource::SystemHosts { line },
                    effective: true,
                    is_winner: false,
                },
                rank: 2,
                score,
                order: line,
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
    collect_system_hosts_matches(query, system_hosts_text, &mut scored);

    scored.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
            .then(b.score.cmp(&a.score))
            .then(a.order.cmp(&b.order))
    });

    let mut winner_assigned = false;
    scored
        .into_iter()
        .map(|s| {
            let mut m = s.m;
            if !winner_assigned && m.effective {
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
    // invalidates it, so this is fresh). Read before spawn_blocking: the
    // cache path is a non-blocking RwLock read.
    let hosts_profiles = state.cached_profiles()?;
    let dns_enabled = state.dns_enabled.load(Ordering::Relaxed);
    let storage = state.storage.clone();

    tauri::async_runtime::spawn_blocking(move || {
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
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"type\":\"profile\""), "{}", json);
        assert!(json.contains("\"is_winner\":true"), "{}", json);
        let sys = RuleMatch {
            domain: "x".into(),
            ip: "1.1.1.1".into(),
            source: MatchSource::SystemHosts { line: 7 },
            effective: true,
            is_winner: false,
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
        };
        assert!(serde_json::to_string(&dns)
            .unwrap()
            .contains("\"type\":\"dns_rule\""));
    }
}
