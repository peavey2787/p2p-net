use libp2p::PeerId;

use crate::api::{accounted_transport_bytes, decode_app_message};

use super::SwarmEventContext;

pub(crate) async fn handle_app_message(
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
            let mut guard = ctx.snapshot.lock().await;
            guard.app_messages_ignored = guard.app_messages_ignored.saturating_add(1);
        }
        Ok(message) if !message.is_for_peer(&ctx.local_peer) => {
            ctx.metrics.bandwidth.record_received(
                Some(propagation_source),
                Some(&message.topic),
                accounted_bytes,
            );
            let mut guard = ctx.snapshot.lock().await;
            guard.app_messages_ignored = guard.app_messages_ignored.saturating_add(1);
        }
        Ok(message) => {
            ctx.metrics.bandwidth.record_received(
                Some(propagation_source),
                Some(&message.topic),
                accounted_bytes,
            );
            let _ = ctx.app_messages.send(message);
            let mut guard = ctx.snapshot.lock().await;
            guard.app_messages_received = guard.app_messages_received.saturating_add(1);
        }
        Err(_) => {
            ctx.metrics
                .bandwidth
                .record_received(Some(propagation_source), None, accounted_bytes);
            let mut guard = ctx.snapshot.lock().await;
            guard.app_messages_rejected = guard.app_messages_rejected.saturating_add(1);
        }
    }
}
