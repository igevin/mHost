//! Verification for the hosts writer
//!
//! Verifies that the written hosts file content matches the expected
//! apply plan.

use mhost_core::{ApplyError, ApplyPlan, MhostError};
use mhost_hosts::Parser;
use std::collections::HashSet;

/// Verify that the written content matches the expected plan.
pub fn verify(written: &str, plan: &ApplyPlan) -> Result<(), MhostError> {
    // Basic verification: check that the managed block markers exist
    // if the plan has rules, and that all expected rules are present.
    if plan.rules.is_empty() {
        // If no rules, there should be no managed block
        if Parser::extract_managed_block(written).is_some() {
            return Err(ApplyError::VerificationFailed(
                "expected no managed block but found one".to_string(),
            )
            .into());
        }
        return Ok(());
    }

    let block = Parser::extract_managed_block(written);
    if block.is_none() {
        return Err(
            ApplyError::VerificationFailed("managed block missing".to_string()).into(),
        );
    }

    // Extract managed block lines into a HashSet for O(1) lookup
    let managed_content = Parser::extract_managed_block_content(written).unwrap_or_default();
    let written_lines: HashSet<&str> = managed_content.lines().collect();

    for rule in &plan.rules {
        let expected = format!("{} {}", rule.ip, rule.domain);
        if !written_lines.contains(expected.as_str()) {
            return Err(ApplyError::VerificationFailed(format!(
                "expected rule '{}' not found",
                expected
            ))
            .into());
        }
    }

    Ok(())
}
