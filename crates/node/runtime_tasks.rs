//! Small runtime-loop tasks kept outside `runtime.rs` so the loop remains SRP-focused.

use std::sync::Arc;

use libp2p::gossipsub::IdentTopic;
use libp2p::{PeerId, Swarm};
use tokio::sync::Mutex;

use crate::api::accounted_transport_bytes;
use crate::common::error::NetError;
use crate::connectivity::dht::{DhtNamespacePlan, DhtProviderState};
use crate::connectivity::peer_book::PeerBook;
use crate::protocol::pulse::{collect_local_heartbeat, encode_heartbeat_wire};
use crate::stack::{
    add_external_address_candidate, refresh_dcutr_candidate, retain_application_peer, MeshBehaviour,
};

use super::public_ip::PublicIpProbeResult;
use super::push_pulse;
use super::snapshot::NodeSnapshot;

pub(crate) struct PublishedHeartbeat {
    pub(crate) accounted_bytes: u64,
    pub(crate) pulse: String,
}

/// Keep verified application peers in Gossipsub's explicit-peer set so the
/// signed application heartbeat is forwarded to them even when they are not
/// selected for the normal mesh. Normal application traffic remains unchanged.
pub(crate) fn refresh_application_keepalive_peers(
    swarm: &mut Swarm<MeshBehaviour>,
    peer_book: &PeerBook,
    application_namespaces: &[String],
) -> usize {
    let peers = swarm
        .connected_peers()
        .copied()
        .filter(|peer| peer_book.has_application_namespace(peer, application_namespaces))
        .collect::<Vec<_>>();
    for peer in &peers {
        retain_application_peer(swarm, *peer);
        swarm.behaviour_mut().gossipsub.add_explicit_peer(peer);
    }
    peers.len()
}

pub(crate) fn publish_heartbeat(
    swarm: &mut Swarm<MeshBehaviour>,
    local_peer: PeerId,
    topic: &IdentTopic,
    keepalive_peers: usize,
) -> Result<PublishedHeartbeat, NetError> {
    let env = collect_local_heartbeat(local_peer)?;
    let payload = encode_heartbeat_wire(&env)?;
    let accounted_bytes = accounted_transport_bytes(payload.len());
    swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic.clone(), payload)
        .map_err(|err| {
            NetError::Heartbeat(format!(
                "application keepalive heartbeat publish failed for {keepalive_peers} peer(s): {err:?}"
            ))
        })?;
    Ok(PublishedHeartbeat {
        accounted_bytes,
        pulse: format!(
            "local heartbeat {} {} keepalive_peers={keepalive_peers}",
            env.peer_id, env.nonce_hex
        ),
    })
}

pub(crate) async fn apply_public_ip_probe_result(
    result: PublicIpProbeResult,
    swarm: &mut Swarm<MeshBehaviour>,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
) {
    for addr in &result.external_addresses {
        refresh_dcutr_candidate(swarm, addr);
        add_external_address_candidate(swarm, addr.clone());
    }

    let mut guard = snapshot.lock().await;
    guard.public_ip_probe_status = result.status.clone();
    guard.public_ip_probe_addr = result.public_ip.clone();
    for addr in &result.external_addresses {
        guard.record_public_external_addr(addr.to_string());
    }

    if let Some(pulse) = result.pulse_line() {
        push_pulse(&mut guard.pulses, pulse);
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
