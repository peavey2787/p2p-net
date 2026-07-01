//! Desktop platform runtime and filesystem storage implementation.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::common::error::NetError;
use crate::PlatformKind;

use super::{NodeStorage, PlatformRuntime};

/// Default desktop adapter. With `data_dir = None`, config paths are interpreted
/// relative to the current process, matching the default desktop CLI/runtime
/// behaviour. Embedders can provide a data directory to keep node state under an
/// application-owned directory without changing the core node implementation.
#[derive(Debug, Clone, Default)]
pub struct DesktopPlatformRuntime {
    data_dir: Option<PathBuf>,
}

impl DesktopPlatformRuntime {
    pub fn new(data_dir: Option<PathBuf>) -> Self {
        Self { data_dir }
    }

    pub fn with_data_dir(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: Some(data_dir.into()),
        }
    }

    fn resolve_path(&self, key: &str) -> PathBuf {
        let path = PathBuf::from(key);
        if path.is_absolute() {
            return path;
        }
        self.data_dir
            .as_ref()
            .map(|root| root.join(&path))
            .unwrap_or(path)
    }
}

impl NodeStorage for DesktopPlatformRuntime {
    fn storage_kind(&self) -> &'static str {
        "desktop_fs"
    }

    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, NetError> {
        let path = self.resolve_path(key);
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(storage_error(&path, err.to_string())),
        }
    }

    fn write_secret(&self, key: &str, value: &[u8]) -> Result<(), NetError> {
        write_bytes(&self.resolve_path(key), value)
    }

    fn write_public(&self, key: &str, value: &[u8]) -> Result<(), NetError> {
        write_bytes(&self.resolve_path(key), value)
    }

    fn delete(&self, key: &str) -> Result<(), NetError> {
        let path = self.resolve_path(key);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
            Err(err) => Err(storage_error(&path, err.to_string())),
        }
    }
}

impl PlatformRuntime for DesktopPlatformRuntime {
    fn runtime_name(&self) -> &'static str {
        "desktop"
    }

    fn platform_kind(&self) -> PlatformKind {
        PlatformKind::current()
    }

    fn default_data_dir(&self) -> Option<PathBuf> {
        if let Some(data_dir) = self.data_dir.clone() {
            Some(data_dir)
        } else {
            std::env::current_dir().ok()
        }
    }

    fn can_listen_tcp(&self) -> bool {
        true
    }

    fn can_listen_quic(&self) -> bool {
        true
    }

    fn can_accept_inbound(&self) -> Option<bool> {
        None
    }

    fn is_battery_sensitive(&self) -> bool {
        false
    }

    fn is_background_restricted(&self) -> bool {
        false
    }
}

fn write_bytes(path: &Path, value: &[u8]) -> Result<(), NetError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| storage_error(parent, err.to_string()))?;
        }
    }
    fs::write(path, value).map_err(|err| storage_error(path, err.to_string()))
}

fn storage_error(path: &Path, reason: String) -> NetError {
    NetError::Config {
        path: path.display().to_string(),
        reason,
    }
}
