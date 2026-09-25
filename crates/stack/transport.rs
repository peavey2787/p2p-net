use std::time::Duration;

use libp2p::identity::Keypair;
use libp2p::swarm::Swarm;

use super::behaviour::MeshBehaviour;
use crate::common::error::NetError;
use crate::platform::NodeStorage;
use crate::{NodeConfig, ResolvedNodeConfig};

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod wasm;

const MIN_SWARM_IDLE_CONNECTION_TIMEOUT_SECS: u64 = 30;
const SWARM_IDLE_TIMEOUT_PING_MULTIPLIER: u64 = 2;

pub(super) fn swarm_idle_connection_timeout(ping_interval_secs: u64) -> Duration {
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
    storage: &dyn NodeStorage,
) -> Result<(Swarm<MeshBehaviour>, TransportPlan), NetError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        native::build_swarm(local_key, cfg, resolved_cfg, storage).await
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = storage;
        wasm::build_swarm(local_key, cfg, resolved_cfg).await
    }
}

pub(super) fn transport_plan(cfg: &NodeConfig, resolved_cfg: &ResolvedNodeConfig) -> TransportPlan {
    use crate::connectivity::webrtc::WEBRTC_DIRECT_TRANSPORT;

    let behaviour_policy = &resolved_cfg.enabled_behaviours;
    let mut active = Vec::new();
    if cfg.listeners.quic && resolved_cfg.transport_capabilities.quic {
        active.push("quic");
    }
    if cfg.listeners.tcp && resolved_cfg.transport_capabilities.tcp {
        active.push("tcp");
    }
    if resolved_cfg.transport_capabilities.secure_websocket {
        active.push("websocket");
    }
    if resolved_cfg.transport_capabilities.webrtc_direct {
        active.push(WEBRTC_DIRECT_TRANSPORT);
    }
    if resolved_cfg.transport_capabilities.webtransport {
        active.push("webtransport");
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
    if behaviour_policy.relay_server && cfg.relay.enabled {
        active.push("relay-server");
        if resolved_cfg.mediator_enabled {
            active.push("mediator");
        }
        active.push("relay-acl");
        if cfg.relay.schedule.enabled {
            active.push("relay-schedule");
        }
    }
    TransportPlan { active }
}
