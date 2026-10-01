use std::sync::Arc;

use libp2p::{PeerId, Swarm};
use libp2p_relay as relay;
use tokio::sync::{broadcast, Mutex};

use crate::NodeEvent;

use super::super::snapshot::NodeSnapshot;
use crate::connectivity::relay::{
    classify_relay_denial, RelayCircuitCloseReason, RelayServiceConfig, RelayServiceHealth,
    RelayState,
};
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
    node_events: &broadcast::Sender<NodeEvent>,
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
            relay_state.record_reservation_accepted(*renewed);
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
            relay_state.record_reservation_closed();
            format!("relay_server reservation closed src={src_peer_id}")
        }
        relay::Event::CircuitReqAccepted {
            src_peer_id,
            dst_peer_id,
            circuit_id,
            usage,
        } => {
            relay_state.active_circuits = relay_state.active_circuits.saturating_add(1);
            relay_state.health = RelayServiceHealth::Enabled;
            relay_state.relay_meter.open(
                *circuit_id,
                *src_peer_id,
                *dst_peer_id,
                usage.clone(),
                unix_timestamp_ms(),
            );
            let _ = node_events.send(NodeEvent::RelayCircuitOpened {
                circuit_id: circuit_id.get(),
                src_peer_id: src_peer_id.to_string(),
                dst_peer_id: dst_peer_id.to_string(),
            });
            format!(
                "relay_server circuit accepted id={} src={src_peer_id} dst={dst_peer_id}",
                circuit_id.get()
            )
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
            circuit_id,
            bytes,
            error,
        } => {
            relay_state.active_circuits = relay_state.active_circuits.saturating_sub(1);
            let usage = relay_state.relay_meter.close(
                *circuit_id,
                *src_peer_id,
                *dst_peer_id,
                *bytes,
                error.as_ref(),
                unix_timestamp_ms(),
            );
            // Quota/duration limits are relay policy working as intended, not
            // service errors; only genuine transport failures degrade health.
            if matches!(
                usage.close_reason,
                Some(RelayCircuitCloseReason::TransportError(_))
            ) {
                relay_state.server_errors = relay_state.server_errors.saturating_add(1);
                relay_state.health = RelayServiceHealth::Error;
            }
            let line = format!(
                "relay_server circuit closed id={} src={src_peer_id} dst={dst_peer_id}                  bytes_src_to_dst={} bytes_dst_to_src={} reason={:?}",
                usage.circuit_id,
                usage.bytes_src_to_dst,
                usage.bytes_dst_to_src,
                usage.close_reason
            );
            let _ = node_events.send(NodeEvent::RelayCircuitClosed {
                circuit_id: usage.circuit_id,
                src_peer_id: usage.src_peer_id.clone(),
                dst_peer_id: usage.dst_peer_id.clone(),
                bytes_src_to_dst: usage.bytes_src_to_dst,
                bytes_dst_to_src: usage.bytes_dst_to_src,
                duration_ms: usage.duration_ms().unwrap_or_default(),
                close_reason: usage
                    .close_reason
                    .clone()
                    .unwrap_or(RelayCircuitCloseReason::Completed),
            });
            line
        }
        _ => format!("relay_server event: {ev:?}"),
    };

    relay_state.refresh_relay_totals();
    let mut guard = snapshot.lock().await;
    guard.apply_relay_state(relay_state);
    push_pulse(&mut guard.pulses, line);
}

/// Coalesced per-circuit progress and fresh aggregates (observability tick).
/// Relay serving is native-only, so browsers never drive this.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn report_relay_usage(
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    relay_state: &mut RelayState,
    node_events: &broadcast::Sender<NodeEvent>,
) -> bool {
    let changed = relay_state.relay_meter.take_changed();
    if changed.is_empty() {
        return false;
    }
    for usage in changed {
        let _ = node_events.send(NodeEvent::RelayCircuitUsage {
            circuit_id: usage.circuit_id,
            bytes_src_to_dst: usage.bytes_src_to_dst,
            bytes_dst_to_src: usage.bytes_dst_to_src,
        });
    }
    relay_state.refresh_relay_totals();
    snapshot.lock().await.apply_relay_state(relay_state);
    true
}

fn unix_timestamp_ms() -> u64 {
    crate::common::utils::unix_timestamp_ns() / 1_000_000
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
