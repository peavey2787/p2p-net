use std::collections::BTreeSet;

use libp2p::core::{PeerRecord, SignedEnvelope};
use libp2p::identity::Keypair;
use libp2p::kad;
use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId, Swarm};

use crate::connectivity::addr::is_public_direct_addr;
use crate::connectivity::discovery::DiscoveryConfig;
use crate::connectivity::relay::{is_p2p_circuit_addr, relay_dial_addr_for_peer};
use crate::stack::MeshBehaviour;

use super::keys::dht_peer_address_record_key;
use super::DhtProviderState;

pub(super) const MAX_DHT_PEER_ADDRESS_RECORD_BYTES: usize = 32 * 1024;
const MAX_DHT_PEER_ADDRESS_COUNT: usize = 16;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DhtAddressPublishPlan {
    pub attempted_records: usize,
    pub addresses: usize,
    pub errors: Vec<String>,
}

pub fn publish_local_peer_address_records(
    swarm: &mut Swarm<MeshBehaviour>,
    local_key: &Keypair,
    network_id: u32,
    discovery_cfg: &DiscoveryConfig,
    state: &mut DhtProviderState,
) -> DhtAddressPublishPlan {
    let local_peer = PeerId::from(local_key.public());
    let addresses = collect_public_peer_addresses(swarm, local_peer);
    publish_local_peer_address_records_with_addresses(
        swarm,
        local_key,
        network_id,
        discovery_cfg,
        state,
        addresses,
    )
}

pub fn publish_local_peer_address_records_with_addresses(
    swarm: &mut Swarm<MeshBehaviour>,
    local_key: &Keypair,
    network_id: u32,
    discovery_cfg: &DiscoveryConfig,
    state: &mut DhtProviderState,
    addresses: impl IntoIterator<Item = Multiaddr>,
) -> DhtAddressPublishPlan {
    let local_peer = PeerId::from(local_key.public());
    let mut plan = DhtAddressPublishPlan::default();
    let mut unique = BTreeSet::new();
    for addr in addresses {
        if let Some(addr) = normalize_public_address_for_peer(addr, local_peer) {
            unique.insert(addr.to_string());
        }
        if unique.len() >= MAX_DHT_PEER_ADDRESS_COUNT {
            break;
        }
    }
    let addresses = unique
        .into_iter()
        .filter_map(|addr| addr.parse::<Multiaddr>().ok())
        .collect::<Vec<_>>();
    plan.addresses = addresses.len();
    if addresses.is_empty() || !discovery_cfg.dht.enabled || !discovery_cfg.dht.announce {
        return plan;
    }

    let peer_record = match PeerRecord::new(local_key, addresses) {
        Ok(record) => record,
        Err(err) => {
            plan.errors
                .push(format!("failed to sign local peer address record: {err}"));
            return plan;
        }
    };
    let encoded = peer_record.into_signed_envelope().into_protobuf_encoding();
    if encoded.len() > MAX_DHT_PEER_ADDRESS_RECORD_BYTES {
        plan.errors.push(format!(
            "signed local peer address record exceeds {} byte bound",
            MAX_DHT_PEER_ADDRESS_RECORD_BYTES
        ));
        return plan;
    }

    let namespaces = match discovery_cfg.rendezvous_namespaces(network_id) {
        Ok(namespaces) => namespaces,
        Err(err) => {
            plan.errors.push(err.to_string());
            return plan;
        }
    };
    for namespace in namespaces
        .into_iter()
        .take(discovery_cfg.dht.max_namespaces_per_refresh)
    {
        let key = dht_peer_address_record_key(&namespace, &local_peer);
        let fingerprint = blake3::hash(&encoded).to_hex().to_string();
        if !state.should_publish_address_record(
            &namespace,
            &fingerprint,
            discovery_cfg.dht.refresh_interval_secs,
        ) {
            continue;
        }
        let record = kad::Record::new(key, encoded.clone());
        match swarm
            .behaviour_mut()
            .kademlia
            .put_record(record, kad::Quorum::One)
        {
            Ok(query_id) => {
                state.track_put_address_record(query_id, namespace.clone(), fingerprint.clone());
                plan.attempted_records = plan.attempted_records.saturating_add(1);
            }
            Err(err) => plan.errors.push(format!(
                "dht peer address record publish failed namespace={namespace}: {err}"
            )),
        }
    }
    plan
}

pub(crate) fn start_peer_address_record_lookup(
    swarm: &mut Swarm<MeshBehaviour>,
    state: &mut DhtProviderState,
    peer: PeerId,
    namespace: &str,
) -> bool {
    if !state.should_lookup_address_record(peer, namespace) {
        return false;
    }
    let key = dht_peer_address_record_key(namespace, &peer);
    let id = swarm.behaviour_mut().kademlia.get_record(key);
    state.track_get_address_record(id, peer, namespace.to_string());
    true
}

pub(crate) fn decode_peer_address_record(
    bytes: &[u8],
    expected_peer: PeerId,
) -> Result<Vec<Multiaddr>, String> {
    if bytes.len() > MAX_DHT_PEER_ADDRESS_RECORD_BYTES {
        return Err("peer address record exceeds size bound".to_string());
    }
    let envelope = SignedEnvelope::from_protobuf_encoding(bytes)
        .map_err(|err| format!("invalid signed peer address envelope: {err}"))?;
    let record = PeerRecord::from_signed_envelope(envelope)
        .map_err(|err| format!("invalid signed peer address record: {err}"))?;
    if record.peer_id() != expected_peer {
        return Err(format!(
            "signed peer address record identity mismatch expected={expected_peer} got={}",
            record.peer_id()
        ));
    }
    let mut addresses = Vec::new();
    for addr in record.addresses().iter().take(MAX_DHT_PEER_ADDRESS_COUNT) {
        if let Some(addr) = normalize_public_address_for_peer(addr.clone(), expected_peer) {
            if !addresses.contains(&addr) {
                addresses.push(addr);
            }
        }
    }
    if addresses.is_empty() {
        return Err("signed peer address record contains no dialable public address".to_string());
    }
    Ok(addresses)
}

fn collect_public_peer_addresses(
    swarm: &Swarm<MeshBehaviour>,
    local_peer: PeerId,
) -> Vec<Multiaddr> {
    let mut addresses = Vec::new();
    for addr in swarm.external_addresses().chain(swarm.listeners()) {
        if let Some(addr) = normalize_public_address_for_peer(addr.clone(), local_peer) {
            if !addresses.contains(&addr) {
                addresses.push(addr);
            }
        }
        if addresses.len() >= MAX_DHT_PEER_ADDRESS_COUNT {
            break;
        }
    }
    addresses
}

fn normalize_public_address_for_peer(addr: Multiaddr, peer: PeerId) -> Option<Multiaddr> {
    if is_p2p_circuit_addr(&addr) {
        if !is_public_direct_addr(&addr) {
            return None;
        }
        return relay_dial_addr_for_peer(&addr, peer);
    }
    if !is_public_direct_addr(&addr) {
        return None;
    }
    let mut existing_peer = None;
    for protocol in addr.iter() {
        if let Protocol::P2p(found) = protocol {
            existing_peer = Some(found);
        }
    }
    match existing_peer {
        Some(found) if found == peer => Some(addr),
        Some(_) => None,
        None => Some(addr.with(Protocol::P2p(peer))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_record_rejects_identity_substitution() {
        let key = Keypair::generate_ed25519();
        let peer = PeerId::from(key.public());
        let addr: Multiaddr = format!("/ip4/8.8.8.8/tcp/4001/p2p/{peer}").parse().unwrap();
        let record = PeerRecord::new(&key, vec![addr]).unwrap();
        let bytes = record.into_signed_envelope().into_protobuf_encoding();
        assert!(decode_peer_address_record(&bytes, PeerId::random()).is_err());
    }

    #[test]
    fn relay_reservation_is_published_as_target_bound_route() {
        let relay = PeerId::random();
        let peer = PeerId::random();
        let reservation: Multiaddr = format!("/ip4/8.8.8.8/tcp/4001/p2p/{relay}/p2p-circuit")
            .parse()
            .unwrap();
        let result = normalize_public_address_for_peer(reservation, peer).unwrap();
        assert!(result
            .to_string()
            .ends_with(&format!("/p2p-circuit/p2p/{peer}")));
    }
}
