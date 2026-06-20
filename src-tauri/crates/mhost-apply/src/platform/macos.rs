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
        let from_escaped = escape_applescript_path(&from.to_string_lossy());
        let to_escaped = escape_applescript_path(&to.to_string_lossy());
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

/// Escape a path for safe use inside an AppleScript string literal.
///
/// AppleScript string literals use `\` as the escape character, so
/// backslashes and double quotes must be escaped.
pub fn escape_applescript_path(path: &str) -> String {
    path.replace('\\', "\\\\").replace('"', "\\\"")
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
            escape_applescript_path("/path/to/file"),
            "/path/to/file"
        );
        assert_eq!(
            escape_applescript_path("/path/with\"quote"),
            "/path/with\\\"quote"
        );
        assert_eq!(
            escape_applescript_path("/path/with\\backslash"),
            "/path/with\\\\backslash"
        );
        assert_eq!(
            escape_applescript_path("/path/with\\\"both"),
            "/path/with\\\\\\\"both"
        );
    }
}
