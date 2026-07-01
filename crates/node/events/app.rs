use crate::api::decode_app_message;

use super::SwarmEventContext;

pub(crate) async fn handle_app_message(data: Vec<u8>, ctx: &mut SwarmEventContext<'_>) {
    match decode_app_message(&data) {
        Ok(message) if message.network_id != ctx.network_id => {
            let mut guard = ctx.snapshot.lock().await;
            guard.app_messages_ignored = guard.app_messages_ignored.saturating_add(1);
        }
        Ok(message) if !message.is_for_peer(&ctx.local_peer) => {
            let mut guard = ctx.snapshot.lock().await;
            guard.app_messages_ignored = guard.app_messages_ignored.saturating_add(1);
        }
        Ok(message) => {
            let _ = ctx.app_messages.send(message);
            let mut guard = ctx.snapshot.lock().await;
            guard.app_messages_received = guard.app_messages_received.saturating_add(1);
        }
        Err(_) => {
            let mut guard = ctx.snapshot.lock().await;
            guard.app_messages_rejected = guard.app_messages_rejected.saturating_add(1);
        }
    }
}
