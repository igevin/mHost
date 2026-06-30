use std::sync::{Arc, Mutex};
use mhost_storage::storage::{FileStorage, Storage};
use mhost_apply::writer::HostsWriter;
use mhost_core::MhostError;

/// Async mutex to serialize apply operations and prevent concurrent writes to /etc/hosts.
/// Security fix (#16): Prevents race conditions when user rapidly toggles profiles.
/// Perf fix (#26): Changed to tokio::sync::Mutex to allow holding across await points.
/// Note: tokio::sync::Mutex does not have poison recovery like std::sync::Mutex.
/// If a spawn_blocking task panics while holding the lock, the lock is released
/// automatically (tokio::sync::Mutex is not poisoned), so recovery is implicit.
pub struct ApplyLock(pub tokio::sync::Mutex<()>);

impl ApplyLock {
    /// Acquire the lock asynchronously.
    pub async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.0.lock().await
    }

    /// Acquire the lock in a blocking context (e.g., `spawn_blocking`).
    pub fn blocking_lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.0.blocking_lock()
    }
}

pub struct AppState {
    pub storage: Arc<dyn Storage + Send + Sync>,
    pub writer: Arc<HostsWriter>,
    pub apply_lock: ApplyLock,
    /// N2: Serialize snapshot save/delete operations to prevent races.
    pub snapshot_lock: ApplyLock,
    /// Perf fix (#29): Track last rendered profile IDs to avoid unnecessary menu rebuilds.
    pub last_profile_ids: Mutex<Vec<String>>,
}

impl AppState {
    pub fn new() -> Result<Self, MhostError> {
        let storage = Arc::new(FileStorage::default()?);
        let writer = Arc::new(HostsWriter::new());
        Ok(Self {
            storage,
            writer,
            apply_lock: ApplyLock(tokio::sync::Mutex::new(())),
            snapshot_lock: ApplyLock(tokio::sync::Mutex::new(())),
            last_profile_ids: Mutex::new(Vec::new()),
        })
    }
}
