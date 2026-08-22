//! Desktop platform runtime and filesystem storage implementation.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
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
        read_regular_file(&self.resolve_path(key), false)
    }

    fn read_secret(&self, key: &str) -> Result<Option<Vec<u8>>, NetError> {
        read_regular_file(&self.resolve_path(key), true)
    }

    fn write_secret(&self, key: &str, value: &[u8]) -> Result<(), NetError> {
        write_secret_create_new(&self.resolve_path(key), value).and_then(|created| {
            if created {
                Ok(())
            } else {
                Err(storage_error(
                    &self.resolve_path(key),
                    "refusing to overwrite existing secret material".to_string(),
                ))
            }
        })
    }

    fn write_secret_if_absent(&self, key: &str, value: &[u8]) -> Result<bool, NetError> {
        write_secret_create_new(&self.resolve_path(key), value)
    }

    fn write_public(&self, key: &str, value: &[u8]) -> Result<(), NetError> {
        write_public_bytes(&self.resolve_path(key), value)
    }

    fn delete(&self, key: &str) -> Result<(), NetError> {
        let path = self.resolve_path(key);
        reject_symlink_if_present(&path)?;
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

fn read_regular_file(path: &Path, secret: bool) -> Result<Option<Vec<u8>>, NetError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(storage_error(path, err.to_string())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(storage_error(
            path,
            "refusing to read node state through a symlink or non-regular file".to_string(),
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
    ensure_parent(path)?;
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

fn write_public_bytes(path: &Path, value: &[u8]) -> Result<(), NetError> {
    ensure_parent(path)?;
    reject_symlink_if_present(path)?;
    fs::write(path, value).map_err(|err| storage_error(path, err.to_string()))
}

fn ensure_parent(path: &Path) -> Result<(), NetError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| storage_error(parent, err.to_string()))?;
        }
    }
    Ok(())
}

fn reject_symlink_if_present(path: &Path) -> Result<(), NetError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(storage_error(
            path,
            "refusing to access node state through a symlink".to_string(),
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

#[cfg(windows)]
fn harden_secret_permissions(path: &Path, _metadata: &fs::Metadata) -> Result<(), NetError> {
    use std::process::Command;

    let output = Command::new("whoami")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .map_err(|err| {
            storage_error(
                path,
                format!("failed to resolve Windows account SID: {err}"),
            )
        })?;
    if !output.status.success() {
        return Err(storage_error(
            path,
            "failed to resolve Windows account SID with whoami".to_string(),
        ));
    }
    let line = String::from_utf8(output.stdout)
        .map_err(|err| storage_error(path, format!("whoami returned non-UTF-8 output: {err}")))?;
    let sid = line
        .split(',')
        .nth(1)
        .map(|value| value.trim().trim_matches('"'))
        .filter(|value| value.starts_with("S-"))
        .ok_or_else(|| storage_error(path, "could not parse Windows account SID".to_string()))?;
    let grant = format!("*{sid}:(F)");

    let grant_status = Command::new("icacls")
        .arg(path)
        .args(["/grant:r", &grant])
        .status()
        .map_err(|err| storage_error(path, format!("failed to execute icacls: {err}")))?;
    if !grant_status.success() {
        return Err(storage_error(
            path,
            "icacls failed to grant the current account exclusive secret access".to_string(),
        ));
    }

    let inheritance_status = Command::new("icacls")
        .arg(path)
        .arg("/inheritance:r")
        .status()
        .map_err(|err| storage_error(path, format!("failed to execute icacls: {err}")))?;
    if !inheritance_status.success() {
        return Err(storage_error(
            path,
            "icacls failed to remove inherited ACLs from secret material".to_string(),
        ));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn harden_secret_permissions(_path: &Path, _metadata: &fs::Metadata) -> Result<(), NetError> {
    Ok(())
}

fn storage_error(path: &Path, reason: String) -> NetError {
    NetError::Config {
        path: path.display().to_string(),
        reason,
    }
}
