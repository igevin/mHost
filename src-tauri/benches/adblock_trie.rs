//! Bench harness for the trie-based ad-block engine (issue #199 sub-task A).
//!
//! This is **not** a Criterion bench — the repo deliberately avoids the
//! `criterion` dependency (CI runs `cargo test`, not `cargo bench`,
//! and these measurements are for landing-time verification of the
//! issue's "100k rules, lookup p99 < 1µs" target, not for tracking
//! regressions over time).
//!
//! Each measurement prints median / p95 / p99 latency in nanoseconds.
//! Run with:
//!
//! ```bash
//! cd src-tauri
//! cargo bench --bench adblock_trie -- --nocapture
//! ```
//!
//! Or to only run the lookup bench:
//! ```bash
//! cargo bench --bench adblock_trie -- --nocapture lookup
//! ```

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::Instant;

use mhost_dns::adblock::AdBlockEngine;

/// Build a synthetic 100k-rule zero-addr dataset. Each domain shares
/// the `example.com` prefix so the trie's shared-prefix compression has
/// something to demonstrate — worst case is the HashMap baseline (no
/// prefix sharing at all).
fn make_100k_rules() -> HashMap<String, std::net::IpAddr> {
    let mut out = HashMap::with_capacity(100_000);
    let ip = std::net::IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0));
    for i in 0..100_000 {
        // 7-digit zero-padded index. All 100k rules share `com`+`example`
        // prefix — the trie should compress these aggressively.
        out.insert(format!("ad{i:07}.example.com"), ip);
    }
    out
}

fn make_queries() -> Vec<String> {
    // Mix of hit / miss queries:
    //  * 70% lookups against registered `*.example.com` domains (hits)
    //  * 30% lookups against unregistered domains (misses — walks the
    //    full label chain before returning)
    let mut queries = Vec::with_capacity(10_000);
    for i in 0..10_000 {
        if i % 10 < 7 {
            queries.push(format!("ad{i:07}.example.com"));
        } else {
            // 3-deep unregistered domains — same depth as a hit, so the
            // trie has to walk all 3 labels.
            queries.push(format!("sub{i:07}.other.com"));
        }
    }
    queries
}

fn percentiles(samples: &mut [u128]) -> (u128, u128, u128) {
    samples.sort_unstable();
    let p50 = samples[samples.len() / 2];
    let p95 = samples[(samples.len() as f64 * 0.95) as usize];
    let p99 = samples[(samples.len() as f64 * 0.99) as usize];
    (p50, p95, p99)
}

fn bench_lookup() {
    let rules = make_100k_rules();
    let queries = make_queries();

    let engine = AdBlockEngine::new();
    engine.rebuild(rules, Default::default(), Default::default());
    engine.set_enabled(true);

    // Warm up — first lookup pays for lazy initialization in the trie
    // (HashMap bucket allocation, etc.). We want steady-state numbers.
    for q in &queries {
        let _ = engine.check(q);
    }

    let mut samples = Vec::with_capacity(queries.len());
    for q in &queries {
        let t = Instant::now();
        let _ = engine.check(q);
        samples.push(t.elapsed().as_nanos());
    }

    let (p50, p95, p99) = percentiles(&mut samples);
    println!(
        "\n=== adblock_trie::lookup (100k rules, {} queries) ===",
        queries.len()
    );
    println!("  p50: {p50} ns");
    println!("  p95: {p95} ns");
    println!("  p99: {p99} ns  (issue #199 target: < 1µs = 1000 ns)");

    if p99 > 1000 {
        eprintln!(
            "  FAIL: p99 ({p99} ns) exceeds the issue #199 1µs target. \
             investigate the trie lookup path before merging."
        );
        // Don't fail the bench (CI doesn't run benches); the assertion is
        // a code-review signal.
    } else {
        println!("  PASS: under the 1µs target.");
    }
}

fn bench_memory_node_count() {
    let rules = make_100k_rules();

    // The trie representation lives inside the engine's snapshot but is
    // not directly accessible. We replicate the trie build here to
    // expose `node_count()` for the bench output.
    let mut trie = mhost_dns::trie::Trie::new();
    for (domain, ip) in &rules {
        trie.insert(domain, *ip);
    }

    println!("\n=== adblock_trie::memory (100k zero-addr rules) ===");
    println!(
        "  node_count: {} (expected: 100_003 = 1 root + com + example + 100k leaves)",
        trie.node_count()
    );
    println!("  rule_count: {} (expected: 100_000)", trie.len());

    // A rough memory estimate: HashMap<String, IpAddr> with 100k entries
    // is ~50 MB on x86_64 (issue #199 estimate). We can't easily measure
    // the trie's exact heap footprint without a custom allocator, so
    // just print the structural counts and leave memory verification to
    // a manual `heaptrack` run during code review.
}

fn main() {
    // Honor an optional first CLI arg as a coarse test selector so the
    // bench binary is callable as `cargo bench -- --nocapture lookup`.
    let arg = std::env::args().nth(1).unwrap_or_default();
    let run_lookup = arg.is_empty() || arg == "lookup";
    let run_memory = arg.is_empty() || arg == "memory";
    if run_lookup {
        bench_lookup();
    }
    if run_memory {
        bench_memory_node_count();
    }
}
