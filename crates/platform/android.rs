//! Android runtime and app-private filesystem storage.
//!
//! Android-specific lifecycle and JNI/UI concerns live under `apps/android`.
//! This module contains only the platform facts and durable storage adapter that
//! the shared Rust networking core needs.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};

use crate::common::error::NetError;
use crate::PlatformKind;

use super::{NodeStorage, PlatformRuntime};

/// Runtime/storage adapter for the native Android foreground-service host.
///
/// The Android app intentionally runs the node in a foreground service while it
/// is enabled. That makes TCP/QUIC listen sockets available to the shared core;
/// actual Internet reachability is still discovered by normal AutoNAT/relay
/// logic rather than assumed here.
#[derive(Debug, Clone)]
pub struct AndroidPlatformRuntime {
    data_dir: PathBuf,
    foreground_service: bool,
}

impl AndroidPlatformRuntime {
    /// Create the production Android adapter used by the foreground service.
    pub fn foreground_service(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            foreground_service: true,
        }
    }

    /// Create a background-restricted Android adapter for planning/tests.
    pub fn background_restricted(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            foreground_service: false,
        }
    }

    fn resolve_key(&self, key: &str) -> Result<PathBuf, NetError> {
        if key.trim().is_empty() {
            return Err(storage_error(
                &self.data_dir,
                "storage key must not be empty".to_string(),
            ));
        }

        let path = Path::new(key);
        if path.is_absolute() {
            return Err(storage_error(
                path,
                "Android storage keys must be app-private relative paths".to_string(),
            ));
        }

        let mut clean = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Normal(segment) => clean.push(segment),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(storage_error(
                        path,
                        "Android storage key escapes the app-private data directory".to_string(),
                    ));
                }
            }
        }
        if clean.as_os_str().is_empty() {
            return Err(storage_error(
                path,
                "Android storage key must resolve to a file".to_string(),
            ));
        }
        Ok(self.data_dir.join(clean))
    }

    fn validate_ancestors(&self, path: &Path, create: bool) -> Result<(), NetError> {
        ensure_directory(&self.data_dir, create)?;
        let parent = path.parent().unwrap_or(&self.data_dir);
        let relative = parent.strip_prefix(&self.data_dir).map_err(|_| {
            storage_error(
                path,
                "Android storage path escaped the app-private data directory".to_string(),
            )
        })?;
        let mut current = self.data_dir.clone();
        for component in relative.components() {
            let Component::Normal(segment) = component else {
                return Err(storage_error(
                    path,
                    "Android storage parent contains an invalid path component".to_string(),
                ));
            };
            current.push(segment);
            ensure_directory(&current, create)?;
        }
        Ok(())
    }
}

impl NodeStorage for AndroidPlatformRuntime {
    fn storage_kind(&self) -> &'static str {
        "android_app_private_fs"
    }

    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, NetError> {
        let path = self.resolve_key(key)?;
        self.validate_ancestors(&path, false)?;
        read_regular_file(&path, false)
    }

    fn read_secret(&self, key: &str) -> Result<Option<Vec<u8>>, NetError> {
        let path = self.resolve_key(key)?;
        self.validate_ancestors(&path, false)?;
        read_regular_file(&path, true)
    }

    fn write_secret(&self, key: &str, value: &[u8]) -> Result<(), NetError> {
        let path = self.resolve_key(key)?;
        self.validate_ancestors(&path, true)?;
        if write_secret_create_new(&path, value)? {
            Ok(())
        } else {
            Err(storage_error(
                &path,
                "refusing to overwrite existing Android identity material".to_string(),
            ))
        }
    }

    fn write_secret_if_absent(&self, key: &str, value: &[u8]) -> Result<bool, NetError> {
        let path = self.resolve_key(key)?;
        self.validate_ancestors(&path, true)?;
        write_secret_create_new(&path, value)
    }

    fn write_public(&self, key: &str, value: &[u8]) -> Result<(), NetError> {
        let path = self.resolve_key(key)?;
        self.validate_ancestors(&path, true)?;
        reject_symlink_if_present(&path)?;
        fs::write(&path, value).map_err(|err| storage_error(&path, err.to_string()))
    }

    fn delete(&self, key: &str) -> Result<(), NetError> {
        let path = self.resolve_key(key)?;
        self.validate_ancestors(&path, false)?;
        reject_symlink_if_present(&path)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
            Err(err) => Err(storage_error(&path, err.to_string())),
        }
    }
}

impl PlatformRuntime for AndroidPlatformRuntime {
    fn runtime_name(&self) -> &'static str {
        if self.foreground_service {
            "android_foreground_service"
        } else {
            "android_background_restricted"
        }
    }

    fn platform_kind(&self) -> PlatformKind {
        PlatformKind::Android
    }

    fn default_data_dir(&self) -> Option<PathBuf> {
        Some(self.data_dir.clone())
    }

    fn can_listen_tcp(&self) -> bool {
        self.foreground_service
    }

    fn can_listen_quic(&self) -> bool {
        self.foreground_service
    }

    fn can_accept_inbound(&self) -> Option<bool> {
        if self.foreground_service {
            None
        } else {
            Some(false)
        }
    }

    fn is_battery_sensitive(&self) -> bool {
        true
    }

    fn is_background_restricted(&self) -> bool {
        !self.foreground_service
    }
}

fn read_regular_file(path: &Path, secret: bool) -> Result<Option<Vec<u8>>, NetError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(storage_error(path, err.to_string())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(storage_error(
            path,
            "refusing to read Android node state through a symlink or non-regular file".to_string(),
        ));
    }
    if secret {
        harden_secret_permissions(path, &metadata)?;
    }
    fs::read(path)
        .map(Some)
        .map_err(|err| storage_error(path, err.to_string()))
}

fn write_secret_create_new(path: &Path, value: &[u8]) -> Result<bool, NetError> {
    reject_symlink_if_present(path)?;

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(err) => return Err(storage_error(path, err.to_string())),
    };
    let metadata = file
        .metadata()
        .map_err(|err| storage_error(path, err.to_string()))?;
    if let Err(err) = harden_secret_permissions(path, &metadata) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(err);
    }
    if let Err(err) = file.write_all(value).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(storage_error(path, err.to_string()));
    }
    Ok(true)
}

fn ensure_directory(path: &Path, create: bool) -> Result<(), NetError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(storage_error(
                path,
                "Android storage ancestor is a symlink or non-directory".to_string(),
            ))
        }
        Ok(_) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound && create => {
            fs::create_dir(path).map_err(|err| storage_error(path, err.to_string()))
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(storage_error(path, err.to_string())),
    }
}

fn reject_symlink_if_present(path: &Path) -> Result<(), NetError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(storage_error(
            path,
            "refusing to access Android node state through a symlink".to_string(),
        )),
        Ok(_) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(storage_error(path, err.to_string())),
    }
}

#[cfg(unix)]
fn harden_secret_permissions(path: &Path, metadata: &fs::Metadata) -> Result<(), NetError> {
    use std::os::unix::fs::PermissionsExt;

    if metadata.permissions().mode() & 0o077 != 0 {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|err| storage_error(path, err.to_string()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn harden_secret_permissions(_path: &Path, _metadata: &fs::Metadata) -> Result<(), NetError> {
    Ok(())
}

fn storage_error(path: &Path, reason: String) -> NetError {
    NetError::Config {
        path: path.display().to_string(),
        reason,
    }
}
