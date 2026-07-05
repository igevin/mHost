//! Content building for the hosts writer
//!
//! Builds the new hosts file content by preserving unmanaged lines and
//! replacing (or appending) the mHost managed block.

use mhost_core::ApplyPlan;
use mhost_hosts::Parser;

/// Build the new hosts content.
///
/// - If a managed block exists, remove it and replace with the new block.
/// - If no managed block exists, append the new block at the end.
/// - All unmanaged content is preserved exactly as-is, including trailing
///   whitespace.
///
/// **fix (P-R6, issue #90)**: was using `Parser::extract_managed_block`
/// which returns **line indices**, then doing a second full-file scan with
/// `current.lines().scan(0, ...).collect()` to convert to byte offsets —
/// for a 5000-line ad-block hosts file this allocated a 5000-element Vec
/// and walked every line just to look up 2 entries. Now uses
/// `Parser::extract_managed_block_bytes` which returns byte offsets
/// directly in a single pass. The full-file scan is gone.
pub fn build_hosts_content(current: &str, plan: &ApplyPlan) -> String {
    let managed_block = crate::format_as_hosts(&plan.rules);

    if let Some((block_start, block_end)) = Parser::extract_managed_block_bytes(current) {
        // Replace existing managed block using byte offsets to preserve
        // original formatting including trailing whitespace.
        let mut output = String::with_capacity(current.len() + managed_block.len());
        output.push_str(&current[..block_start]);
        if !managed_block.is_empty() {
            output.push_str(&managed_block);
        }
        output.push_str(&current[block_end..]);
        output
    } else {
        // No managed block — append at the end
        let mut output = String::with_capacity(current.len() + managed_block.len() + 1);
        output.push_str(current);
        if !output.ends_with('\n') && !output.is_empty() {
            output.push('\n');
        }
        if !managed_block.is_empty() {
            output.push_str(&managed_block);
        }
        output
    }
}
