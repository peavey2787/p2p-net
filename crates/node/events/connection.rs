use libp2p::swarm::ConnectionId;
use libp2p::{autonat, identify, Multiaddr, PeerId, Swarm};

use crate::connectivity::addr::{is_local_direct_addr, is_public_direct_addr};
use crate::connectivity::peer_cache;
use crate::connectivity::relay::{is_p2p_circuit_addr, update_nat_state, RelayServiceHealth};
use crate::stack::{refresh_rendezvous, MeshBehaviour};

use super::super::push_pulse;
use super::{sync_peer_connectivity_snapshot, SwarmEventContext};

pub(crate) async fn handle_identify_observed_addr(
    swarm: &mut Swarm<MeshBehaviour>,
    ev: &identify::Event,
    ctx: &mut SwarmEventContext<'_>,
) {
    let observed_addr = match ev {
        identify::Event::Received { info, .. } => &info.observed_addr,
        _ => return,
    };

    let classification = classify_listen_addr(observed_addr);
    if classification.advertise_as_external() {
        swarm.add_external_address(observed_addr.clone());
    }

    let mut guard = ctx.snapshot.lock().await;
    record_listen_addr_snapshot(&mut guard, observed_addr, classification);
    match classification {
        ListenAddrClass::PublicDirect => push_pulse(
            &mut guard.pulses,
            format!("identify observed public direct addr {observed_addr}"),
        ),
        ListenAddrClass::Relayed => push_pulse(
            &mut guard.pulses,
            format!("identify observed relayed addr {observed_addr}"),
        ),
        ListenAddrClass::LocalOnly => {}
    }
}

pub(crate) async fn handle_connection_established(
    peer_id: PeerId,
    connection_id: ConnectionId,
    remote_addr: Multiaddr,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) {
    peer_cache::record_seen_peer_addr_with_storage(
        ctx.discovery_cfg,
        &peer_id,
        &remote_addr,
        ctx.storage,
    );

    ctx.peer_book.record_connected(peer_id, Some(remote_addr.clone()));
    ctx.pending_connections.complete(&peer_id);
    ctx.auto_dial_stats.clear_awaiting(&peer_id);

    if ctx.relay_cfg.enabled && !ctx.relay_cfg.schedule.is_open_now_utc() {
        let _ = swarm.close_connection(connection_id);
        ctx.relay_state.health = RelayServiceHealth::ClosedBySchedule;
        let mut guard = ctx.snapshot.lock().await;
        guard.connected_peers = swarm.connected_peers().count();
        sync_peer_connectivity_snapshot(&mut guard, ctx);
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
        .record_established(connection_id, &remote_addr);
    if over_ip_cap {
        let _ = swarm.close_connection(connection_id);
        let mut guard = ctx.snapshot.lock().await;
        guard.connected_peers = swarm.connected_peers().count();
        sync_peer_connectivity_snapshot(&mut guard, ctx);
        guard.connection_limit_events = guard.connection_limit_events.saturating_add(1);
        guard.connection_cap_disconnects = ctx.connection_caps.cap_disconnects;
        push_pulse(
            &mut guard.pulses,
            format!("connection cap exceeded; closing connection {connection_id:?} from {remote_addr}"),
        );
        return;
    }

    let mut guard = ctx.snapshot.lock().await;
    guard.connected_peers = swarm.connected_peers().count();
    sync_peer_connectivity_snapshot(&mut guard, ctx);
    guard.connection_cap_disconnects = ctx.connection_caps.cap_disconnects;
    if is_p2p_circuit_addr(&remote_addr) {
        if ctx.dcutr_policy.keep_relay_fallback {
            ctx.relay_state.dcutr_relay_fallbacks = ctx
                .relay_state
                .dcutr_relay_fallbacks
                .saturating_add(1);
        }

        if ctx.dcutr_policy.enabled && ctx.dcutr_policy.attempt_after_relay_connection {
            let max_attempts = ctx.dcutr_policy.max_attempts_per_peer;
            let attempt_budget = {
                let attempts = ctx
                    .relay_state
                    .dcutr_attempts_by_peer
                    .entry(peer_id.to_owned())
                    .or_insert(0);
                if *attempts >= max_attempts {
                    None
                } else {
                    *attempts = attempts.saturating_add(1);
                    Some(*attempts)
                }
            };

            if let Some(attempt_budget) = attempt_budget {
                ctx.relay_state.dcutr_upgrade_eligible_connections = ctx
                    .relay_state
                    .dcutr_upgrade_eligible_connections
                    .saturating_add(1);
                push_pulse(
                    &mut guard.pulses,
                    format!(
                        "dcutr upgrade eligible for {peer_id} via relay fallback {remote_addr}; attempt_budget={attempt_budget}/{max_attempts}",
                    ),
                );
            } else {
                ctx.relay_state.dcutr_retry_suppressed = ctx
                    .relay_state
                    .dcutr_retry_suppressed
                    .saturating_add(1);
                push_pulse(
                    &mut guard.pulses,
                    format!(
                        "dcutr retry suppressed for {peer_id}; max_attempts_per_peer={max_attempts} reached; relay fallback retained via {remote_addr}",
                    ),
                );
            }
        } else {
            push_pulse(
                &mut guard.pulses,
                format!("relay_fallback connection established via {remote_addr}"),
            );
        }

        guard.apply_relay_state(ctx.relay_state);
    }
}

pub(crate) async fn handle_connection_closed(
    peer_id: PeerId,
    connection_id: ConnectionId,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) {
    ctx.connection_caps.record_closed(connection_id);
    ctx.peer_book.record_disconnected(peer_id);
    let mut guard = ctx.snapshot.lock().await;
    guard.connected_peers = swarm.connected_peers().count();
    sync_peer_connectivity_snapshot(&mut guard, ctx);
    guard.connection_cap_disconnects = ctx.connection_caps.cap_disconnects;
}

pub(crate) async fn handle_incoming_connection_error(
    peer_id_debug: String,
    error_debug: String,
    ctx: &mut SwarmEventContext<'_>,
) {
    let mut guard = ctx.snapshot.lock().await;
    guard.connection_limit_events = guard.connection_limit_events.saturating_add(1);
    push_pulse(
        &mut guard.pulses,
        format!("incoming connection error peer={peer_id_debug} error={error_debug}"),
    );
}

pub(crate) async fn handle_outgoing_connection_error(
    peer_id: Option<PeerId>,
    error_debug: String,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) {
    let mut planner_pulses = Vec::new();
    if let Some(peer) = peer_id.as_ref() {
        peer_cache::record_peer_addr_failure_with_storage(ctx.discovery_cfg, peer, ctx.storage);
        ctx.peer_book.record_failure(peer.to_owned());

        while let Some(attempt) = ctx.pending_connections.next_after_failure(peer) {
            match swarm.dial(attempt.addr.clone()) {
                Ok(()) => {
                    planner_pulses.push(format!(
                        "connection planner fallback dial peer={peer} kind={} addr={}",
                        attempt.kind.as_str(),
                        attempt.addr
                    ));
                    break;
                }
                Err(err) => planner_pulses.push(format!(
                    "connection planner fallback failed immediately peer={peer} kind={} addr={} error={}",
                    attempt.kind.as_str(),
                    attempt.addr,
                    err
                )),
            }
        }
    }
    let mut guard = ctx.snapshot.lock().await;
    guard.connection_limit_events = guard.connection_limit_events.saturating_add(1);
    sync_peer_connectivity_snapshot(&mut guard, ctx);
    push_pulse(
        &mut guard.pulses,
        format!("outgoing connection error peer={peer_id:?} error={error_debug}"),
    );
    for pulse in planner_pulses {
        push_pulse(&mut guard.pulses, pulse);
    }
}

pub(crate) async fn handle_new_listen_addr(
    address: Multiaddr,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) {
    let classification = classify_listen_addr(&address);
    if classification.advertise_as_external() {
        swarm.add_external_address(address.clone());
    }
    if classification.is_relayed() {
        ctx.relay_state
            .relayed_listen_addrs
            .insert(address.to_string());
    }

    let rendezvous_plan = refresh_rendezvous(
        swarm,
        ctx.network_id,
        ctx.discovery_cfg,
        ctx.rendezvous_peers,
        ctx.rendezvous_state,
    );

    let mut guard = ctx.snapshot.lock().await;
    record_listen_addr_snapshot(&mut guard, &address, classification);
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
    match classification {
        ListenAddrClass::PublicDirect => push_pulse(
            &mut guard.pulses,
            format!("public direct listen addr confirmed {address}"),
        ),
        ListenAddrClass::Relayed => push_pulse(
            &mut guard.pulses,
            format!("relay_client relayed listen addr confirmed {address}"),
        ),
        ListenAddrClass::LocalOnly => push_pulse(
            &mut guard.pulses,
            format!("local/private listen addr {address}; not advertised as public reachability"),
        ),
    }
}

pub(crate) async fn handle_expired_listen_addr(
    address: Multiaddr,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) {
    let classification = classify_listen_addr(&address);
    if classification.advertise_as_external() {
        swarm.remove_external_address(&address);
    }
    if classification.is_relayed() {
        ctx.relay_state
            .relayed_listen_addrs
            .remove(&address.to_string());
    }
    let mut guard = ctx.snapshot.lock().await;
    remove_listen_addr_snapshot(&mut guard, &address, classification);
    if classification.is_relayed() {
        push_pulse(
            &mut guard.pulses,
            format!("relay_client relayed listen addr expired {address}"),
        );
    }
}

pub(crate) async fn handle_listener_error(
    error_debug: String,
    ctx: &mut SwarmEventContext<'_>,
) {
    ctx.relay_state.server_errors = ctx.relay_state.server_errors.saturating_add(1);
    ctx.relay_state.health = RelayServiceHealth::Error;
    let mut guard = ctx.snapshot.lock().await;
    guard.apply_relay_state(ctx.relay_state);
    push_pulse(&mut guard.pulses, format!("listener error: {error_debug}"));
}

pub(crate) async fn handle_autonat_event(
    ev: autonat::Event,
    ctx: &mut SwarmEventContext<'_>,
) {
    update_nat_state(ctx.relay_state, &ev);
    let debug = format!("{ev:?}");
    let status = autonat_status_label(&debug);
    let relay_fallback_available = ctx.relay_state.relay_client_reservation_attempts > 0
        || !ctx.relay_state.relay_discovery_selected_relays.is_empty();
    let mut guard = ctx.snapshot.lock().await;
    guard.nat_status = status;
    if debug.contains("NoAddresses") && relay_fallback_available {
        push_pulse(
            &mut guard.pulses,
            "autonat has no public direct address yet; continuing with relay fallback".to_string(),
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListenAddrClass {
    PublicDirect,
    Relayed,
    LocalOnly,
}

impl ListenAddrClass {
    fn advertise_as_external(self) -> bool {
        matches!(self, Self::PublicDirect | Self::Relayed)
    }

    fn is_relayed(self) -> bool {
        matches!(self, Self::Relayed)
    }
}

fn classify_listen_addr(addr: &Multiaddr) -> ListenAddrClass {
    if is_p2p_circuit_addr(addr) {
        ListenAddrClass::Relayed
    } else if is_public_direct_addr(addr) {
        ListenAddrClass::PublicDirect
    } else {
        ListenAddrClass::LocalOnly
    }
}

fn record_listen_addr_snapshot(
    snapshot: &mut super::super::snapshot::NodeSnapshot,
    addr: &Multiaddr,
    classification: ListenAddrClass,
) {
    let addr_string = addr.to_string();
    match classification {
        ListenAddrClass::PublicDirect => {
            push_unique(
                &mut snapshot.public_direct_listen_addresses,
                addr_string.clone(),
            );
            snapshot.public_addr = Some(addr_string);
        }
        ListenAddrClass::Relayed => {
            push_unique(&mut snapshot.relayed_listen_addresses, addr_string.clone());
            snapshot.public_addr = Some(addr_string);
        }
        ListenAddrClass::LocalOnly if is_local_direct_addr(addr) => {
            push_unique(&mut snapshot.local_listen_addresses, addr_string);
        }
        ListenAddrClass::LocalOnly => {}
    }
}

fn remove_listen_addr_snapshot(
    snapshot: &mut super::super::snapshot::NodeSnapshot,
    addr: &Multiaddr,
    classification: ListenAddrClass,
) {
    let addr = addr.to_string();
    match classification {
        ListenAddrClass::PublicDirect => snapshot
            .public_direct_listen_addresses
            .retain(|v| v != &addr),
        ListenAddrClass::Relayed => snapshot.relayed_listen_addresses.retain(|v| v != &addr),
        ListenAddrClass::LocalOnly => snapshot.local_listen_addresses.retain(|v| v != &addr),
    }
    snapshot.public_addr = snapshot
        .relayed_listen_addresses
        .first()
        .cloned()
        .or_else(|| snapshot.public_direct_listen_addresses.first().cloned());
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn autonat_status_label(debug: &str) -> String {
    if debug.contains("NoAddresses") {
        "unknown_no_public_direct_addr_yet".to_string()
    } else if debug.contains("NoServer") {
        "unknown_waiting_for_autonat_server".to_string()
    } else {
        debug.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_listen_addresses_are_not_public_addresses() {
        let docker: Multiaddr = "/ip4/172.17.0.1/udp/4001/quic-v1".parse().unwrap();
        let public: Multiaddr = "/ip4/8.8.8.8/udp/4001/quic-v1".parse().unwrap();

        assert_eq!(classify_listen_addr(&docker), ListenAddrClass::LocalOnly);
        assert_eq!(classify_listen_addr(&public), ListenAddrClass::PublicDirect);
    }

    #[test]
    fn autonat_no_addresses_is_labeled_as_pending_public_direct_addr() {
        assert_eq!(
            autonat_status_label("OutboundProbe(Error { error: NoAddresses })"),
            "unknown_no_public_direct_addr_yet"
        );
    }
}
