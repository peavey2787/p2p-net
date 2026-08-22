use libp2p::gossipsub::{MessageAcceptance, MessageId};
use libp2p::{PeerId, Swarm};

use crate::api::{accounted_transport_bytes, PeerSource};
use crate::protocol::pulse::{validate_heartbeat_wire, HeartbeatValidationDecision};
use crate::stack::MeshBehaviour;

use super::SwarmEventContext;

pub(crate) fn handle_heartbeat_message(
    swarm: &mut Swarm<MeshBehaviour>,
    peer: PeerId,
    msg_id: MessageId,
    data: Vec<u8>,
    ctx: &mut SwarmEventContext<'_>,
) {
    ctx.metrics.bandwidth.record_received(
        Some(peer),
        Some("heartbeat"),
        accounted_transport_bytes(data.len()),
    );
    let validation = validate_heartbeat_wire(
        peer,
        &data,
        crate::common::utils::unix_timestamp_ns(),
        ctx.message_security,
        ctx.replay_cache,
    );

    match validation.decision {
        HeartbeatValidationDecision::Accept => {
            ctx.rep.accept(peer);
            swarm
                .behaviour_mut()
                .gossipsub
                .report_message_validation_result(&msg_id, &peer, MessageAcceptance::Accept);
            let peer_dirty = validation.envelope.is_some();
            if peer_dirty {
                record_heartbeat_peer(peer, ctx);
            }
            ctx.observability.gossip_accepted(peer_dirty);
            if let Some(env) = validation.envelope {
                ctx.observability
                    .pulse(format!("peer heartbeat {} {}", env.peer_id, env.nonce_hex));
            }
        }
        HeartbeatValidationDecision::IgnoreDuplicate => {
            ctx.rep.ignore_duplicate(peer);
            swarm
                .behaviour_mut()
                .gossipsub
                .report_message_validation_result(&msg_id, &peer, MessageAcceptance::Ignore);
            ctx.observability.gossip_ignored();
            ctx.observability
                .pulse(format!("peer {peer} ignored_duplicate_heartbeat"));
        }
        HeartbeatValidationDecision::RejectOversize => {
            reject_heartbeat(swarm, peer, &msg_id, ctx, "rejected_oversize");
        }
        HeartbeatValidationDecision::Reject => {
            reject_heartbeat(swarm, peer, &msg_id, ctx, "rejected_heartbeat");
        }
    }
}

fn record_heartbeat_peer(peer: PeerId, ctx: &mut SwarmEventContext<'_>) {
    for namespace in ctx.application_namespaces {
        ctx.peer_book
            .record_namespace(peer, namespace.clone(), PeerSource::Connected);
    }
    ctx.peer_book.record_connected(peer, None);
    ctx.relay_state.unverified_relayed_peers.remove(&peer);
}

pub(crate) fn handle_unexpected_topic_message(
    swarm: &mut Swarm<MeshBehaviour>,
    peer: PeerId,
    msg_id: MessageId,
    ctx: &mut SwarmEventContext<'_>,
) {
    swarm
        .behaviour_mut()
        .gossipsub
        .report_message_validation_result(&msg_id, &peer, MessageAcceptance::Ignore);
    ctx.observability.gossip_ignored();
    ctx.observability
        .pulse(format!("peer {peer} ignored_unexpected_gossip_topic"));
}

fn reject_heartbeat(
    swarm: &mut Swarm<MeshBehaviour>,
    peer: PeerId,
    msg_id: &MessageId,
    ctx: &mut SwarmEventContext<'_>,
    reason: &str,
) {
    ctx.rep.penalize_invalid(peer);
    swarm
        .behaviour_mut()
        .gossipsub
        .report_message_validation_result(msg_id, &peer, MessageAcceptance::Reject);
    ctx.observability.gossip_rejected();
    ctx.observability.pulse(format!("peer {peer} {reason}"));
}
