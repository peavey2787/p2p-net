use libp2p::kad;
use libp2p::PeerId;
use sha2::{Digest, Sha256};

use crate::connectivity::discovery::DiscoveryConfig;

pub(super) const DHT_PROVIDER_ANCHOR_PREFIX_BYTES: usize = 2;
const DHT_PROVIDER_ANCHOR_MAX_ATTEMPTS: u32 = 1 << 20;
const DHT_PROVIDER_ANCHOR_CONTEXT: &str = "p2p-net.dht.provider.anchor.v1";
const DHT_PEER_ADDRESS_RECORD_CONTEXT: &str = "p2p-net.dht.peer-address.v1";
pub(super) const MULTIHASH_SHA2_256_CODE: u8 = 0x12;
pub(super) const SHA2_256_DIGEST_BYTES: u8 = 32;

pub fn dht_record_key(namespace: &str) -> kad::RecordKey {
    provider_multihash_key(namespace.as_bytes())
}

pub(super) fn dht_peer_address_record_key(namespace: &str, peer: &PeerId) -> kad::RecordKey {
    let mut material = Vec::with_capacity(
        DHT_PEER_ADDRESS_RECORD_CONTEXT.len() + namespace.len() + peer.to_bytes().len() + 2,
    );
    material.extend_from_slice(DHT_PEER_ADDRESS_RECORD_CONTEXT.as_bytes());
    material.push(0);
    material.extend_from_slice(namespace.as_bytes());
    material.push(0);
    material.extend_from_slice(&peer.to_bytes());
    provider_multihash_key(material)
}

pub(super) fn dht_record_replica_key(namespace: &str, replica: u8) -> kad::RecordKey {
    if replica == 0 {
        dht_record_key(namespace)
    } else {
        provider_multihash_key(format!("{namespace}/provider-replica/{replica}").as_bytes())
    }
}

fn dht_record_replica_tracking_key(namespace: &str, replica: u8) -> String {
    if replica == 0 {
        namespace.to_string()
    } else {
        format!("{namespace}:provider-replica:{replica}")
    }
}

pub(super) fn dht_provider_keys(
    namespace: &str,
    discovery_cfg: &DiscoveryConfig,
) -> Vec<(String, kad::RecordKey)> {
    let public_anchors = if discovery_cfg.public_bootstrap.mode.is_enabled() {
        discovery_cfg
            .public_bootstrap
            .bootstrap_seed_peers
            .iter()
            .filter_map(|addr| {
                addr.rsplit_once("/p2p/")
                    .and_then(|(_, peer)| peer.parse::<PeerId>().ok())
            })
            .take(discovery_cfg.dht.provider_key_replicas)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    if !public_anchors.is_empty() {
        return public_anchors
            .into_iter()
            .enumerate()
            .map(|(replica, anchor)| {
                (
                    format!("{namespace}:provider-anchor:{replica}"),
                    anchored_provider_key(namespace, &anchor, replica as u8),
                )
            })
            .collect();
    }

    (0..discovery_cfg.dht.provider_key_replicas)
        .map(|replica| {
            let replica = u8::try_from(replica).expect("validated provider key replica count");
            (
                dht_record_replica_tracking_key(namespace, replica),
                dht_record_replica_key(namespace, replica),
            )
        })
        .collect()
}

fn anchored_provider_key(namespace: &str, anchor: &PeerId, replica: u8) -> kad::RecordKey {
    let anchor_bytes = anchor.to_bytes();
    let target = Sha256::digest(&anchor_bytes);

    // The counter is the only changing input. Clone the pre-hashed SHA-256
    // state instead of rebuilding and hashing the common context/namespace/
    // anchor prefix for every search attempt.
    let mut base = Sha256::new();
    base.update(DHT_PROVIDER_ANCHOR_CONTEXT.as_bytes());
    base.update([0]);
    base.update(namespace.as_bytes());
    base.update([replica]);
    base.update(&anchor_bytes);

    for counter in 0..DHT_PROVIDER_ANCHOR_MAX_ATTEMPTS {
        let mut attempt = base.clone();
        attempt.update(counter.to_be_bytes());
        let digest = attempt.finalize();
        let mut candidate = [0_u8; 34];
        candidate[0] = MULTIHASH_SHA2_256_CODE;
        candidate[1] = SHA2_256_DIGEST_BYTES;
        candidate[2..].copy_from_slice(&digest);
        let location = Sha256::digest(candidate);
        if location[..DHT_PROVIDER_ANCHOR_PREFIX_BYTES]
            == target[..DHT_PROVIDER_ANCHOR_PREFIX_BYTES]
        {
            return kad::RecordKey::new(&candidate);
        }
    }
    dht_record_replica_key(namespace, replica)
}

fn provider_multihash_key(material: impl AsRef<[u8]>) -> kad::RecordKey {
    let digest = Sha256::digest(material.as_ref());
    let mut key = Vec::with_capacity(2 + digest.len());
    key.push(MULTIHASH_SHA2_256_CODE);
    key.push(SHA2_256_DIGEST_BYTES);
    key.extend_from_slice(&digest);
    kad::RecordKey::new(&key)
}
