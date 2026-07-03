use std::path::Path;

use libp2p::identity::Keypair;

use crate::common::error::NetError;
use crate::platform::{DesktopPlatformRuntime, NodeStorage};

/// Load a stable libp2p node identity key from the default desktop storage, or
/// create and persist one on first run.
pub fn load_or_create_identity_key(path: impl AsRef<Path>) -> Result<Keypair, NetError> {
    let path_s = path.as_ref().to_string_lossy().to_string();
    load_or_create_identity_key_with_storage(&path_s, &DesktopPlatformRuntime::default())
}

/// Load a stable libp2p node identity key from an abstract storage backend, or
/// create and persist one on first run. Mobile/Desktop embedders should prefer
/// this function so key persistence can live in platform-owned storage.
pub fn load_or_create_identity_key_with_storage(
    key: &str,
    storage: &dyn NodeStorage,
) -> Result<Keypair, NetError> {
    match storage.read(key)? {
        Some(raw) => decode_identity_key(key, decode_utf8(key, raw)?.trim()),
        None => {
            let keypair = Keypair::generate_ed25519();
            persist_identity_key(key, storage, &keypair)?;
            Ok(keypair)
        }
    }
}

fn decode_utf8(path: &str, raw: Vec<u8>) -> Result<String, NetError> {
    String::from_utf8(raw).map_err(|err| NetError::Identity {
        path: path.to_string(),
        reason: format!("identity key is not valid UTF-8: {err}"),
    })
}

fn decode_identity_key(path: &str, encoded: &str) -> Result<Keypair, NetError> {
    let bytes = hex::decode(encoded).map_err(|err| NetError::Identity {
        path: path.to_string(),
        reason: format!("identity key is not valid hex: {err}"),
    })?;
    Keypair::from_protobuf_encoding(&bytes).map_err(|err| NetError::Identity {
        path: path.to_string(),
        reason: format!("identity key protobuf decode failed: {err}"),
    })
}

fn persist_identity_key(
    path: &str,
    storage: &dyn NodeStorage,
    key: &Keypair,
) -> Result<(), NetError> {
    let bytes = key
        .to_protobuf_encoding()
        .map_err(|err| NetError::Identity {
            path: path.to_string(),
            reason: format!("identity key protobuf encode failed: {err}"),
        })?;
    storage.write_secret(path, hex::encode(bytes).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::MemoryNodeStorage;

    #[test]
    fn persistent_identity_reuses_same_peer_id() {
        let path = format!("p2p-net-test-key-{}.hex", libp2p::PeerId::random());
        let storage = MemoryNodeStorage::new();
        let first =
            load_or_create_identity_key_with_storage(&path, &storage).expect("create identity");
        let second =
            load_or_create_identity_key_with_storage(&path, &storage).expect("reload identity");
        assert_eq!(
            libp2p::PeerId::from(first.public()),
            libp2p::PeerId::from(second.public())
        );
    }
}
