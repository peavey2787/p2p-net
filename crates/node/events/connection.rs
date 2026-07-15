use libp2p::multiaddr::Protocol;
use libp2p::swarm::ConnectionId;
use libp2p::{autonat, identify, Multiaddr, PeerId, Swarm};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crate::api::PeerSource;
use crate::connectivity::addr::is_local_direct_addr;
use crate::connectivity::peer_cache;
use crate::connectivity::relay::{
    relay_peer_id, relay_reservation_addr, update_nat_state, RelayServiceHealth,
};
use crate::connectivity::relay_discovery::{
    relay_candidate_addr, supported_relay_addr_score, RelayCandidateSource,
};
use crate::stack::{
    add_external_address_candidate, add_hole_punch_candidate, add_peer_address_to_discovery,
    refresh_rendezvous, MeshBehaviour,
};

use super::super::push_pulse;
use super::{sync_swarm_connection_snapshot, SwarmEventContext};

mod listen_addr;

use self::listen_addr::{
    autonat_status_label, classify_listen_addr, record_listen_addr_snapshot,
    remove_listen_addr_snapshot, ListenAddrClass,
};

const MAX_PUBLIC_DHT_RELAY_ATTEMPTS: usize = 8;

pub(crate) async fn handle_identify_observed_addr(
    swarm: &mut Swarm<MeshBehaviour>,
    ev: &identify::Event,
    ctx: &mut SwarmEventContext<'_>,
) {
    let (peer_id, info) = match ev {
        identify::Event::Received { peer_id, info, .. } => (peer_id, info),
        _ => return,
    };
    let protocols = info
        .protocols
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let supports_relay_hop = protocols
        .iter()
        .any(|protocol| protocol == "/libp2p/circuit/relay/0.2.0/hop");
    let supports_rendezvous = protocols
        .iter()
        .any(|protocol| protocol.starts_with("/rendezvous/"));
    let supports_dcutr = protocols
        .iter()
        .any(|protocol| protocol.starts_with("/libp2p/dcutr"));
    let relay_pulse = if supports_relay_hop {
        maybe_reserve_dht_relay(*peer_id, info, swarm, ctx)
    } else {
        None
    };
    if ctx.peer_book.record(peer_id).is_some() || relay_pulse.is_some() {
        ctx.peer_book.record_capabilities(
            *peer_id,
            Some(supports_relay_hop),
            Some(supports_rendezvous),
            Some(supports_dcutr),
        );
    }

    let observed_addr = &info.observed_addr;
    let classification = classify_listen_addr(observed_addr);
    if classification.advertise_as_external() {
        add_external_address_candidate(swarm, observed_addr.clone());
    }

    let mut guard = ctx.snapshot.lock().await;
    record_listen_addr_snapshot(&mut guard, observed_addr, classification);
    guard.apply_relay_state(ctx.relay_state);
    if let Some(pulse) = relay_pulse {
        push_pulse(&mut guard.pulses, pulse);
    }
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

fn maybe_reserve_dht_relay(
    peer_id: PeerId,
    info: &identify::Info,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) -> Option<String> {
    if peer_id == *swarm.local_peer_id() {
        return None;
    }
    let policy = &ctx.discovery_cfg.relay_discovery;
    if !policy.enabled
        || !policy.use_dht_relays
        || !ctx.discovery_cfg.public_bootstrap.mode.is_enabled()
        || ctx.relay_state.relay_client_reservations.len() >= policy.min_reservations
        || ctx.relay_state.relay_client_attempted_peers.len() >= MAX_PUBLIC_DHT_RELAY_ATTEMPTS
        || ctx
            .relay_state
            .relay_client_attempted_peers
            .contains(&peer_id)
    {
        return None;
    }

    let candidate = info
        .listen_addrs
        .iter()
        .filter(|addr| !is_local_direct_addr(addr))
        .filter_map(|addr| {
            let mut addr = addr.clone();
            if !addr
                .iter()
                .any(|protocol| matches!(protocol, Protocol::P2p(_)))
            {
                addr.push(Protocol::P2p(peer_id));
            }
            relay_candidate_addr(addr, RelayCandidateSource::PublicFallback)
        })
        .filter_map(|candidate| {
            supported_relay_addr_score(&candidate.addr).map(|score| (score, candidate))
        })
        .min_by_key(|(score, _)| *score)?
        .1;
    let reservation_addr = relay_reservation_addr(&candidate.addr)?;

    add_peer_address_to_discovery(swarm, peer_id, candidate.addr.clone());
    ctx.relay_state.relay_client_attempted_peers.insert(peer_id);
    ctx.relay_state.relay_discovery_candidate_count = ctx
        .relay_state
        .relay_discovery_candidate_count
        .saturating_add(1);
    ctx.relay_state.relay_discovery_public_candidates = ctx
        .relay_state
        .relay_discovery_public_candidates
        .saturating_add(1);
    ctx.relay_state
        .relay_discovery_selected_relays
        .insert(candidate.addr.to_string());

    match swarm.listen_on(reservation_addr.clone()) {
        Ok(_) => {
            ctx.relay_state.relay_client_reservation_attempts = ctx
                .relay_state
                .relay_client_reservation_attempts
                .saturating_add(1);
            Some(format!(
                "relay_discovery dht reservation requested relay={peer_id} addr={reservation_addr}"
            ))
        }
        Err(err) => {
            ctx.relay_state.relay_client_reservation_failures = ctx
                .relay_state
                .relay_client_reservation_failures
                .saturating_add(1);
            ctx.relay_state.relay_discovery_failures =
                ctx.relay_state.relay_discovery_failures.saturating_add(1);
            Some(format!(
                "relay_discovery dht reservation failed relay={peer_id} addr={reservation_addr} error={err}"
            ))
        }
    }
}

pub(crate) async fn handle_connection_established(
    peer_id: PeerId,
    connection_id: ConnectionId,
    remote_addr: Multiaddr,
    relayed_endpoint: bool,
    endpoint_debug: String,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) {
    if ctx.relay_cfg.enabled && !ctx.relay_cfg.schedule.is_open_now_utc() {
        let _ = swarm.close_connection(connection_id);
        ctx.relay_state.health = RelayServiceHealth::ClosedBySchedule;
        let mut guard = ctx.snapshot.lock().await;
        sync_swarm_connection_snapshot(&mut guard, swarm, ctx);
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
        ctx.metrics
            .record_choked_peers(ctx.connection_caps.cap_disconnects);
        let mut guard = ctx.snapshot.lock().await;
        sync_swarm_connection_snapshot(&mut guard, swarm, ctx);
        guard.connection_limit_events = guard.connection_limit_events.saturating_add(1);
        guard.connection_cap_disconnects = ctx.connection_caps.cap_disconnects;
        push_pulse(
            &mut guard.pulses,
            format!(
                "connection cap exceeded; closing connection {connection_id:?} from {remote_addr}"
            ),
        );
        return;
    }

    let track_as_application_peer = should_track_peer_in_peer_book(peer_id, ctx);
    if relayed_endpoint && !is_intended_relayed_destination(peer_id, ctx) {
        let _ = swarm.close_connection(connection_id);
        ctx.relay_state.denied_circuits = ctx.relay_state.denied_circuits.saturating_add(1);
        let mut guard = ctx.snapshot.lock().await;
        sync_swarm_connection_snapshot(&mut guard, swarm, ctx);
        guard.connection_limit_events = guard.connection_limit_events.saturating_add(1);
        guard.apply_relay_state(ctx.relay_state);
        push_pulse(
            &mut guard.pulses,
            format!(
                "relayed connection closed for non-application peer={peer_id}; relay infrastructure remains allowed"
            ),
        );
        return;
    }

    if track_as_application_peer {
        peer_cache::record_seen_peer_addr_with_storage(
            ctx.discovery_cfg,
            &peer_id,
            &remote_addr,
            ctx.storage,
        );
        ctx.metrics
            .record_storage_write(remote_addr.to_string().len());
        ctx.peer_book
            .record_connected(peer_id, Some(remote_addr.clone()));
    }
    ctx.metrics
        .record_connection_handshake(track_as_application_peer.then_some(peer_id));
    ctx.pending_connections.complete(&peer_id);
    ctx.auto_dial_stats.clear_awaiting(&peer_id);

    let mut guard = ctx.snapshot.lock().await;
    sync_swarm_connection_snapshot(&mut guard, swarm, ctx);
    guard.connection_cap_disconnects = ctx.connection_caps.cap_disconnects;
    ctx.metrics
        .record_choked_peers(ctx.connection_caps.cap_disconnects);
    push_pulse(
        &mut guard.pulses,
        format!("connection endpoint peer={peer_id} relayed={relayed_endpoint} {endpoint_debug}"),
    );
    if relayed_endpoint {
        if ctx.dcutr_policy.keep_relay_fallback {
            ctx.relay_state.dcutr_relay_fallbacks =
                ctx.relay_state.dcutr_relay_fallbacks.saturating_add(1);
        }

        if ctx.dcutr_policy.enabled && ctx.dcutr_policy.attempt_after_relay_connection {
            let max_attempts = ctx.dcutr_policy.max_attempts_per_peer;
            let now = Instant::now();
            let retry_interval = Duration::from_secs(ctx.dcutr_policy.retry_interval_secs.max(1));
            let attempts = ctx
                .relay_state
                .dcutr_attempts_by_peer
                .entry(peer_id.to_owned())
                .or_insert(0);
            let cooldown_remaining = ctx
                .relay_state
                .dcutr_last_attempt_by_peer
                .get(&peer_id)
                .and_then(|last| retry_interval.checked_sub(now.duration_since(*last)));
            let attempt_budget = if *attempts >= max_attempts {
                None
            } else if cooldown_remaining.is_some() {
                None
            } else {
                *attempts = attempts.saturating_add(1);
                ctx.relay_state
                    .dcutr_last_attempt_by_peer
                    .insert(peer_id.to_owned(), now);
                Some(*attempts)
            };

            if let Some(attempt_budget) = attempt_budget {
                ctx.relay_state.dcutr_upgrade_eligible_connections = ctx
                    .relay_state
                    .dcutr_upgrade_eligible_connections
                    .saturating_add(1);
                ctx.relay_state.dcutr_attempts = ctx.relay_state.dcutr_attempts.saturating_add(1);
                push_pulse(
                    &mut guard.pulses,
                    format!(
                        "dcutr upgrade eligible for {peer_id} via relay fallback {remote_addr}; attempt_budget={attempt_budget}/{max_attempts}",
                    ),
                );
            } else {
                ctx.relay_state.dcutr_retry_suppressed =
                    ctx.relay_state.dcutr_retry_suppressed.saturating_add(1);
                push_pulse(
                    &mut guard.pulses,
                    dcutr_suppressed_pulse(peer_id, max_attempts, cooldown_remaining, &remote_addr),
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

fn dcutr_suppressed_pulse(
    peer_id: PeerId,
    max_attempts: u32,
    cooldown_remaining: Option<Duration>,
    remote_addr: &Multiaddr,
) -> String {
    if let Some(remaining) = cooldown_remaining {
        return format!(
            "dcutr retry suppressed for {peer_id}; retry cooldown remaining={}s; relay fallback retained via {remote_addr}",
            remaining.as_secs().saturating_add(1)
        );
    }
    format!(
        "dcutr retry suppressed for {peer_id}; max_attempts_per_peer={max_attempts} reached; relay fallback retained via {remote_addr}",
    )
}

fn should_track_peer_in_peer_book(peer_id: PeerId, ctx: &SwarmEventContext<'_>) -> bool {
    ctx.peer_book.record(&peer_id).is_some_and(|record| {
        record.sources.iter().any(|source| {
            matches!(
                source,
                PeerSource::Manual
                    | PeerSource::PeerCache
                    | PeerSource::Rendezvous
                    | PeerSource::PublicRendezvous
                    | PeerSource::DhtProvider
            )
        }) || !record.namespaces.is_empty()
    })
}

fn is_intended_relayed_destination(peer_id: PeerId, ctx: &SwarmEventContext<'_>) -> bool {
    let namespaces = ctx
        .discovery_cfg
        .rendezvous_namespaces(ctx.network_id)
        .map(|items| items.into_iter().collect::<BTreeSet<_>>())
        .unwrap_or_default();
    ctx.peer_book
        .has_application_namespace(&peer_id, &namespaces)
        || should_track_peer_in_peer_book(peer_id, ctx)
}

pub(crate) async fn handle_connection_closed(
    peer_id: PeerId,
    connection_id: ConnectionId,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) {
    ctx.connection_caps.record_closed(connection_id);
    if !swarm
        .connected_peers()
        .any(|connected| connected == &peer_id)
    {
        ctx.peer_book.record_disconnected_if_known(peer_id);
    }
    let mut guard = ctx.snapshot.lock().await;
    sync_swarm_connection_snapshot(&mut guard, swarm, ctx);
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
        if ctx.peer_book.record(peer).is_some() {
            peer_cache::record_peer_addr_failure_with_storage(ctx.discovery_cfg, peer, ctx.storage);
            ctx.peer_book.record_failure(peer.to_owned());
        }

        let mut fallback_dial_started = false;
        while let Some(attempt) = ctx.pending_connections.next_after_failure(peer) {
            match swarm.dial(attempt.addr.clone()) {
                Ok(()) => {
                    fallback_dial_started = true;
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
        if !fallback_dial_started && ctx.dht_state.mark_auto_connect_failed(peer) {
            ctx.auto_dial_stats.record_async_failure(peer);
            planner_pulses.push(format!(
                "dht provider auto-connect retry scheduled peer={peer} on next provider result"
            ));
        }
    }
    let mut guard = ctx.snapshot.lock().await;
    guard.connection_limit_events = guard.connection_limit_events.saturating_add(1);
    sync_swarm_connection_snapshot(&mut guard, swarm, ctx);
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
        add_external_address_candidate(swarm, address.clone());
    } else if matches!(classification, ListenAddrClass::LocalOnly) {
        add_hole_punch_candidate(swarm, address.clone());
    }
    if classification.is_relayed() {
        if let Some(relay) = relay_peer_id(&address) {
            if ctx.relay_state.relay_client_reservations.contains(&relay) {
                ctx.relay_state
                    .relayed_listen_addrs
                    .insert(address.to_string());
            } else {
                ctx.relay_state
                    .pending_relay_listen_addrs
                    .entry(relay)
                    .or_default()
                    .insert(address.to_string());
            }
        }
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
        for pending in ctx.relay_state.pending_relay_listen_addrs.values_mut() {
            pending.remove(&address.to_string());
        }
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

pub(crate) async fn handle_listener_error(error_debug: String, ctx: &mut SwarmEventContext<'_>) {
    ctx.relay_state.server_errors = ctx.relay_state.server_errors.saturating_add(1);
    ctx.relay_state.health = RelayServiceHealth::Error;
    let mut guard = ctx.snapshot.lock().await;
    guard.apply_relay_state(ctx.relay_state);
    push_pulse(&mut guard.pulses, format!("listener error: {error_debug}"));
}

pub(crate) async fn handle_autonat_event(ev: autonat::Event, ctx: &mut SwarmEventContext<'_>) {
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
