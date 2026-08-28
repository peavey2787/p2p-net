use libp2p::multiaddr::Protocol;
use libp2p::swarm::ConnectionId;
use libp2p::{autonat, Multiaddr, PeerId, Swarm};
use std::time::{Duration, Instant};

use crate::api::PeerSource;
use crate::connectivity::addr::{has_reachable_transport, is_local_direct_addr};
use crate::connectivity::relay::{relay_peer_id, update_nat_state, RelayServiceHealth};
use crate::stack::{
    add_external_address_candidate, add_hole_punch_candidate, refresh_rendezvous, MeshBehaviour,
};

use super::super::push_pulse;
use super::{sync_swarm_connection_snapshot, SwarmEventContext};

mod errors;
mod identify;
mod listen_addr;

pub(crate) use self::errors::{handle_incoming_connection_error, handle_outgoing_connection_error};
pub(crate) use self::identify::handle_identify_observed_addr;
use self::listen_addr::{
    autonat_status_label, classify_listen_addr, record_listen_addr_snapshot,
    remove_listen_addr_snapshot, ListenAddrClass,
};

const MAX_UNVERIFIED_RELAYED_PEERS: usize = 8;

pub(crate) struct EstablishedConnection {
    pub(crate) peer_id: PeerId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) remote_addr: Multiaddr,
    pub(crate) relayed_endpoint: bool,
    pub(crate) outgoing: bool,
    pub(crate) endpoint_debug: String,
}

pub(crate) async fn handle_connection_established(
    connection: EstablishedConnection,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) {
    let EstablishedConnection {
        peer_id,
        connection_id,
        remote_addr,
        relayed_endpoint,
        outgoing,
        endpoint_debug,
    } = connection;
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

    let over_ip_cap =
        ctx.connection_caps
            .record_established(connection_id, peer_id, &remote_addr, outgoing);
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
    let mut evicted_unverified_peer = None;
    if relayed_endpoint && !track_as_application_peer {
        if !ctx
            .relay_state
            .unverified_relayed_peers
            .contains_key(&peer_id)
            && ctx.relay_state.unverified_relayed_peers.len() >= MAX_UNVERIFIED_RELAYED_PEERS
        {
            // A public relay reservation is intentionally reachable by
            // unrelated peers. Rotate the oldest verification candidate
            // instead of rejecting every newcomer once the small bounded pool
            // fills; otherwise random relay traffic can permanently starve the
            // actual application peer before Identify proves compatibility.
            if let Some(oldest_peer) = ctx
                .relay_state
                .unverified_relayed_peers
                .iter()
                .min_by_key(|(_, connected_at)| **connected_at)
                .map(|(peer, _)| *peer)
            {
                ctx.relay_state
                    .unverified_relayed_peers
                    .remove(&oldest_peer);
                let _ = swarm.disconnect_peer_id(oldest_peer);
                evicted_unverified_peer = Some(oldest_peer);
            }
        }
        ctx.relay_state
            .unverified_relayed_peers
            .insert(peer_id, Instant::now());
    } else {
        ctx.relay_state.unverified_relayed_peers.remove(&peer_id);
    }

    if track_as_application_peer {
        ctx.peer_cache_writes
            .record_seen(peer_id, remote_addr.clone());
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
    if let Some(evicted_peer) = evicted_unverified_peer {
        push_pulse(
            &mut guard.pulses,
            format!(
                "rotated unverified relayed peer={evicted_peer} for new verification candidate={peer_id}; slots={MAX_UNVERIFIED_RELAYED_PEERS}"
            ),
        );
    }
    if relayed_endpoint {
        if !track_as_application_peer {
            guard.apply_relay_state(ctx.relay_state);
            push_pulse(
                &mut guard.pulses,
                format!(
                    "relayed connection pending application verification peer={peer_id}; DCUtR deferred"
                ),
            );
            return;
        }

        if ctx.dcutr_policy.keep_relay_fallback {
            ctx.relay_state.dcutr_relay_fallbacks =
                ctx.relay_state.dcutr_relay_fallbacks.saturating_add(1);
        }

        if ctx.dcutr_policy.enabled && ctx.dcutr_policy.attempt_after_relay_connection {
            let max_attempts = ctx.dcutr_policy.max_attempts_per_peer;
            let now = Instant::now();
            let retry_interval = Duration::from_secs(ctx.dcutr_policy.retry_interval_secs.max(1));
            ctx.relay_state.track_dcutr_peer(peer_id);
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
            let attempt_budget = if *attempts >= max_attempts || cooldown_remaining.is_some() {
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
                    | PeerSource::LanDiscovery
            )
        }) || !record.namespaces.is_empty()
    })
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
        if ctx
            .peer_book
            .record(&peer_id)
            .is_some_and(|record| !record.namespaces.is_empty())
        {
            ctx.dht_state.mark_auto_connect_disconnected(&peer_id);
        }
        ctx.peer_book.record_disconnected_if_known(peer_id);
        ctx.relay_state.unverified_relayed_peers.remove(&peer_id);
    }
    let mut guard = ctx.snapshot.lock().await;
    sync_swarm_connection_snapshot(&mut guard, swarm, ctx);
    guard.connection_cap_disconnects = ctx.connection_caps.cap_disconnects;
}

pub(crate) async fn handle_new_listen_addr(
    address: Multiaddr,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) {
    let classification = classify_listen_addr(&address);
    let mut relayed_addr_confirmed_by_reservation = false;
    let mut relayed_addr_pending_reservation = false;
    let relayed_route_public =
        !classification.is_relayed() || relayed_route_has_public_relay_endpoint(&address);
    if matches!(classification, ListenAddrClass::PublicDirect) {
        add_external_address_candidate(swarm, address.clone());
    } else if matches!(classification, ListenAddrClass::LocalOnly) {
        add_hole_punch_candidate(swarm, address.clone());
    }
    if classification.is_relayed() && relayed_route_public {
        if let Some(relay) = relay_peer_id(&address) {
            if ctx.relay_state.relay_client_reservations.contains(&relay) {
                relayed_addr_confirmed_by_reservation = true;
                add_external_address_candidate(swarm, address.clone());
                ctx.relay_state
                    .relayed_listen_addrs
                    .insert(address.to_string());
            } else {
                ctx.relay_state
                    .pending_relay_listen_addrs
                    .entry(relay)
                    .or_default()
                    .insert(address.to_string());
                relayed_addr_pending_reservation = true;
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
    if !classification.is_relayed() || relayed_addr_confirmed_by_reservation {
        record_listen_addr_snapshot(&mut guard, &address, classification);
    }
    guard.apply_relay_state(ctx.relay_state);
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
            if relayed_addr_confirmed_by_reservation {
                format!("relay_client relayed listen addr confirmed {address}")
            } else if relayed_addr_pending_reservation {
                format!("relay_client relayed listen addr pending reservation {address}")
            } else {
                format!("relay_client relayed listen addr ignored non-public relay route {address}")
            },
        ),
        ListenAddrClass::LocalOnly => push_pulse(
            &mut guard.pulses,
            format!("local/private listen addr {address}; not advertised as public reachability"),
        ),
    }
}

fn relayed_route_has_public_relay_endpoint(addr: &Multiaddr) -> bool {
    let mut relay_route = Multiaddr::empty();
    for protocol in addr.iter() {
        if matches!(protocol, Protocol::P2pCircuit) {
            break;
        }
        relay_route.push(protocol);
    }

    has_reachable_transport(&relay_route) && !is_local_direct_addr(&relay_route)
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
        let address_string = address.to_string();
        for pending in ctx.relay_state.pending_relay_listen_addrs.values_mut() {
            pending.remove(&address_string);
        }
        ctx.relay_state
            .pending_relay_listen_addrs
            .retain(|_, pending| !pending.is_empty());
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
