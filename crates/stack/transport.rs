use std::time::Duration;

use libp2p::identity::Keypair;
use libp2p::swarm::Swarm;
use libp2p::{noise, tcp, yamux, SwarmBuilder};
use libp2p_webrtc::tokio::{Certificate as WebRtcCertificate, Transport as WebRtcTransport};

use super::behaviour::{build_behaviour, BehaviourBuildContext, MeshBehaviour};
use crate::common::error::NetError;
use crate::connectivity::webrtc::WEBRTC_DIRECT_TRANSPORT;
use crate::{NodeConfig, ResolvedNodeConfig};

// Keep idle expiry safely beyond the configured Ping cadence. Otherwise a
// low-frequency keepalive policy can continuously tear down healthy idle
// connections just before their next ping, causing rediscovery/redial churn.
const MIN_SWARM_IDLE_CONNECTION_TIMEOUT_SECS: u64 = 30;
const SWARM_IDLE_TIMEOUT_PING_MULTIPLIER: u64 = 2;

fn swarm_idle_connection_timeout(ping_interval_secs: u64) -> Duration {
    Duration::from_secs(
        ping_interval_secs
            .saturating_mul(SWARM_IDLE_TIMEOUT_PING_MULTIPLIER)
            .max(MIN_SWARM_IDLE_CONNECTION_TIMEOUT_SECS),
    )
}

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
        .with_other_transport(|key| {
            let certificate = WebRtcCertificate::generate(&mut rand::thread_rng()).map_err(
                |err| -> Box<dyn std::error::Error + Send + Sync + 'static> { Box::new(err) },
            )?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync + 'static>>(WebRtcTransport::new(
                key.clone(),
                certificate,
            ))
        })
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_dns()
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_websocket(noise::Config::new, yamux::Config::default)
        .await
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .map_err(|e| NetError::Build(e.to_string()))?;

    // Only report transports/capabilities that are actually enabled by the
    // resolved profile policy. WebRTC-direct is a real swarm transport here,
    // so it shares the same peer routing and connection state as TCP/QUIC/WS.
    let behaviour_policy = &resolved_cfg.enabled_behaviours;
    let mut active = Vec::new();
    if cfg.listeners.quic {
        active.push("quic");
    }
    if cfg.listeners.tcp {
        active.push("tcp");
    }
    if cfg.listeners.websocket {
        active.push("websocket");
    }
    if cfg.listeners.webrtc_direct {
        active.push(WEBRTC_DIRECT_TRANSPORT);
    }
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
                gossipsub_heartbeat_interval_secs: cfg.gossipsub_heartbeat_interval_secs,
                ping_interval_secs: cfg.ping_interval_secs,
                relay_cfg: &relay_cfg,
                connection_limits_cfg: &cfg.connection_limits,
                discovery_cfg: &cfg.discovery,
                resolved_cfg,
            })
        })
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_swarm_config(|swarm_cfg| {
            swarm_cfg.with_idle_connection_timeout(swarm_idle_connection_timeout(
                cfg.ping_interval_secs,
            ))
        })
        .build();

    let listen_addrs = cfg.enabled_listen_addresses()?;
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
