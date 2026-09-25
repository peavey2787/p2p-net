//! Low-level libp2p stack: `NetworkBehaviour`, swarm transport build, and discovery hooks.

mod behaviour;
mod dcutr;
mod discovery;
#[cfg(not(target_arch = "wasm32"))]
mod dns_transport;
mod external_addresses;
mod transport;

pub use behaviour::*;
use dcutr::DcutrBehaviour;
pub use discovery::*;
pub use external_addresses::*;
pub use transport::*;

pub(crate) fn allow_dcutr_peer(swarm: &mut libp2p::Swarm<MeshBehaviour>, peer: libp2p::PeerId) {
    if let Some(dcutr) = swarm.behaviour_mut().dcutr.as_mut() {
        dcutr.allow_peer(peer);
    }
}

/// Pre-authorize DCUtR for peers the node already knows through a trusted
/// discovery source (manual, cache, DHT, LAN, or rendezvous).
pub(crate) fn allow_dcutr_for_known_peers(
    swarm: &mut libp2p::Swarm<MeshBehaviour>,
    peer_book: &crate::connectivity::peer_book::PeerBook,
) {
    use crate::api::PeerSource;
    for record in peer_book.records() {
        let trusted_source = record.sources.iter().any(|source| {
            matches!(
                source,
                PeerSource::Manual
                    | PeerSource::PeerCache
                    | PeerSource::DhtProvider
                    | PeerSource::LanDiscovery
                    | PeerSource::Rendezvous
                    | PeerSource::PublicRendezvous
            )
        });
        if !record.namespaces.is_empty() || trusted_source {
            allow_dcutr_peer(swarm, record.peer_id);
        }
    }
}
