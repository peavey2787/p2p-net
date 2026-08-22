use super::*;
use super::keys::{
    dht_record_replica_key, DHT_PROVIDER_ANCHOR_PREFIX_BYTES, MULTIHASH_SHA2_256_CODE,
    SHA2_256_DIGEST_BYTES,
};
use sha2::{Digest, Sha256};

#[test]
fn provider_tracking_is_bounded() {
    let mut state = DhtProviderState::default();
    for idx in 0..MAX_TRACKED_PROVIDER_PEERS + 5 {
        state.record_provider_peer(PeerId::random(), format!("namespace-{idx}"));
    }

    assert_eq!(
        state.discovered_provider_peers.len(),
        MAX_TRACKED_PROVIDER_PEERS
    );
}

#[test]
fn auto_connect_tracking_is_bounded() {
    let mut state = DhtProviderState::default();
    for _ in 0..MAX_TRACKED_PROVIDER_PEERS + 5 {
        state.mark_auto_connect_waiting_for_addrs(PeerId::random());
        state.mark_auto_connect_attempted(PeerId::random());
    }

    assert_eq!(
        state.auto_connect_waiting_for_addrs.len(),
        MAX_TRACKED_PROVIDER_PEERS
    );
    assert_eq!(
        state.auto_connect_attempted_peers.len(),
        MAX_TRACKED_PROVIDER_PEERS
    );
    assert_eq!(state.auto_connect_retry.len(), MAX_TRACKED_PROVIDER_PEERS);
}

#[test]
fn disconnected_peer_can_retry_without_resetting_its_budget() {
    let mut state = DhtProviderState::default();
    let peer = PeerId::random();
    assert!(state.mark_auto_connect_attempted(peer));
    let retry_before = state
        .auto_connect_retry
        .get(&peer)
        .cloned()
        .expect("retry state");

    state.mark_auto_connect_disconnected(&peer);

    assert!(!state.auto_connect_attempted_peers.contains(&peer));
    assert!(!state.auto_connect_waiting_for_addrs.contains(&peer));
    assert_eq!(state.auto_connect_retry.get(&peer), Some(&retry_before));
}

#[test]
fn provider_keys_are_sha2_256_multihashes() {
    let key = dht_record_key("p2p-net/1/hydra-msg/abc").to_vec();
    let replica = dht_record_replica_key("p2p-net/1/hydra-msg/abc", 1).to_vec();

    assert_eq!(key.len(), 34);
    assert_eq!(key[0], MULTIHASH_SHA2_256_CODE);
    assert_eq!(key[1], SHA2_256_DIGEST_BYTES);
    assert_eq!(replica.len(), 34);
    assert_ne!(key, replica);
}

#[test]
fn public_provider_keys_share_a_bootstrap_anchor_prefix() {
    let config = DiscoveryConfig::default();
    let (_, anchor_text) = config.public_bootstrap.bootstrap_seed_peers[0]
        .rsplit_once("/p2p/")
        .expect("peer suffix");
    let anchor = anchor_text.parse::<PeerId>().expect("peer id");
    let keys = dht_provider_keys("p2p-net/1/test/private", &config);
    let target = Sha256::digest(anchor.to_bytes());
    let location = Sha256::digest(keys[0].1.to_vec());

    assert_eq!(
        location[..DHT_PROVIDER_ANCHOR_PREFIX_BYTES],
        target[..DHT_PROVIDER_ANCHOR_PREFIX_BYTES]
    );
}
