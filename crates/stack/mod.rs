//! Low-level libp2p stack: `NetworkBehaviour`, swarm transport build, and discovery hooks.

mod application_keep_alive;
mod behaviour;
mod dcutr;
mod discovery;
mod dns_transport;
mod external_addresses;
mod transport;

pub(crate) use application_keep_alive::ApplicationKeepAlive;
pub use behaviour::*;
use dcutr::DcutrBehaviour;
pub use discovery::*;
pub use external_addresses::*;
pub use transport::*;

pub(crate) fn retain_application_peer(
    swarm: &mut libp2p::Swarm<MeshBehaviour>,
    peer: libp2p::PeerId,
) {
    swarm
        .behaviour_mut()
        .application_keep_alive
        .allow_peer(peer);
}

pub(crate) fn release_application_peer(
    swarm: &mut libp2p::Swarm<MeshBehaviour>,
    peer: &libp2p::PeerId,
) {
    swarm
        .behaviour_mut()
        .application_keep_alive
        .release_peer(peer);
}

pub(crate) fn allow_dcutr_peer(swarm: &mut libp2p::Swarm<MeshBehaviour>, peer: libp2p::PeerId) {
    if let Some(dcutr) = swarm.behaviour_mut().dcutr.as_mut() {
        dcutr.allow_peer(peer);
    }
}
