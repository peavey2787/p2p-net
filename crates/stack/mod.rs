//! Low-level libp2p stack: `NetworkBehaviour`, swarm transport build, and discovery hooks.

mod application_keep_alive;
mod behaviour;
mod connection_limits;
mod dcutr;
mod discovery;
mod dns_transport;
mod external_addresses;
mod quic_transport;
mod tcp_transport;
mod transport;

pub(crate) use application_keep_alive::ApplicationKeepAlive;
pub use behaviour::*;
use connection_limits::PrioritizedConnectionLimits;
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

pub(crate) fn release_application_keep_alive(
    swarm: &mut libp2p::Swarm<MeshBehaviour>,
    peer: &libp2p::PeerId,
) {
    // A disconnected app peer remains an intended app peer. Keep its bounded
    // admission priority so a transient relay close cannot demote its reconnect
    // to the infrastructure ceiling. This does not initiate any reconnect or
    // override explicit user-disconnect suppression in the connection planner.
    swarm
        .behaviour_mut()
        .application_keep_alive
        .release_peer(peer);
}

pub(crate) fn allow_dcutr_peer(swarm: &mut libp2p::Swarm<MeshBehaviour>, peer: libp2p::PeerId) {
    swarm
        .behaviour_mut()
        .connection_limits
        .allow_application_peer(peer);
    if let Some(dcutr) = swarm.behaviour_mut().dcutr.as_mut() {
        dcutr.allow_peer(peer);
    }
}

#[cfg(test)]
mod reconnect_tests {
    use super::*;
    use libp2p::core::{transport::PortUse, ConnectedPoint, Endpoint};
    use libp2p::swarm::{
        behaviour::ConnectionEstablished, ConnectionId, FromSwarm, NetworkBehaviour,
    };

    #[tokio::test]
    async fn transient_disconnect_keeps_app_admission_headroom() {
        let mut config = crate::NodeConfig::default();
        config.listen_addresses = vec!["/ip4/127.0.0.1/tcp/0".into()];
        config.connection_limits.max_established_outgoing = Some(4);
        let resolved =
            crate::resolve_node_config(&config, &crate::EnvironmentReport::detect(&config))
                .unwrap();
        let (mut swarm, _) = build_swarm(
            libp2p::identity::Keypair::generate_ed25519(),
            &config,
            &resolved,
        )
        .await
        .unwrap();
        let addr: libp2p::Multiaddr = "/ip4/8.8.8.8/tcp/4001".parse().unwrap();
        let endpoint = ConnectedPoint::Dialer {
            address: addr.clone(),
            role_override: Endpoint::Dialer,
            port_use: PortUse::Reuse,
        };
        // Fill the infrastructure ceiling without opening real network dials.
        for n in 0..3 {
            swarm.behaviour_mut().connection_limits.on_swarm_event(
                FromSwarm::ConnectionEstablished(ConnectionEstablished {
                    peer_id: libp2p::PeerId::random(),
                    connection_id: ConnectionId::new_unchecked(n),
                    endpoint: &endpoint,
                    failed_addresses: &[],
                    other_established: 0,
                }),
            );
        }
        let app = libp2p::PeerId::random();
        allow_dcutr_peer(&mut swarm, app);
        retain_application_peer(&mut swarm, app);
        release_application_keep_alive(&mut swarm, &app);
        let limits = &mut swarm.behaviour_mut().connection_limits;
        assert!(limits
            .handle_established_outbound_connection(
                ConnectionId::new_unchecked(3),
                app,
                &addr,
                Endpoint::Dialer,
                PortUse::Reuse
            )
            .is_ok());
        assert!(limits
            .handle_established_outbound_connection(
                ConnectionId::new_unchecked(4),
                libp2p::PeerId::random(),
                &addr,
                Endpoint::Dialer,
                PortUse::Reuse
            )
            .is_err());
    }
}
