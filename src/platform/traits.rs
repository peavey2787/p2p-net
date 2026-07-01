//! Platform abstraction traits for the shared P2P core.
//!
//! These traits keep filesystem, lifecycle, and listen-capability differences at
//! the edge of the crate so Android/iOS/Desktop shells do not need separate P2P
//! implementations.

use std::path::PathBuf;

use crate::common::error::NetError;
use crate::{EnvironmentConfig, PlatformKind};

/// Storage used by the node for identities, peer caches, and small secrets.
///
/// Keys are logical paths from `NodeConfig` such as `.p2p-net-identity-key` or
/// `.p2p-net-peer-cache.json`. Desktop implementations may map them directly to
/// filesystem paths. Mobile implementations can map the same keys into app
/// private storage, Keychain/Keystore-backed blobs, or encrypted containers.
pub trait NodeStorage: Send + Sync {
    fn storage_kind(&self) -> &'static str {
        "custom"
    }

    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, NetError>;
    fn write_secret(&self, key: &str, value: &[u8]) -> Result<(), NetError>;
    fn write_public(&self, key: &str, value: &[u8]) -> Result<(), NetError>;
    fn delete(&self, key: &str) -> Result<(), NetError>;
}

/// Runtime facts supplied by the embedding platform shell.
///
/// The core treats these as advisory hints for profile/capability resolution.
/// They should not contain UI-specific details.
pub trait PlatformRuntime: Send + Sync {
    fn runtime_name(&self) -> &'static str {
        "custom"
    }

    fn platform_kind(&self) -> PlatformKind;
    fn default_data_dir(&self) -> Option<PathBuf>;
    fn can_listen_tcp(&self) -> bool;
    fn can_listen_quic(&self) -> bool;
    fn can_accept_inbound(&self) -> Option<bool>;
    fn is_battery_sensitive(&self) -> bool;
    fn is_background_restricted(&self) -> bool;

    fn environment_config(&self) -> EnvironmentConfig {
        EnvironmentConfig {
            platform_hint: Some(self.platform_kind()),
            reachability_hint: None,
            nat_hint: None,
            can_listen_tcp: Some(self.can_listen_tcp()),
            can_listen_quic: Some(self.can_listen_quic()),
            can_accept_inbound: self.can_accept_inbound(),
            likely_cgnat: None,
            battery_sensitive: Some(self.is_battery_sensitive()),
            background_restricted: Some(self.is_background_restricted()),
        }
    }
}
