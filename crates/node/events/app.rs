use libp2p::PeerId;

use crate::api::{accounted_transport_bytes, decode_app_message};

use super::SwarmEventContext;

pub(crate) fn handle_app_message(
    propagation_source: PeerId,
    data: Vec<u8>,
    ctx: &mut SwarmEventContext<'_>,
) {
    let accounted_bytes = accounted_transport_bytes(data.len());
    match decode_app_message(&data) {
        Ok(message) if message.network_id != ctx.network_id => {
            ctx.metrics.bandwidth.record_received(
                Some(propagation_source),
                Some(&message.topic),
                accounted_bytes,
            );
            ctx.observability.app_ignored();
        }
        Ok(message) if !message.is_for_peer(&ctx.local_peer) => {
            ctx.metrics.bandwidth.record_received(
                Some(propagation_source),
                Some(&message.topic),
                accounted_bytes,
            );
            ctx.observability.app_ignored();
        }
        Ok(message) => {
            ctx.metrics.bandwidth.record_received(
                Some(propagation_source),
                Some(&message.topic),
                accounted_bytes,
            );
            let _ = ctx.app_messages.send(message);
            ctx.observability.app_received();
        }
        Err(_) => {
            ctx.metrics
                .bandwidth
                .record_received(Some(propagation_source), None, accounted_bytes);
            ctx.observability.app_rejected();
        }
    }
}
