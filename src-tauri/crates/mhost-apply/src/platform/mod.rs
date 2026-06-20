//! Platform abstraction layer
//!
//! Provides a `PlatformAdapter` trait that abstracts platform-specific operations
//! such as file moving with elevated privileges, DNS cache flushing, and hosts
//! file path resolution. A factory function `create_platform_adapter()` returns
//! the correct implementation based on the target OS.

pub mod macos;

use std::path::{Path, PathBuf};

use mhost_core::MhostError;

pub use macos::MacOsAdapter;

/// Trait for platform-specific operations required by the hosts writer.
///
/// Each platform (macOS, Windows, Linux) provides its own implementation.
pub trait PlatformAdapter: Send + Sync {
    /// Return the path to the system hosts file.
    fn hosts_path(&self) -> PathBuf;

    /// Move a file with elevated privileges (e.g., administrator/root).
    fn elevated_move(&self, from: &Path, to: &Path) -> Result<(), MhostError>;

    /// Flush the system DNS cache.
    fn flush_dns_cache(&self) -> Result<(), MhostError>;

    /// Return the name of the platform (for logging / diagnostics).
    fn platform_name(&self) -> &'static str;
}

/// Factory function that returns the appropriate `PlatformAdapter` for the
/// current target OS.
pub fn create_platform_adapter() -> Box<dyn PlatformAdapter> {
    #[cfg(target_os = "macos")]
    {
        Box::new(MacOsAdapter)
    }
    #[cfg(target_os = "windows")]
    {
        Box::new(WindowsAdapter)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        compile_error!("Unsupported platform")
    }
}

/// Placeholder for future Windows support.
#[cfg(target_os = "windows")]
pub struct WindowsAdapter;

#[cfg(target_os = "windows")]
impl PlatformAdapter for WindowsAdapter {
    fn hosts_path(&self) -> PathBuf {
        PathBuf::from(r"C:\Windows\System32\drivers\etc\hosts")
    }

    fn elevated_move(&self, _from: &Path, _to: &Path) -> Result<(), MhostError> {
        todo!("Windows elevated move not yet implemented")
    }

    fn flush_dns_cache(&self) -> Result<(), MhostError> {
        todo!("Windows DNS cache flush not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "windows"
    }
}
