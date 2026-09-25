use libp2p::gossipsub::{MessageAcceptance, MessageId, TopicHash};
use libp2p::{PeerId, Swarm};

use crate::api::{
    accounted_transport_bytes, app_ident_topic, decode_app_message, decode_fragment,
    validate_app_message_authentication, AppMessage,
};
use crate::protocol::app_security::{validate_app_message_security, AppMessageSecurityDecision};
use crate::stack::MeshBehaviour;

use super::SwarmEventContext;

pub(crate) fn handle_app_message(
    swarm: &mut Swarm<MeshBehaviour>,
    propagation_source: PeerId,
    authenticated_source: Option<PeerId>,
    message_id: MessageId,
    received_topic: TopicHash,
    data: Vec<u8>,
    ctx: &mut SwarmEventContext<'_>,
) {
    let accounted_bytes = accounted_transport_bytes(data.len());
    let Some(author) = authenticated_source else {
        reject_app_message(
            swarm,
            propagation_source,
            &message_id,
            None,
            accounted_bytes,
            ctx,
            "missing authenticated Gossipsub author",
        );
        return;
    };

    // Small messages retain the original wire envelope for backwards
    // compatibility. Large messages use bounded internal fragments and are
    // reassembled before freshness/replay validation or app delivery.
    let message = match decode_app_message(&data) {
        Ok(message) => message,
        Err(_) => {
            let fragment = match decode_fragment(&data) {
                Ok(fragment) => fragment,
                Err(_) => {
                    reject_app_message(
                        swarm,
                        propagation_source,
                        &message_id,
                        Some(author),
                        accounted_bytes,
                        ctx,
                        "invalid application envelope/fragment",
                    );
                    return;
                }
            };
            let expected_topic = match app_ident_topic(fragment.network_id, &fragment.topic) {
                Ok(topic) => topic.hash(),
                Err(_) => {
                    reject_app_message(
                        swarm,
                        propagation_source,
                        &message_id,
                        Some(author),
                        accounted_bytes,
                        ctx,
                        "invalid fragment topic",
                    );
                    return;
                }
            };
            if fragment.network_id != ctx.network_id
                || fragment.source_peer_id != author.to_string()
                || expected_topic != received_topic
            {
                reject_app_message(
                    swarm,
                    propagation_source,
                    &message_id,
                    Some(author),
                    accounted_bytes,
                    ctx,
                    "fragment authentication/topic mismatch",
                );
                return;
            }
            let reassembled = match ctx.app_fragment_reassembler.push(author, fragment) {
                Ok(message) => message,
                Err(_) => {
                    reject_app_message(
                        swarm,
                        propagation_source,
                        &message_id,
                        Some(author),
                        accounted_bytes,
                        ctx,
                        "fragment reassembly rejected",
                    );
                    return;
                }
            };
            ctx.metrics
                .bandwidth
                .record_received(Some(propagation_source), None, accounted_bytes);
            let Some(message) = reassembled else {
                // Every individually valid incomplete fragment must be accepted
                // so the mesh forwards it. Security/replay validation runs once
                // on the completed logical message.
                swarm
                    .behaviour_mut()
                    .gossipsub
                    .report_message_validation_result(
                        &message_id,
                        &propagation_source,
                        MessageAcceptance::Accept,
                    );
                return;
            };
            message
        }
    };

    process_complete_message(
        swarm,
        propagation_source,
        author,
        message_id,
        received_topic,
        accounted_bytes,
        message,
        ctx,
    );
}

#[allow(clippy::too_many_arguments)]
fn process_complete_message(
    swarm: &mut Swarm<MeshBehaviour>,
    propagation_source: PeerId,
    author: PeerId,
    message_id: MessageId,
    received_topic: TopicHash,
    accounted_bytes: u64,
    message: AppMessage,
    ctx: &mut SwarmEventContext<'_>,
) {
    if message.network_id != ctx.network_id
        || validate_app_message_authentication(&message, &author, &received_topic).is_err()
    {
        reject_app_message(
            swarm,
            propagation_source,
            &message_id,
            Some(author),
            accounted_bytes,
            ctx,
            "application authentication/topic mismatch",
        );
        return;
    }

    match validate_app_message_security(
        author,
        &message,
        crate::common::utils::unix_timestamp_ns(),
        ctx.message_security,
        ctx.app_replay_cache,
    ) {
        AppMessageSecurityDecision::Accept => {}
        AppMessageSecurityDecision::IgnoreDuplicate => {
            ctx.rep.ignore_duplicate(author);
            swarm
                .behaviour_mut()
                .gossipsub
                .report_message_validation_result(
                    &message_id,
                    &propagation_source,
                    MessageAcceptance::Ignore,
                );
            ctx.metrics.bandwidth.record_received(
                Some(propagation_source),
                Some(&message.topic),
                accounted_bytes,
            );
            ctx.observability.app_ignored();
            ctx.observability
                .pulse(format!("peer {author} ignored_duplicate_app_message"));
            return;
        }
        AppMessageSecurityDecision::Reject => {
            reject_app_message(
                swarm,
                propagation_source,
                &message_id,
                Some(author),
                accounted_bytes,
                ctx,
                "application freshness/replay validation failed",
            );
            return;
        }
    }

    swarm
        .behaviour_mut()
        .gossipsub
        .report_message_validation_result(
            &message_id,
            &propagation_source,
            MessageAcceptance::Accept,
        );
    ctx.rep.accept(author);
    ctx.metrics.bandwidth.record_received(
        Some(propagation_source),
        Some(&message.topic),
        accounted_bytes,
    );

    if message.is_for_peer(&ctx.local_peer) {
        let _ = ctx.app_messages.send(message);
        ctx.observability.app_received();
    } else {
        ctx.observability.app_ignored();
    }
}

fn reject_app_message(
    swarm: &mut Swarm<MeshBehaviour>,
    propagation_source: PeerId,
    message_id: &MessageId,
    authenticated_source: Option<PeerId>,
    accounted_bytes: u64,
    ctx: &mut SwarmEventContext<'_>,
    reason: &str,
) {
    if let Some(author) = authenticated_source {
        ctx.rep.penalize_invalid(author);
    }
    swarm
        .behaviour_mut()
        .gossipsub
        .report_message_validation_result(
            message_id,
            &propagation_source,
            MessageAcceptance::Reject,
        );
    ctx.metrics
        .bandwidth
        .record_received(Some(propagation_source), None, accounted_bytes);
    ctx.observability.app_rejected();
    ctx.observability.pulse(format!(
        "peer {propagation_source} rejected_app_message {reason}"
    ));
}
