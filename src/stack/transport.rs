use std::time::Duration;

use libp2p::identity::Keypair;
use libp2p::swarm::Swarm;
use libp2p::{noise, tcp, yamux, SwarmBuilder};

use super::behaviour::{build_behaviour, MeshBehaviour};
use crate::common::error::NetError;
use crate::NodeConfig;

#[derive(Debug, Clone)]
pub struct TransportPlan {
    pub active: Vec<&'static str>,
}

pub async fn build_swarm(
    local_key: Keypair,
    cfg: &NodeConfig,
) -> Result<(Swarm<MeshBehaviour>, TransportPlan), NetError> {
    let local_peer = libp2p::PeerId::from(local_key.public());
    let relay_cfg = cfg.relay.clone();

    let builder = SwarmBuilder::with_existing_identity(local_key)
        .with_tokio()
        .with_tcp(
            tcp::Config::default().nodelay(true),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_quic()
        .with_dns()
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_websocket(noise::Config::new, yamux::Config::default)
        .await
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .map_err(|e| NetError::Build(e.to_string()))?;

    // Only report transports/capabilities that are actually configured in this stack.
    // Do not list WebRTC/WebTransport until they are implemented as real listeners.
    let mut active = vec![
        "quic",
        "tcp",
        "websocket",
        "relay-client",
        "autonat",
        "dcutr",
    ];
    #[cfg(feature = "dns")]
    active.push("dns");
    if !cfg.discovery.bootstrap_seed_peers.is_empty() {
        active.push("bootstrap-seeds");
    }
    if !cfg.discovery.rendezvous_peers.is_empty() {
        active.push("rendezvous-peers");
    }
    if cfg.discovery.rendezvous.client_enabled {
        active.push("rendezvous-client");
    }
    if cfg.discovery.rendezvous.server_enabled {
        active.push("rendezvous-server");
    }
    if cfg.connection_limits.enabled {
        active.push("connection-limits");
    }
    if cfg.reserve_configured_relays && !cfg.relay_peers.is_empty() {
        active.push("relay-reservations");
    }
    if relay_cfg.enabled {
        active.push("relay-server");
        if cfg.mediator.enabled {
            active.push("mediator");
        }
        active.push("relay-acl");
        if relay_cfg.schedule.enabled {
            active.push("relay-schedule");
        }
    }

    let mut swarm = builder
        .with_behaviour(|key, relay_behaviour| {
            build_behaviour(
                key,
                local_peer,
                relay_behaviour,
                cfg.network_id,
                &relay_cfg,
                &cfg.connection_limits,
                &cfg.discovery,
            )
        })
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    let listen_addrs = cfg.parsed_listen_addresses()?;
    for addr in &listen_addrs {
        swarm
            .listen_on(addr.clone())
            .map_err(|e| NetError::Listen {
                addr: addr.to_string(),
                reason: e.to_string(),
            })?;
    }

    Ok((swarm, TransportPlan { active }))
}
