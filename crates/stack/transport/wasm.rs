use libp2p::{noise, webrtc_websys, websocket_websys, yamux, SwarmBuilder, Transport};

use super::{swarm_idle_connection_timeout, transport_plan, TransportPlan};
use crate::common::error::NetError;
use crate::stack::behaviour::{build_behaviour, BehaviourBuildContext, MeshBehaviour};
use crate::{NodeConfig, ResolvedNodeConfig};

pub(super) async fn build_swarm(
    local_key: libp2p::identity::Keypair,
    cfg: &NodeConfig,
    resolved_cfg: &ResolvedNodeConfig,
) -> Result<(libp2p::Swarm<MeshBehaviour>, TransportPlan), NetError> {
    let local_peer = libp2p::PeerId::from(local_key.public());
    let relay_cfg = cfg.relay.clone();
    let (relay_transport, relay_behaviour) = libp2p_relay::client::new(local_peer);

    let builder = SwarmBuilder::with_existing_identity(local_key)
        .with_wasm_bindgen()
        .with_other_transport(|key| webrtc_websys::Transport::new(webrtc_websys::Config::new(key)))
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_other_transport(|key| {
            let noise = noise::Config::new(key).map_err(
                |err| -> Box<dyn std::error::Error + Send + Sync + 'static> { Box::new(err) },
            )?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync + 'static>>(
                websocket_websys::Transport::default()
                    .upgrade(libp2p::core::upgrade::Version::V1Lazy)
                    .authenticate(noise)
                    .multiplex(yamux::Config::default()),
            )
        })
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_other_transport(move |key| {
            // Circuit Relay v2 client from p2p-net-relay (metered relay);
            // same Noise + Yamux upgrade the libp2p relay builder applies.
            let noise = noise::Config::new(key).map_err(
                |err| -> Box<dyn std::error::Error + Send + Sync + 'static> { Box::new(err) },
            )?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync + 'static>>(
                relay_transport
                    .upgrade(libp2p::core::upgrade::Version::V1Lazy)
                    .authenticate(noise)
                    .multiplex(yamux::Config::default()),
            )
        })
        .map_err(|e| NetError::Build(e.to_string()))?;

    let swarm = builder
        .with_behaviour(move |key| {
            build_behaviour(BehaviourBuildContext {
                local_key: key,
                local_peer,
                relay_behaviour,
                network_id: cfg.network_id,
                gossipsub_heartbeat_interval_secs: cfg.gossipsub_heartbeat_interval_secs,
                ping_interval_secs: cfg.ping_interval_secs,
                relay_cfg: &relay_cfg,
                connection_limits_cfg: &cfg.connection_limits,
                discovery_cfg: &cfg.discovery,
                resolved_cfg,
            })
        })
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_swarm_config(|c| {
            c.with_idle_connection_timeout(swarm_idle_connection_timeout(cfg.ping_interval_secs))
        })
        .build();

    // Browsers cannot accept raw TCP/QUIC listeners. WebRTC/WSS/WebTransport are
    // dial transports here; inbound browser connectivity is provided through a
    // Circuit Relay reservation and remains invisible to applications.
    Ok((swarm, transport_plan(cfg, resolved_cfg)))
}
