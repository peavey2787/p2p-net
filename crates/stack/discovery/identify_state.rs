use std::collections::{HashMap, VecDeque};

use libp2p::{Multiaddr, PeerId, Swarm};

use super::super::behaviour::MeshBehaviour;

const MAX_IDENTIFY_ADDRS_PER_PEER: usize = 8;
const MAX_IDENTIFY_ROUTING_PEERS: usize = 2_048;
const MAX_OBSERVED_LOCAL_ADDRS: usize = 32;

/// Bounded Identify address memory for Kademlia routing peers plus a small
/// deduplication set for addresses remote peers report for this node.
#[derive(Debug, Default)]
pub struct IdentifyAddressState {
    by_peer: HashMap<PeerId, VecDeque<Multiaddr>>,
    peer_order: VecDeque<PeerId>,
    observed_local_addrs: VecDeque<Multiaddr>,
}

impl IdentifyAddressState {
    pub(crate) fn record_observed_local_addr(&mut self, address: &Multiaddr) -> bool {
        if self.observed_local_addrs.iter().any(|known| known == address) {
            return false;
        }
        if self.observed_local_addrs.len() >= MAX_OBSERVED_LOCAL_ADDRS {
            let _ = self.observed_local_addrs.pop_front();
        }
        self.observed_local_addrs.push_back(address.clone());
        true
    }

    pub(super) fn record(
        &mut self,
        swarm: &mut Swarm<MeshBehaviour>,
        peer: PeerId,
        address: Multiaddr,
    ) {
        let is_new_peer = !self.by_peer.contains_key(&peer);
        let addresses = self.by_peer.entry(peer).or_default();
        if addresses.iter().any(|known| known == &address) {
            return;
        }
        addresses.push_back(address.clone());
        swarm.behaviour_mut().kademlia.add_address(&peer, address);

        while addresses.len() > MAX_IDENTIFY_ADDRS_PER_PEER {
            let Some(expired) = addresses.pop_front() else {
                break;
            };
            swarm
                .behaviour_mut()
                .kademlia
                .remove_address(&peer, &expired);
        }

        if is_new_peer {
            self.peer_order.push_back(peer);
        }
        while self.by_peer.len() > MAX_IDENTIFY_ROUTING_PEERS {
            let Some(expired_peer) = self.peer_order.pop_front() else {
                break;
            };
            let Some(expired_addresses) = self.by_peer.remove(&expired_peer) else {
                continue;
            };
            for expired in expired_addresses {
                swarm
                    .behaviour_mut()
                    .kademlia
                    .remove_address(&expired_peer, &expired);
            }
        }
    }
}
