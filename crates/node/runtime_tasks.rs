//! Small runtime-loop tasks kept outside `runtime.rs` so the loop remains SRP-focused.

use std::sync::Arc;

use libp2p::gossipsub::IdentTopic;
use libp2p::{PeerId, Swarm};
use tokio::sync::Mutex;

use crate::api::accounted_transport_bytes;
use crate::common::error::NetError;
use crate::connectivity::dht::{
    start_dht_namespace_discovery_immediate, DhtNamespacePlan, DhtProviderState,
};
use crate::protocol::pulse::{collect_local_heartbeat, encode_heartbeat_wire};
use crate::stack::{add_external_address_candidate, MeshBehaviour};

use super::config::NodeConfig;
use super::public_ip::PublicIpProbeResult;
use super::push_pulse;
use super::snapshot::NodeSnapshot;

pub(crate) struct PublishedHeartbeat {
    pub(crate) accounted_bytes: u64,
    pub(crate) pulse: String,
}

pub(crate) fn publish_heartbeat(
    swarm: &mut Swarm<MeshBehaviour>,
    local_peer: PeerId,
    topic: &IdentTopic,
) -> Result<PublishedHeartbeat, NetError> {
    let env = collect_local_heartbeat(local_peer)?;
    let payload = encode_heartbeat_wire(&env)?;
    let accounted_bytes = accounted_transport_bytes(payload.len());
    let _ = swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic.clone(), payload);
    Ok(PublishedHeartbeat {
        accounted_bytes,
        pulse: format!("local heartbeat {} {}", env.peer_id, env.nonce_hex),
    })
}

pub(crate) async fn apply_public_ip_probe_result(
    result: PublicIpProbeResult,
    cfg: &NodeConfig,
    swarm: &mut Swarm<MeshBehaviour>,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    dht_state: &mut DhtProviderState,
    rendezvous_peer_count: usize,
) {
    for addr in &result.external_addresses {
        add_external_address_candidate(swarm, addr.clone());
    }

    let dht_plan = if result.external_addresses.is_empty() {
        None
    } else {
        Some(start_dht_namespace_discovery_immediate(
            swarm,
            cfg.network_id,
            &cfg.discovery,
            rendezvous_peer_count,
            dht_state,
        ))
    };

    let mut guard = snapshot.lock().await;
    guard.public_ip_probe_status = result.status.clone();
    guard.public_ip_probe_addr = result.public_ip.clone();
    for addr in &result.external_addresses {
        guard.record_public_external_addr(addr.to_string());
    }

    if let Some(pulse) = result.pulse_line() {
        push_pulse(&mut guard.pulses, pulse);
    }
    if let Some(plan) = dht_plan {
        apply_dht_refresh_snapshot(&mut guard, dht_state, &plan, "public_ip_probe");
    }
}

pub(crate) fn apply_dht_refresh_snapshot(
    snapshot: &mut NodeSnapshot,
    dht_state: &DhtProviderState,
    plan: &DhtNamespacePlan,
    reason: &str,
) {
    snapshot.dht_provider_announce_attempts = dht_state.announce_attempts;
    snapshot.dht_provider_announce_failures = dht_state.announce_failures;
    snapshot.dht_provider_queries = dht_state.provider_queries;
    snapshot.dht_provider_namespaces_announced = dht_state.namespaces_announced.len();
    snapshot.dht_provider_peers_discovered = dht_state.provider_peer_count();
    for err in &plan.errors {
        push_pulse(
            &mut snapshot.pulses,
            format!("dht provider refresh reason={reason} error={err}"),
        );
    }
    if plan.announce_attempts > 0 || plan.provider_queries > 0 {
        push_pulse(
            &mut snapshot.pulses,
            format!(
                "dht provider refresh reason={reason} namespaces={} announce_attempts={} provider_queries={}",
                plan.namespace_count, plan.announce_attempts, plan.provider_queries
            ),
        );
    }
}
