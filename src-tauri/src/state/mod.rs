use std::sync::Arc;
use mhost_storage::storage::{FileStorage, Storage};
use mhost_apply::writer::HostsWriter;
use mhost_core::MhostError;

pub struct AppState {
    pub storage: Arc<dyn Storage + Send + Sync>,
    pub writer: HostsWriter,
}

impl AppState {
    pub fn new() -> Result<Self, MhostError> {
        let storage = Arc::new(FileStorage::default()?);
        let writer = HostsWriter::new();
        Ok(Self { storage, writer })
    }
}
