use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::models::ProfileId;

// ---------------------------------------------------------------------------
// MhostError
// ---------------------------------------------------------------------------

#[derive(Error, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MhostError {
    #[error("parse error: {0}")]
    Parse(#[from] ParseError),

    #[error("apply error: {0}")]
    Apply(#[from] ApplyError),

    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("io error: {0}")]
    Io(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),
}

impl From<std::io::Error> for MhostError {
    fn from(err: std::io::Error) -> Self {
        MhostError::Io(err.to_string())
    }
}

// ---------------------------------------------------------------------------
// ParseError
// ---------------------------------------------------------------------------

#[derive(Error, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParseError {
    #[error("invalid IP address: {0}")]
    InvalidIp(String),

    #[error("invalid domain: {0}")]
    InvalidDomain(String),

    #[error("malformed line: {0}")]
    MalformedLine(String),
}

// ---------------------------------------------------------------------------
// ApplyError
// ---------------------------------------------------------------------------

#[derive(Error, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplyError {
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("hosts file not found")]
    HostsFileNotFound,

    #[error("backup failed: {0}")]
    BackupFailed(String),

    #[error("external modification detected")]
    ExternalModification,

    #[error("verification failed: {0}")]
    VerificationFailed(String),
}

// ---------------------------------------------------------------------------
// StorageError
// ---------------------------------------------------------------------------

#[derive(Error, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageError {
    #[error("profile not found: {0}")]
    ProfileNotFound(ProfileId),

    #[error("manifest corrupted: {0}")]
    ManifestCorrupted(String),

    #[error("version mismatch: expected {expected}, found {found}")]
    VersionMismatch { expected: u32, found: u32 },

    #[error("io error: {0}")]
    Io(String),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let cases: Vec<(&str, MhostError, &str)> = vec![
            (
                "parse_invalid_ip",
                MhostError::Parse(ParseError::InvalidIp("999.999.999.999".to_string())),
                "invalid IP address",
            ),
            (
                "parse_invalid_domain",
                MhostError::Parse(ParseError::InvalidDomain("-bad.com".to_string())),
                "invalid domain",
            ),
            (
                "parse_malformed_line",
                MhostError::Parse(ParseError::MalformedLine("foo bar".to_string())),
                "malformed line",
            ),
            (
                "apply_permission_denied",
                MhostError::Apply(ApplyError::PermissionDenied("no sudo".to_string())),
                "permission denied",
            ),
            (
                "apply_hosts_file_not_found",
                MhostError::Apply(ApplyError::HostsFileNotFound),
                "hosts file not found",
            ),
            (
                "apply_backup_failed",
                MhostError::Apply(ApplyError::BackupFailed("disk full".to_string())),
                "backup failed",
            ),
            (
                "apply_external_modification",
                MhostError::Apply(ApplyError::ExternalModification),
                "external modification detected",
            ),
            (
                "apply_verification_failed",
                MhostError::Apply(ApplyError::VerificationFailed("mismatch".to_string())),
                "verification failed",
            ),
            (
                "storage_profile_not_found",
                MhostError::Storage(StorageError::ProfileNotFound(ProfileId(uuid::Uuid::nil()))),
                "profile not found",
            ),
            (
                "storage_manifest_corrupted",
                MhostError::Storage(StorageError::ManifestCorrupted("bad json".to_string())),
                "manifest corrupted",
            ),
            (
                "storage_io",
                MhostError::Storage(StorageError::Io("disk full".to_string())),
                "io error",
            ),
            (
                "storage_version_mismatch",
                MhostError::Storage(StorageError::VersionMismatch {
                    expected: 1,
                    found: 2,
                }),
                "version mismatch",
            ),
            (
                "invalid_input",
                MhostError::InvalidInput("bad args".to_string()),
                "invalid input",
            ),
        ];

        for (name, err, expected_substring) in cases {
            let msg = err.to_string();
            assert!(
                msg.contains(expected_substring),
                "case: {} — expected '{}' in '{}'",
                name,
                expected_substring,
                msg
            );
        }
    }

    #[test]
    fn test_parse_error_from_impl() {
        let parse_err = ParseError::InvalidIp("bad".to_string());
        let mhost_err: MhostError = parse_err.into();
        assert!(mhost_err.to_string().contains("invalid IP address"));
    }

    #[test]
    fn test_apply_error_from_impl() {
        let apply_err = ApplyError::HostsFileNotFound;
        let mhost_err: MhostError = apply_err.into();
        assert!(mhost_err.to_string().contains("hosts file not found"));
    }

    #[test]
    fn test_storage_error_from_impl() {
        let storage_err = StorageError::VersionMismatch {
            expected: 1,
            found: 2,
        };
        let mhost_err: MhostError = storage_err.into();
        assert!(mhost_err.to_string().contains("version mismatch"));
    }

    #[test]
    fn test_io_error_from_impl() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file gone");
        let mhost_err: MhostError = io_err.into();
        assert!(mhost_err.to_string().contains("file gone"));
        // Verify it serializes correctly (Io wraps a String)
        let json = serde_json::to_string(&mhost_err).unwrap();
        let restored: MhostError = serde_json::from_str(&json).unwrap();
        assert_eq!(mhost_err.to_string(), restored.to_string());
    }

    #[test]
    fn test_serde_roundtrip() {
        let cases: Vec<(&str, MhostError)> = vec![
            (
                "parse",
                MhostError::Parse(ParseError::InvalidIp("x".to_string())),
            ),
            (
                "apply",
                MhostError::Apply(ApplyError::PermissionDenied("no".to_string())),
            ),
            (
                "storage",
                MhostError::Storage(StorageError::ManifestCorrupted("bad".to_string())),
            ),
            (
                "storage_io",
                MhostError::Storage(StorageError::Io("bad".to_string())),
            ),
            ("invalid_input", MhostError::InvalidInput("bad".to_string())),
        ];

        for (name, err) in cases {
            let json = serde_json::to_string(&err).unwrap();
            let restored: MhostError = serde_json::from_str(&json).unwrap();
            assert_eq!(err.to_string(), restored.to_string(), "case: {}", name);
        }
    }
}
