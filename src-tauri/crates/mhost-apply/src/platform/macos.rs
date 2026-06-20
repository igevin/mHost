//! macOS platform adapter implementation

use std::path::{Path, PathBuf};

use mhost_core::{ApplyError, MhostError};

use super::PlatformAdapter;

/// macOS-specific platform adapter.
///
/// Uses `osascript` for elevated file moves and `dscacheutil` for DNS cache
/// flushing.
pub struct MacOsAdapter;

impl PlatformAdapter for MacOsAdapter {
    fn hosts_path(&self) -> PathBuf {
        PathBuf::from("/etc/hosts")
    }

    fn elevated_move(&self, from: &Path, to: &Path) -> Result<(), MhostError> {
        let from_escaped = escape_applescript_path(&from.to_string_lossy())?;
        let to_escaped = escape_applescript_path(&to.to_string_lossy())?;
        let script = format!(
            "do shell script \"mv {} {}\" with administrator privileges",
            from_escaped, to_escaped
        );
        let output = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ApplyError::PermissionDenied(stderr.to_string()).into());
        }

        Ok(())
    }

    fn flush_dns_cache(&self) -> Result<(), MhostError> {
        std::process::Command::new("dscacheutil")
            .args(["-flushcache"])
            .output()?;
        Ok(())
    }

    fn platform_name(&self) -> &'static str {
        "macos"
    }
}

/// Validate that a path contains only allowed characters.
///
/// Allowed: `[a-zA-Z0-9/._-\\"]`
/// Backslash and double quote are allowed because they are handled by
/// the escaping logic.
fn validate_path_characters(path: &str) -> Result<(), MhostError> {
    let allowed: std::collections::HashSet<char> =
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789/._-\\\""
            .chars()
            .collect();
    if let Some(ch) = path.chars().find(|c| !allowed.contains(c)) {
        return Err(MhostError::InvalidInput(format!(
            "Path contains illegal character: '{}'",
            ch
        )));
    }
    Ok(())
}

/// Escape a path for safe use inside an AppleScript string literal.
///
/// AppleScript string literals use `\` as the escape character, so
/// backslashes and double quotes must be escaped.
///
/// Before escaping, validates that the path only contains allowed characters
/// (`[a-zA-Z0-9/._-]`). If illegal characters are found, returns an error.
pub fn escape_applescript_path(path: &str) -> Result<String, MhostError> {
    validate_path_characters(path)?;
    Ok(path.replace('\\', "\\\\").replace('"', "\\\""))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_applescript_path() {
        assert_eq!(
            escape_applescript_path("/path/to/file").unwrap(),
            "/path/to/file"
        );
        assert_eq!(
            escape_applescript_path("/path/with\"quote").unwrap(),
            "/path/with\\\"quote"
        );
        assert_eq!(
            escape_applescript_path("/path/with\\backslash").unwrap(),
            "/path/with\\\\backslash"
        );
        assert_eq!(
            escape_applescript_path("/path/with\\\"both").unwrap(),
            "/path/with\\\\\\\"both"
        );
    }

    #[test]
    fn test_escape_applescript_path_rejects_illegal_chars() {
        let result = escape_applescript_path("/path/with space");
        assert!(result.is_err(), "space should be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("illegal character"), "error should mention illegal character: {}", msg);

        let result = escape_applescript_path("/path/with;semicolon");
        assert!(result.is_err(), "semicolon should be rejected");

        let result = escape_applescript_path("/path/with$dollar");
        assert!(result.is_err(), "dollar should be rejected");

        let result = escape_applescript_path("/path/with|pipe");
        assert!(result.is_err(), "pipe should be rejected");
    }
}
