use libp2p::multiaddr::Protocol;
use libp2p::{identify, Multiaddr, PeerId, Swarm};

use crate::api::PeerSource;
use crate::connectivity::addr::is_local_direct_addr;
use crate::connectivity::relay::{relay_peer_id, relay_reservation_addr, RelayState};
use crate::connectivity::relay_discovery::{
    relay_candidate_addr, supported_relay_addr_score, RelayCandidateSource,
};
use crate::stack::{add_external_address_candidate, add_peer_address_to_discovery, MeshBehaviour};

use super::super::super::push_pulse;
use super::super::{sync_swarm_connection_snapshot, SwarmEventContext};
use super::listen_addr::{classify_listen_addr, record_listen_addr_snapshot, ListenAddrClass};
use super::relayed_route_has_public_relay_endpoint;

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
    let application_compatible = info.protocol_version == ctx.application_protocol_version;
    let was_pending_relay_verification = ctx
        .relay_state
        .unverified_relayed_peers
        .contains_key(peer_id);
    if application_compatible {
        for namespace in ctx.application_namespaces {
            ctx.peer_book.record_namespace(
                *peer_id,
                namespace.clone(),
                PeerSource::Connected,
            );
        }
        ctx.peer_book.record_connected(*peer_id, None);
        ctx.relay_state.unverified_relayed_peers.remove(peer_id);
        crate::stack::allow_dcutr_peer(swarm, *peer_id);
    }
    // Identify can be one of the busiest paths while public DHT queries are
    // converging. Inspect borrowed protocol strings directly instead of
    // allocating a Vec<String> for every Identify response.
    let supports_relay_hop = info
        .protocols
        .iter()
        .any(|protocol| protocol.as_ref() == "/libp2p/circuit/relay/0.2.0/hop");
    let supports_rendezvous = info
        .protocols
        .iter()
        .any(|protocol| protocol.as_ref().starts_with("/rendezvous/"));
    let supports_dcutr = info
        .protocols
        .iter()
        .any(|protocol| protocol.as_ref().starts_with("/libp2p/dcutr"));
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
    let observed_addr_changed = ctx
        .identify_addresses
        .record_observed_local_addr(observed_addr);
    if should_advertise_observed_addr(observed_addr, classification, ctx.relay_state)
        && (observed_addr_changed || classification.is_relayed())
    {
        // A relayed address can become valid after its reservation is confirmed,
        // so re-check relayed observations even when the multiaddr is unchanged.
        // ExternalAddressCandidates performs its own bounded deduplication.
        add_external_address_candidate(swarm, observed_addr.clone());
    }

    if !observed_addr_changed && relay_pulse.is_none() && !application_compatible {
        return;
    }

    let mut guard = ctx.snapshot.lock().await;
    if observed_addr_changed {
        record_listen_addr_snapshot(&mut guard, observed_addr, classification);
    }
    guard.apply_relay_state(ctx.relay_state);
    if let Some(pulse) = relay_pulse {
        push_pulse(&mut guard.pulses, pulse);
    }
    if application_compatible {
        sync_swarm_connection_snapshot(&mut guard, swarm, ctx);
        push_pulse(
            &mut guard.pulses,
            if was_pending_relay_verification {
                format!(
                    "application compatibility verified peer={peer_id} on relayed connection; exact namespace match"
                )
            } else {
                format!("application compatibility verified peer={peer_id}; exact namespace match")
            },
        );
    }
    if observed_addr_changed {
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
}

fn should_advertise_observed_addr(
    addr: &Multiaddr,
    classification: ListenAddrClass,
    relay_state: &RelayState,
) -> bool {
    match classification {
        ListenAddrClass::PublicDirect => true,
        ListenAddrClass::Relayed => {
            relayed_route_has_public_relay_endpoint(addr)
                && relay_peer_id(addr)
                    .is_some_and(|relay| relay_state.relay_client_reservations.contains(&relay))
        }
        ListenAddrClass::LocalOnly => false,
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
