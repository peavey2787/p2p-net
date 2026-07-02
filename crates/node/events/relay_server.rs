use std::sync::Arc;

use libp2p::relay;
use libp2p::{PeerId, Swarm};
use tokio::sync::Mutex;

use crate::connectivity::relay::{classify_relay_denial, RelayServiceConfig, RelayServiceHealth, RelayState};
use super::super::snapshot::NodeSnapshot;
use crate::stack::MeshBehaviour;

use super::super::push_pulse;

pub(crate) async fn enforce_relay_schedule(
    relay_cfg: &RelayServiceConfig,
    swarm: &mut Swarm<MeshBehaviour>,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    relay_state: &mut RelayState,
) {
    let next_health = relay_cfg.health_now();
    let was_open = matches!(relay_state.health, RelayServiceHealth::Enabled);
    let is_open = matches!(next_health, RelayServiceHealth::Enabled);

    relay_state.health = next_health;
    relay_state.server_enabled = is_open;

    let mut guard = snapshot.lock().await;
    guard.relay_service_health = next_health;
    guard.relay_server_enabled = is_open;

    if relay_cfg.enabled && was_open && !is_open {
        let peers: Vec<PeerId> = swarm.connected_peers().cloned().collect();
        for peer in peers.iter().cloned() {
            let _ = swarm.disconnect_peer_id(peer);
        }
        push_pulse(
            &mut guard.pulses,
            format!(
                "relay_server schedule closed; disconnecting {} connected peers",
                peers.len()
            ),
        );
    } else if relay_cfg.enabled && !was_open && is_open {
        push_pulse(
            &mut guard.pulses,
            "relay_server schedule opened".to_string(),
        );
    }
}

pub(crate) async fn handle_event(
    ev: relay::Event,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    relay_state: &mut RelayState,
) {
    relay_state.server_enabled = true;
    if matches!(relay_state.health, RelayServiceHealth::Disabled) {
        relay_state.health = RelayServiceHealth::Enabled;
    }

    let line = match &ev {
        relay::Event::ReservationReqAccepted {
            src_peer_id,
            renewed,
        } => {
            if !renewed {
                relay_state.accepted_reservations =
                    relay_state.accepted_reservations.saturating_add(1);
            }
            relay_state.health = RelayServiceHealth::Enabled;
            format!("relay_server reservation accepted src={src_peer_id} renewed={renewed}")
        }
        relay::Event::ReservationReqDenied {
            src_peer_id,
            status,
        } => {
            relay_state.denied_reservations = relay_state.denied_reservations.saturating_add(1);
            apply_denial_health(relay_state, &format!("{status:?}"));
            format!("relay_server reservation denied src={src_peer_id} status={status:?}")
        }
        relay::Event::ReservationClosed { src_peer_id }
        | relay::Event::ReservationTimedOut { src_peer_id } => {
            relay_state.accepted_reservations = relay_state.accepted_reservations.saturating_sub(1);
            format!("relay_server reservation closed src={src_peer_id}")
        }
        relay::Event::CircuitReqAccepted {
            src_peer_id,
            dst_peer_id,
        } => {
            relay_state.active_circuits = relay_state.active_circuits.saturating_add(1);
            relay_state.health = RelayServiceHealth::Enabled;
            format!("relay_server circuit accepted src={src_peer_id} dst={dst_peer_id}")
        }
        relay::Event::CircuitReqDenied {
            src_peer_id,
            dst_peer_id,
            status,
        } => {
            relay_state.denied_circuits = relay_state.denied_circuits.saturating_add(1);
            apply_denial_health(relay_state, &format!("{status:?}"));
            format!(
                "relay_server circuit denied src={src_peer_id} dst={dst_peer_id} status={status:?}"
            )
        }
        relay::Event::CircuitClosed {
            src_peer_id,
            dst_peer_id,
            error,
        } => {
            relay_state.active_circuits = relay_state.active_circuits.saturating_sub(1);
            if error.is_some() {
                relay_state.server_errors = relay_state.server_errors.saturating_add(1);
                relay_state.health = RelayServiceHealth::Error;
            }
            format!(
                "relay_server circuit closed src={src_peer_id} dst={dst_peer_id} error={error:?}"
            )
        }
        _ => format!("relay_server event: {ev:?}"),
    };

    let mut guard = snapshot.lock().await;
    guard.apply_relay_state(relay_state);
    push_pulse(&mut guard.pulses, line);
}

fn apply_denial_health(relay_state: &mut RelayState, status_debug: &str) {
    match classify_relay_denial(status_debug) {
        RelayServiceHealth::RateLimited => {
            relay_state.rate_limited_events = relay_state.rate_limited_events.saturating_add(1);
            relay_state.health = RelayServiceHealth::RateLimited;
        }
        RelayServiceHealth::AtCapacity => {
            relay_state.at_capacity_events = relay_state.at_capacity_events.saturating_add(1);
            relay_state.health = RelayServiceHealth::AtCapacity;
        }
        _ => {
            relay_state.server_errors = relay_state.server_errors.saturating_add(1);
            relay_state.health = RelayServiceHealth::Error;
        }
    }
}
