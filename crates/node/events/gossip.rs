use libp2p::gossipsub::{MessageAcceptance, MessageId};
use libp2p::{PeerId, Swarm};

use crate::api::{accounted_transport_bytes, PeerSource};
use crate::protocol::pulse::{validate_heartbeat_wire, HeartbeatValidationDecision};
use crate::stack::MeshBehaviour;

use super::SwarmEventContext;

pub(crate) fn handle_heartbeat_message(
    swarm: &mut Swarm<MeshBehaviour>,
    propagation_source: PeerId,
    authenticated_source: Option<PeerId>,
    msg_id: MessageId,
    data: Vec<u8>,
    ctx: &mut SwarmEventContext<'_>,
) {
    ctx.metrics.bandwidth.record_received(
        Some(propagation_source),
        Some("heartbeat"),
        accounted_transport_bytes(data.len()),
    );

    let Some(author) = authenticated_source else {
        reject_heartbeat(
            swarm,
            propagation_source,
            propagation_source,
            &msg_id,
            ctx,
            "rejected_missing_authenticated_author",
        );
        return;
    };

    // `propagation_source` is only the immediate forwarding peer. In signed
    // Gossipsub the authenticated `message.source` is the heartbeat author and
    // must be the identity bound to the heartbeat envelope.
    let validation = validate_heartbeat_wire(
        author,
        &data,
        crate::common::utils::unix_timestamp_ns(),
        ctx.message_security,
        ctx.replay_cache,
    );

    match validation.decision {
        HeartbeatValidationDecision::Accept => {
            ctx.rep.accept(author);
            swarm
                .behaviour_mut()
                .gossipsub
                .report_message_validation_result(
                    &msg_id,
                    &propagation_source,
                    MessageAcceptance::Accept,
                );
            let peer_dirty = validation.envelope.is_some();
            if peer_dirty {
                record_heartbeat_peer(author, ctx);
            }
            ctx.observability.gossip_accepted(peer_dirty);
            if let Some(env) = validation.envelope {
                ctx.observability
                    .pulse(format!("peer heartbeat {} {}", env.peer_id, env.nonce_hex));
            }
        }
        HeartbeatValidationDecision::IgnoreDuplicate => {
            ctx.rep.ignore_duplicate(author);
            swarm
                .behaviour_mut()
                .gossipsub
                .report_message_validation_result(
                    &msg_id,
                    &propagation_source,
                    MessageAcceptance::Ignore,
                );
            ctx.observability.gossip_ignored();
            ctx.observability
                .pulse(format!("peer {author} ignored_duplicate_heartbeat"));
        }
        HeartbeatValidationDecision::RejectOversize => {
            reject_heartbeat(
                swarm,
                propagation_source,
                author,
                &msg_id,
                ctx,
                "rejected_oversize",
            );
        }
        HeartbeatValidationDecision::Reject => {
            reject_heartbeat(
                swarm,
                propagation_source,
                author,
                &msg_id,
                ctx,
                "rejected_heartbeat",
            );
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
    propagation_source: PeerId,
    offender: PeerId,
    msg_id: &MessageId,
    ctx: &mut SwarmEventContext<'_>,
    reason: &str,
) {
    ctx.rep.penalize_invalid(offender);
    swarm
        .behaviour_mut()
        .gossipsub
        .report_message_validation_result(msg_id, &propagation_source, MessageAcceptance::Reject);
    ctx.observability.gossip_rejected();
    ctx.observability.pulse(format!("peer {offender} {reason}"));
}
