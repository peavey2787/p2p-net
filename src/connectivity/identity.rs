use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use libp2p::identity::Keypair;

use crate::common::error::NetError;

/// Load a stable libp2p node identity key from disk, or create and persist one
/// on first run. The file is hex-encoded protobuf key material so it remains
/// portable across platforms and obvious to back up.
pub fn load_or_create_identity_key(path: impl AsRef<Path>) -> Result<Keypair, NetError> {
    let path = path.as_ref();
    match fs::read_to_string(path) {
        Ok(raw) => decode_identity_key(path, raw.trim()),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            let key = Keypair::generate_ed25519();
            persist_identity_key(path, &key)?;
            Ok(key)
        }
        Err(err) => Err(NetError::Identity {
            path: path.display().to_string(),
            reason: err.to_string(),
        }),
    }
}

fn decode_identity_key(path: &Path, encoded: &str) -> Result<Keypair, NetError> {
    let bytes = hex::decode(encoded).map_err(|err| NetError::Identity {
        path: path.display().to_string(),
        reason: format!("identity key is not valid hex: {err}"),
    })?;
    Keypair::from_protobuf_encoding(&bytes).map_err(|err| NetError::Identity {
        path: path.display().to_string(),
        reason: format!("identity key protobuf decode failed: {err}"),
    })
}

fn persist_identity_key(path: &Path, key: &Keypair) -> Result<(), NetError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| NetError::Identity {
                path: parent.display().to_string(),
                reason: err.to_string(),
            })?;
        }
    }

    let bytes = key
        .to_protobuf_encoding()
        .map_err(|err| NetError::Identity {
            path: path.display().to_string(),
            reason: format!("identity key protobuf encode failed: {err}"),
        })?;
    fs::write(path, hex::encode(bytes)).map_err(|err| NetError::Identity {
        path: path.display().to_string(),
        reason: err.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_identity_reuses_same_peer_id() {
        let path =
            std::env::temp_dir().join(format!("p2p-net-test-key-{}.hex", libp2p::PeerId::random()));
        let first = load_or_create_identity_key(&path).expect("create identity");
        let second = load_or_create_identity_key(&path).expect("reload identity");
        let _ = fs::remove_file(path);
        assert_eq!(
            libp2p::PeerId::from(first.public()),
            libp2p::PeerId::from(second.public())
        );
    }
}
