use std::sync::Arc;

use libp2p::{Multiaddr, Swarm};
use libp2p_relay as relay;
use tokio::sync::Mutex;

use super::super::snapshot::NodeSnapshot;
use super::connection::confirm_relayed_listen_addr;
use crate::connectivity::relay::RelayState;
use crate::stack::MeshBehaviour;

use super::super::push_pulse;

pub(crate) async fn handle_event(
    ev: relay::client::Event,
    swarm: &mut Swarm<MeshBehaviour>,
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
            relay_state.reservation_retries.accepted(&relay_peer_id);
            let local = *swarm.local_peer_id();
            for route in relay_state.requested_routes(&relay_peer_id, local) {
                confirm_relayed_listen_addr(swarm, relay_state, &route);
            }
            if let Some(addresses) = relay_state
                .pending_relay_listen_addrs
                .remove(&relay_peer_id)
            {
                for address in addresses {
                    if let Ok(addr) = address.parse::<Multiaddr>() {
                        confirm_relayed_listen_addr(swarm, relay_state, &addr);
                    }
                }
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

/// A swarm listener closed. A closed `/p2p-circuit` reservation listener
/// means the reservation failed (or timed out) or the relay connection was
/// lost; schedule a re-request with backoff.
pub(crate) async fn handle_listener_closed(
    listener: libp2p::core::transport::ListenerId,
    error: Option<String>,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    relay_state: &mut RelayState,
) {
    let Some((addr, delay)) = relay_state
        .reservation_retries
        .closed(listener, web_time::Instant::now())
    else {
        return;
    };
    if error.is_some() {
        relay_state.relay_client_reservation_failures = relay_state
            .relay_client_reservation_failures
            .saturating_add(1);
    }
    let reason = error.unwrap_or_else(|| "relay connection closed".to_string());
    let mut guard = snapshot.lock().await;
    guard.apply_relay_state(relay_state);
    push_pulse(
        &mut guard.pulses,
        format!(
            "relay_client reservation closed via {addr} reason={reason} retry_in={}s",
            delay.as_secs()
        ),
    );
}

/// Re-request reservations whose backoff elapsed.
pub(crate) fn retry_due_reservations(
    swarm: &mut Swarm<MeshBehaviour>,
    relay_state: &mut RelayState,
) -> Vec<String> {
    let due = relay_state
        .reservation_retries
        .take_due(web_time::Instant::now());
    due.into_iter()
        .map(|addr| match swarm.listen_on(addr.clone()) {
            Ok(listener) => {
                relay_state
                    .reservation_retries
                    .track(listener, addr.clone());
                relay_state.relay_client_reservation_attempts = relay_state
                    .relay_client_reservation_attempts
                    .saturating_add(1);
                format!("relay_client reservation re-requested via {addr}")
            }
            Err(err) => {
                relay_state.relay_client_reservation_failures = relay_state
                    .relay_client_reservation_failures
                    .saturating_add(1);
                format!("relay_client reservation re-request failed via {addr}: {err}")
            }
        })
        .collect()
}
