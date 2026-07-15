use std::sync::Arc;

use libp2p::gossipsub::{MessageAcceptance, MessageId};
use libp2p::{PeerId, Swarm};
use tokio::sync::Mutex;

use super::super::snapshot::NodeSnapshot;
use crate::api::accounted_transport_bytes;
use crate::protocol::pulse::{validate_heartbeat_wire, HeartbeatValidationDecision};
use crate::stack::MeshBehaviour;

use super::super::push_pulse;
use super::SwarmEventContext;

pub(crate) async fn handle_heartbeat_message(
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
            let mut guard = ctx.snapshot.lock().await;
            guard.gossip_messages_accepted = guard.gossip_messages_accepted.saturating_add(1);
            if let Some(env) = validation.envelope {
                push_pulse(
                    &mut guard.pulses,
                    format!("peer heartbeat {} {}", env.peer_id, env.nonce_hex),
                );
            }
        }
        HeartbeatValidationDecision::IgnoreDuplicate => {
            ctx.rep.ignore_duplicate(peer);
            swarm
                .behaviour_mut()
                .gossipsub
                .report_message_validation_result(&msg_id, &peer, MessageAcceptance::Ignore);
            let mut guard = ctx.snapshot.lock().await;
            guard.gossip_messages_ignored = guard.gossip_messages_ignored.saturating_add(1);
            push_pulse(
                &mut guard.pulses,
                format!("peer {peer} ignored_duplicate_heartbeat"),
            );
        }
        HeartbeatValidationDecision::RejectOversize => {
            reject_heartbeat(swarm, peer, &msg_id, ctx, "rejected_oversize").await;
        }
        HeartbeatValidationDecision::Reject => {
            reject_heartbeat(swarm, peer, &msg_id, ctx, "rejected_heartbeat").await;
        }
    }
}

pub(crate) async fn handle_unexpected_topic_message(
    swarm: &mut Swarm<MeshBehaviour>,
    peer: PeerId,
    msg_id: MessageId,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
) {
    swarm
        .behaviour_mut()
        .gossipsub
        .report_message_validation_result(&msg_id, &peer, MessageAcceptance::Ignore);
    let mut guard = snapshot.lock().await;
    guard.gossip_messages_ignored = guard.gossip_messages_ignored.saturating_add(1);
    push_pulse(
        &mut guard.pulses,
        format!("peer {peer} ignored_unexpected_gossip_topic"),
    );
}

async fn reject_heartbeat(
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
    let mut guard = ctx.snapshot.lock().await;
    guard.gossip_messages_rejected = guard.gossip_messages_rejected.saturating_add(1);
    push_pulse(&mut guard.pulses, format!("peer {peer} {reason}"));
}
