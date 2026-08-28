//! Low-level libp2p stack: `NetworkBehaviour`, swarm transport build, and discovery hooks.

mod behaviour;
mod dcutr;
mod discovery;
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
