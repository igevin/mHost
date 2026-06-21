use std::sync::{Arc, MutexGuard};
use mhost_storage::storage::{FileStorage, Storage};
use mhost_apply::writer::HostsWriter;
use mhost_core::MhostError;

/// Mutex to serialize apply operations and prevent concurrent writes to /etc/hosts.
/// Security fix (#16): Prevents race conditions when user rapidly toggles profiles.
pub struct ApplyLock(pub std::sync::Mutex<()>);

impl ApplyLock {
    /// Acquire the lock, recovering from poison if a previous holder panicked.
    /// This prevents the entire app from becoming unusable after one failed apply.
    pub fn lock(&self) -> MutexGuard<'_, ()> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub struct AppState {
    pub storage: Arc<dyn Storage + Send + Sync>,
    pub writer: HostsWriter,
    pub apply_lock: ApplyLock,
}

impl AppState {
    pub fn new() -> Result<Self, MhostError> {
        let storage = Arc::new(FileStorage::default()?);
        let writer = HostsWriter::new();
        Ok(Self {
            storage,
            writer,
            apply_lock: ApplyLock(std::sync::Mutex::new(())),
        })
    }
}
