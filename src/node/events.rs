use std::sync::Arc;

use libp2p::gossipsub::{MessageAcceptance, MessageId, TopicHash};
use libp2p::relay;
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm};
use tokio::sync::Mutex;

use crate::connectivity::discovery::DiscoveryConfig;
use crate::connectivity::limits::ConnectionCapState;
use crate::connectivity::peer_cache;
use crate::connectivity::relay::{
    classify_relay_denial, is_p2p_circuit_addr, update_nat_state, RelayServiceConfig,
    RelayServiceHealth, RelayState,
};
use crate::connectivity::rendezvous::RendezvousState;
use crate::protocol::pulse::{
    validate_heartbeat_wire, HeartbeatReplayCache, HeartbeatValidationDecision,
    MessageSecurityConfig,
};
use crate::protocol::reputation::ReputationStore;
use crate::stack::{
    on_mesh_event, on_rendezvous_client_event, on_rendezvous_server_event, refresh_rendezvous,
    MeshBehaviour, MeshEvent,
};

use super::push_pulse;
use super::types::NodeSnapshot;

pub(crate) struct SwarmEventContext<'a> {
    pub(crate) snapshot: &'a Arc<Mutex<NodeSnapshot>>,
    pub(crate) rep: &'a mut ReputationStore,
    pub(crate) relay_state: &'a mut RelayState,
    pub(crate) rendezvous_state: &'a mut RendezvousState,
    pub(crate) connection_caps: &'a mut ConnectionCapState,
    pub(crate) relay_cfg: &'a RelayServiceConfig,
    pub(crate) discovery_cfg: &'a DiscoveryConfig,
    pub(crate) rendezvous_peers: &'a [Multiaddr],
    pub(crate) message_security: &'a MessageSecurityConfig,
    pub(crate) replay_cache: &'a mut HeartbeatReplayCache,
    pub(crate) heartbeat_topic_hash: &'a TopicHash,
}

pub(crate) async fn handle_swarm_event(
    evt: SwarmEvent<MeshEvent>,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) {
    match evt {
        SwarmEvent::ConnectionEstablished {
            peer_id,
            connection_id,
            endpoint,
            ..
        } => {
            peer_cache::record_seen_peer_addr(
                ctx.discovery_cfg,
                &peer_id,
                endpoint.get_remote_address(),
            );

            if ctx.relay_cfg.enabled && !ctx.relay_cfg.schedule.is_open_now_utc() {
                let _ = swarm.close_connection(connection_id);
                ctx.relay_state.health = RelayServiceHealth::ClosedBySchedule;
                let mut guard = ctx.snapshot.lock().await;
                guard.connected_peers = swarm.connected_peers().count();
                guard.apply_relay_state(ctx.relay_state);
                guard.relay_server_enabled = false;
                push_pulse(
                    &mut guard.pulses,
                    format!("relay_server schedule closed; closing connection from {peer_id}"),
                );
                return;
            }

            let over_ip_cap = ctx
                .connection_caps
                .record_established(connection_id, endpoint.get_remote_address());
            if over_ip_cap {
                let _ = swarm.close_connection(connection_id);
                let mut guard = ctx.snapshot.lock().await;
                guard.connected_peers = swarm.connected_peers().count();
                guard.connection_limit_events = guard.connection_limit_events.saturating_add(1);
                guard.connection_cap_disconnects = ctx.connection_caps.cap_disconnects;
                push_pulse(
                    &mut guard.pulses,
                    format!(
                        "connection cap exceeded; closing connection {connection_id:?} from {}",
                        endpoint.get_remote_address()
                    ),
                );
                return;
            }

            let mut guard = ctx.snapshot.lock().await;
            guard.connected_peers = swarm.connected_peers().count();
            guard.connection_cap_disconnects = ctx.connection_caps.cap_disconnects;
            if is_p2p_circuit_addr(endpoint.get_remote_address()) {
                push_pulse(
                    &mut guard.pulses,
                    format!(
                        "relay_fallback connection established via {}",
                        endpoint.get_remote_address()
                    ),
                );
            }
        }
        SwarmEvent::ConnectionClosed { connection_id, .. } => {
            ctx.connection_caps.record_closed(connection_id);
            let mut guard = ctx.snapshot.lock().await;
            guard.connected_peers = swarm.connected_peers().count();
            guard.connection_cap_disconnects = ctx.connection_caps.cap_disconnects;
        }
        SwarmEvent::IncomingConnectionError { error, peer_id, .. } => {
            let mut guard = ctx.snapshot.lock().await;
            guard.connection_limit_events = guard.connection_limit_events.saturating_add(1);
            push_pulse(
                &mut guard.pulses,
                format!("incoming connection error peer={peer_id:?} error={error:?}"),
            );
        }
        SwarmEvent::OutgoingConnectionError { error, peer_id, .. } => {
            if let Some(peer) = peer_id {
                peer_cache::record_peer_addr_failure(ctx.discovery_cfg, &peer);
            }
            let mut guard = ctx.snapshot.lock().await;
            guard.connection_limit_events = guard.connection_limit_events.saturating_add(1);
            push_pulse(
                &mut guard.pulses,
                format!("outgoing connection error peer={peer_id:?} error={error:?}"),
            );
        }
        SwarmEvent::NewListenAddr { address, .. } => {
            let is_relayed = is_p2p_circuit_addr(&address);
            if is_advertisable_listen_addr(&address) {
                swarm.add_external_address(address.clone());
            }
            if is_relayed {
                ctx.relay_state
                    .relayed_listen_addrs
                    .insert(address.to_string());
            }

            let rendezvous_plan = refresh_rendezvous(
                swarm,
                ctx.discovery_cfg,
                ctx.rendezvous_peers,
                ctx.rendezvous_state,
            );

            let mut guard = ctx.snapshot.lock().await;
            guard.public_addr = Some(address.to_string());
            guard.rendezvous_register_attempts = ctx.rendezvous_state.register_attempts;
            guard.rendezvous_register_failures = ctx.rendezvous_state.register_failures;
            guard.rendezvous_discover_attempts = ctx.rendezvous_state.discover_attempts;
            guard.rendezvous_discover_failures = ctx.rendezvous_state.discover_failures;
            for err in rendezvous_plan.errors {
                push_pulse(
                    &mut guard.pulses,
                    format!("rendezvous startup error: {err}"),
                );
            }
            if rendezvous_plan.register_attempts > 0 || rendezvous_plan.discover_attempts > 0 {
                push_pulse(
                    &mut guard.pulses,
                    format!(
                        "rendezvous refresh register_attempts={} discover_attempts={}",
                        rendezvous_plan.register_attempts, rendezvous_plan.discover_attempts
                    ),
                );
            }
            if is_relayed {
                guard.relayed_listen_addresses = ctx
                    .relay_state
                    .relayed_listen_addrs
                    .iter()
                    .cloned()
                    .collect();
                push_pulse(
                    &mut guard.pulses,
                    format!("relay_client relayed listen addr confirmed {address}"),
                );
            }
        }
        SwarmEvent::ExpiredListenAddr { address, .. } => {
            if is_p2p_circuit_addr(&address) {
                swarm.remove_external_address(&address);
                ctx.relay_state
                    .relayed_listen_addrs
                    .remove(&address.to_string());
                let mut guard = ctx.snapshot.lock().await;
                guard.relayed_listen_addresses = ctx
                    .relay_state
                    .relayed_listen_addrs
                    .iter()
                    .cloned()
                    .collect();
                push_pulse(
                    &mut guard.pulses,
                    format!("relay_client relayed listen addr expired {address}"),
                );
            }
        }
        SwarmEvent::ListenerError { error, .. } => {
            ctx.relay_state.server_errors = ctx.relay_state.server_errors.saturating_add(1);
            ctx.relay_state.health = RelayServiceHealth::Error;
            let mut guard = ctx.snapshot.lock().await;
            guard.apply_relay_state(ctx.relay_state);
            push_pulse(&mut guard.pulses, format!("listener error: {error:?}"));
        }
        SwarmEvent::Behaviour(MeshEvent::AutoNat(ev)) => {
            update_nat_state(ctx.relay_state, &ev);
            let mut guard = ctx.snapshot.lock().await;
            guard.nat_status = format!("{ev:?}");
        }
        SwarmEvent::Behaviour(MeshEvent::RelayClient(ev)) => {
            process_relay_client_event(ev, ctx.snapshot, ctx.relay_state).await;
        }
        SwarmEvent::Behaviour(MeshEvent::RelayServer(ev)) => {
            process_relay_server_event(ev, ctx.snapshot, ctx.relay_state).await;
        }
        SwarmEvent::Behaviour(MeshEvent::Dcutr(ev)) => {
            process_dcutr_event(ev, ctx.snapshot, ctx.relay_state).await;
        }
        SwarmEvent::Behaviour(MeshEvent::RendezvousClient(ev)) => {
            let line =
                on_rendezvous_client_event(swarm, &ev, ctx.discovery_cfg, ctx.rendezvous_state);
            let mut guard = ctx.snapshot.lock().await;
            guard.rendezvous_registered_with = ctx.rendezvous_state.registered_with.len();
            guard.rendezvous_discovered_peers = ctx.rendezvous_state.discovered_peers.len();
            guard.rendezvous_register_attempts = ctx.rendezvous_state.register_attempts;
            guard.rendezvous_register_failures = ctx.rendezvous_state.register_failures;
            guard.rendezvous_discover_attempts = ctx.rendezvous_state.discover_attempts;
            guard.rendezvous_discover_failures = ctx.rendezvous_state.discover_failures;
            guard.rendezvous_server_registrations = ctx.rendezvous_state.server_registrations;
            guard.rendezvous_server_discoveries_served =
                ctx.rendezvous_state.server_discoveries_served;
            guard.rendezvous_server_errors = ctx.rendezvous_state.server_errors;
            push_pulse(&mut guard.pulses, line);
        }
        SwarmEvent::Behaviour(MeshEvent::RendezvousServer(ev)) => {
            let line = on_rendezvous_server_event(&ev, ctx.rendezvous_state);
            let mut guard = ctx.snapshot.lock().await;
            guard.rendezvous_server_registrations = ctx.rendezvous_state.server_registrations;
            guard.rendezvous_server_discoveries_served =
                ctx.rendezvous_state.server_discoveries_served;
            guard.rendezvous_server_errors = ctx.rendezvous_state.server_errors;
            push_pulse(&mut guard.pulses, line);
        }
        SwarmEvent::Behaviour(MeshEvent::Gossipsub(libp2p::gossipsub::Event::Message {
            propagation_source,
            message,
            message_id,
        })) if message.topic == *ctx.heartbeat_topic_hash => {
            process_inbound_heartbeat(swarm, propagation_source, message_id, message.data, ctx)
                .await;
        }
        SwarmEvent::Behaviour(MeshEvent::Gossipsub(libp2p::gossipsub::Event::Message {
            propagation_source,
            message_id,
            ..
        })) => {
            swarm
                .behaviour_mut()
                .gossipsub
                .report_message_validation_result(
                    &message_id,
                    &propagation_source,
                    MessageAcceptance::Ignore,
                );
            let mut guard = ctx.snapshot.lock().await;
            guard.gossip_messages_ignored = guard.gossip_messages_ignored.saturating_add(1);
            push_pulse(
                &mut guard.pulses,
                format!("peer {propagation_source} ignored_unexpected_gossip_topic"),
            );
        }
        SwarmEvent::Behaviour(ev) => {
            on_mesh_event(swarm, &ev, ctx.discovery_cfg);
        }
        _ => {}
    }
}

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

async fn process_relay_client_event(
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
            format!("relay_client reservation accepted relay={relay_peer_id} renewal={renewal}")
        }
        relay::client::Event::OutboundCircuitEstablished {
            relay_peer_id,
            limit: _,
        } => format!("relay_client outbound relayed circuit established relay={relay_peer_id}"),
        relay::client::Event::InboundCircuitEstablished {
            src_peer_id,
            limit: _,
        } => {
            format!("relay_client inbound relayed circuit established src={src_peer_id}")
        }
    };

    let mut guard = snapshot.lock().await;
    guard.apply_relay_state(relay_state);
    push_pulse(&mut guard.pulses, line);
}

async fn process_relay_server_event(
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

async fn process_dcutr_event(
    ev: libp2p::dcutr::Event,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    relay_state: &mut RelayState,
) {
    relay_state.dcutr_attempts = relay_state.dcutr_attempts.saturating_add(1);
    let debug = format!("{ev:?}");
    let lower = debug.to_ascii_lowercase();
    if lower.contains("success") || lower.contains("established") {
        relay_state.dcutr_successes = relay_state.dcutr_successes.saturating_add(1);
    }

    let mut guard = snapshot.lock().await;
    guard.apply_relay_state(relay_state);
    push_pulse(&mut guard.pulses, format!("dcutr event {debug}"));
}

async fn process_inbound_heartbeat(
    swarm: &mut Swarm<MeshBehaviour>,
    peer: PeerId,
    msg_id: MessageId,
    data: Vec<u8>,
    ctx: &mut SwarmEventContext<'_>,
) {
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
            ctx.rep.penalize_invalid(peer);
            swarm
                .behaviour_mut()
                .gossipsub
                .report_message_validation_result(&msg_id, &peer, MessageAcceptance::Reject);
            let mut guard = ctx.snapshot.lock().await;
            guard.gossip_messages_rejected = guard.gossip_messages_rejected.saturating_add(1);
            push_pulse(&mut guard.pulses, format!("peer {peer} rejected_oversize"));
        }
        HeartbeatValidationDecision::Reject => {
            ctx.rep.penalize_invalid(peer);
            swarm
                .behaviour_mut()
                .gossipsub
                .report_message_validation_result(&msg_id, &peer, MessageAcceptance::Reject);
            let mut guard = ctx.snapshot.lock().await;
            guard.gossip_messages_rejected = guard.gossip_messages_rejected.saturating_add(1);
            push_pulse(&mut guard.pulses, format!("peer {peer} rejected_heartbeat"));
        }
    }
}

fn is_advertisable_listen_addr(addr: &Multiaddr) -> bool {
    let mut has_ip = false;
    for protocol in addr.iter() {
        match protocol {
            libp2p::multiaddr::Protocol::Ip4(ip) => {
                has_ip = true;
                if ip.is_unspecified() {
                    return false;
                }
            }
            libp2p::multiaddr::Protocol::Ip6(ip) => {
                has_ip = true;
                if ip.is_unspecified() {
                    return false;
                }
            }
            _ => {}
        }
    }
    has_ip || is_p2p_circuit_addr(addr)
}
