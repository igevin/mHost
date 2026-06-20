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
pub fn build_hosts_content(current: &str, plan: &ApplyPlan) -> String {
    let managed_block = crate::format_as_hosts(&plan.rules);

    if let Some((start, end)) = Parser::extract_managed_block(current) {
        // Replace existing managed block using byte offsets to preserve
        // original formatting including trailing whitespace.
        let line_offsets: Vec<(usize, usize)> = current
            .lines()
            .scan(0, |pos, line| {
                let line_start = *pos;
                // lines() does not include the newline; find it manually
                let after_line = line_start + line.len();
                let nl_len = if current[after_line..].starts_with("\r\n") {
                    2
                } else if current[after_line..].starts_with('\n') {
                    1
                } else {
                    0
                };
                *pos = after_line + nl_len;
                Some((line_start, *pos))
            })
            .collect();

        let block_start = line_offsets[start].0;
        let block_end = line_offsets[end].1;

        let mut output = String::new();
        output.push_str(&current[..block_start]);
        if !managed_block.is_empty() {
            output.push_str(&managed_block);
        }
        output.push_str(&current[block_end..]);
        output
    } else {
        // No managed block — append at the end
        let mut output = current.to_string();
        if !output.ends_with('\n') && !output.is_empty() {
            output.push('\n');
        }
        if !managed_block.is_empty() {
            output.push_str(&managed_block);
        }
        output
    }
}
