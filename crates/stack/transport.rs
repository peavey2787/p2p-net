use std::time::Duration;

use libp2p::identity::Keypair;
use libp2p::swarm::Swarm;
use libp2p::{noise, tcp, yamux, SwarmBuilder};

use super::behaviour::{build_behaviour, BehaviourBuildContext, MeshBehaviour};
use crate::common::error::NetError;
use crate::{NodeConfig, ResolvedNodeConfig};

#[derive(Debug, Clone)]
pub struct TransportPlan {
    pub active: Vec<&'static str>,
}

pub async fn build_swarm(
    local_key: Keypair,
    cfg: &NodeConfig,
    resolved_cfg: &ResolvedNodeConfig,
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

    // Only report transports/capabilities that are actually enabled by the
    // resolved profile policy. Do not list WebRTC/WebTransport until they are
    // implemented as real listeners.
    let behaviour_policy = &resolved_cfg.enabled_behaviours;
    let mut active = vec!["quic", "tcp", "websocket"];
    if behaviour_policy.gossipsub {
        active.push("gossipsub");
    }
    if behaviour_policy.kademlia_server {
        active.push("kademlia-server");
    } else if behaviour_policy.kademlia_client {
        active.push("kademlia-client");
    }
    if behaviour_policy.relay_client {
        active.push("relay-client");
    }
    if behaviour_policy.autonat {
        active.push("autonat");
    }
    if behaviour_policy.dcutr {
        active.push("dcutr");
    }
    #[cfg(feature = "dns")]
    active.push("dns");
    if !cfg.discovery.bootstrap_seed_peers.is_empty() {
        active.push("bootstrap-seeds");
    }
    if !cfg.discovery.rendezvous_peers.is_empty() {
        active.push("rendezvous-peers");
    }
    if behaviour_policy.rendezvous_client && cfg.discovery.rendezvous.client_enabled {
        active.push("rendezvous-client");
    }
    if behaviour_policy.rendezvous_server && cfg.discovery.rendezvous.server_enabled {
        active.push("rendezvous-server");
    }
    if cfg.connection_limits.enabled {
        active.push("connection-limits");
    }
    if resolved_cfg.should_reserve_configured_relays
        || (resolved_cfg.should_reserve_selected_relays
            && cfg.discovery.public_bootstrap.has_relay_candidates())
    {
        active.push("relay-reservations");
    }
    if behaviour_policy.relay_server && relay_cfg.enabled {
        active.push("relay-server");
        if resolved_cfg.mediator_enabled {
            active.push("mediator");
        }
        active.push("relay-acl");
        if relay_cfg.schedule.enabled {
            active.push("relay-schedule");
        }
    }

    let mut swarm = builder
        .with_behaviour(|key, relay_behaviour| {
            build_behaviour(BehaviourBuildContext {
                local_key: key,
                local_peer,
                relay_behaviour,
                network_id: cfg.network_id,
                relay_cfg: &relay_cfg,
                connection_limits_cfg: &cfg.connection_limits,
                discovery_cfg: &cfg.discovery,
                resolved_cfg,
            })
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
