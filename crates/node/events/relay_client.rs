use std::sync::Arc;

use libp2p::relay;
use tokio::sync::Mutex;

use super::super::snapshot::NodeSnapshot;
use crate::connectivity::relay::RelayState;

use super::super::push_pulse;

pub(crate) async fn handle_event(
    ev: relay::client::Event,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    relay_state: &mut RelayState,
) {
    let line = match ev {
        relay::client::Event::ReservationReqAccepted {
            relay_peer_id,
            renewal,
            limit: _,
        } => {
            relay_state.reservation_attempted = true;
            relay_state.relay_client_reservations.insert(relay_peer_id);
            if let Some(addresses) = relay_state
                .pending_relay_listen_addrs
                .remove(&relay_peer_id)
            {
                relay_state.relayed_listen_addrs.extend(addresses);
            }
            format!("relay_client reservation accepted relay={relay_peer_id} renewal={renewal}")
        }
        relay::client::Event::OutboundCircuitEstablished {
            relay_peer_id,
            limit: _,
        } => format!("relay_client outbound relayed circuit established relay={relay_peer_id}"),
        relay::client::Event::InboundCircuitEstablished {
            src_peer_id,
            limit: _,
        } => format!("relay_client inbound relayed circuit established src={src_peer_id}"),
    };

    let mut guard = snapshot.lock().await;
    guard.apply_relay_state(relay_state);
    push_pulse(&mut guard.pulses, line);
}
