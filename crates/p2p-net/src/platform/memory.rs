//! In-memory storage for tests and embedders that want to adapt persistence at a
//! higher layer.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::common::error::NetError;

use super::NodeStorage;

#[derive(Debug, Clone, Default)]
pub struct MemoryNodeStorage {
    entries: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
}

impl MemoryNodeStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

impl NodeStorage for MemoryNodeStorage {
    fn storage_kind(&self) -> &'static str {
        "memory"
    }

    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, NetError> {
        let entries = self.entries.lock().map_err(|_| storage_error(key))?;
        Ok(entries.get(key).cloned())
    }

    fn write_secret(&self, key: &str, value: &[u8]) -> Result<(), NetError> {
        self.write_public(key, value)
    }

    fn write_public(&self, key: &str, value: &[u8]) -> Result<(), NetError> {
        let mut entries = self.entries.lock().map_err(|_| storage_error(key))?;
        entries.insert(key.to_string(), value.to_vec());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), NetError> {
        let mut entries = self.entries.lock().map_err(|_| storage_error(key))?;
        entries.remove(key);
        Ok(())
    }
}

fn storage_error(key: &str) -> NetError {
    NetError::Config {
        path: key.to_string(),
        reason: "node storage mutex poisoned".to_string(),
    }
}
