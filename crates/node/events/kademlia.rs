use crate::api::PeerSource;
use crate::connectivity::dht::on_kademlia_event;
use crate::connectivity::relay::is_p2p_circuit_addr;

use super::super::dial::{auto_dial_dht_provider, AutoDialOutcome};
use super::SwarmEventContext;
use crate::stack::{add_peer_address_to_discovery, allow_dcutr_peer, MeshBehaviour};
use libp2p::{Multiaddr, PeerId, Swarm};
use std::collections::HashSet;

const APPLICATION_DIAL_REQUIRED_HEADROOM: u32 = 1;

#[derive(Debug, Clone, Copy)]
struct AutoDialCandidate {
    peer: PeerId,
    address_updated: bool,
}

fn record_dht_provider_peers(
    ev: &libp2p::kad::Event,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) -> Vec<AutoDialCandidate> {
    let libp2p::kad::Event::OutboundQueryProgressed { id, result, .. } = ev else {
        return Vec::new();
    };
    let libp2p::kad::QueryResult::GetProviders(Ok(libp2p::kad::GetProvidersOk::FoundProviders {
        providers,
        ..
    })) = result
    else {
        return Vec::new();
    };
    let Some(namespace) = ctx.dht_state.provider_namespace(id) else {
        return Vec::new();
    };

    let mut learned = Vec::new();
    for provider in providers {
        ctx.peer_book
            .record_namespace(*provider, namespace.clone(), PeerSource::DhtProvider);
        allow_dcutr_peer(swarm, *provider);
        if swarm.is_connected(provider) {
            // Provider discovery can finish after an inbound relayed
            // connection has already arrived. The exact namespace record is
            // sufficient to promote that live connection immediately.
            ctx.peer_book.record_connected(*provider, None);
            ctx.relay_state.unverified_relayed_peers.remove(provider);
        }
        let has_known_addr = ctx
            .peer_book
            .record(provider)
            .is_some_and(|record| !record.addresses.is_empty());
        if *provider != ctx.local_peer && !swarm.is_connected(provider) && !has_known_addr {
            ctx.dht_state
                .start_provider_addr_lookup_if_due(swarm, *provider);
        }
        learned.push(AutoDialCandidate {
            peer: *provider,
            address_updated: false,
        });
    }
    learned
}

fn record_kademlia_provider_addrs(
    ev: &libp2p::kad::Event,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) -> Vec<AutoDialCandidate> {
    match ev {
        libp2p::kad::Event::OutboundQueryProgressed { id, result, .. } => {
            record_provider_addr_lookup_result(id, result, swarm, ctx)
        }
        libp2p::kad::Event::RoutingUpdated {
            peer, addresses, ..
        } => record_known_provider_addrs(peer, addresses.iter().cloned(), swarm, ctx),
        libp2p::kad::Event::RoutablePeer { peer, address }
        | libp2p::kad::Event::PendingRoutablePeer { peer, address } => {
            record_known_provider_addrs(peer, std::iter::once(address.clone()), swarm, ctx)
        }
        _ => Vec::new(),
    }
}

fn record_provider_addr_lookup_result(
    id: &libp2p::kad::QueryId,
    result: &libp2p::kad::QueryResult,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) -> Vec<AutoDialCandidate> {
    let Some(target_peer) = ctx.dht_state.provider_addr_lookup_peer(id) else {
        return Vec::new();
    };

    let closest_peers = match result {
        libp2p::kad::QueryResult::GetClosestPeers(Ok(ok)) => &ok.peers,
        libp2p::kad::QueryResult::GetClosestPeers(Err(
            libp2p::kad::GetClosestPeersError::Timeout { peers, .. },
        )) => peers,
        _ => return Vec::new(),
    };

    for info in closest_peers {
        for addr in &info.addrs {
            add_peer_address_to_discovery(swarm, info.peer_id, addr.clone());
        }
    }

    let target_addrs = closest_peers
        .iter()
        .filter(|info| info.peer_id == target_peer)
        .flat_map(|info| info.addrs.iter().cloned())
        .collect::<Vec<_>>();
    if target_addrs.is_empty() {
        return Vec::new();
    }

    record_known_provider_addrs(&target_peer, target_addrs, swarm, ctx)
}

fn record_known_provider_addrs<I>(
    peer: &PeerId,
    addrs: I,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) -> Vec<AutoDialCandidate>
where
    I: IntoIterator<Item = Multiaddr>,
{
    let is_dht_provider = ctx
        .peer_book
        .record(peer)
        .map(|record| record.sources.contains(&PeerSource::DhtProvider))
        .unwrap_or(false);
    if !is_dht_provider {
        return Vec::new();
    }
    allow_dcutr_peer(swarm, *peer);

    let mut relay_preferred = false;
    for addr in addrs {
        add_peer_address_to_discovery(swarm, *peer, addr.clone());
        relay_preferred |= is_p2p_circuit_addr(&addr);
        ctx.peer_book
            .record_addr(*peer, addr.clone(), PeerSource::DhtProvider);
        ctx.peer_cache_writes.record_seen(*peer, addr.clone());
    }
    if relay_preferred {
        ctx.peer_book.record_relay_preferred(*peer, true);
    }
    vec![AutoDialCandidate {
        peer: *peer,
        address_updated: true,
    }]
}

fn maybe_auto_dial_dht_providers(
    peers: impl IntoIterator<Item = AutoDialCandidate>,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) -> Vec<String> {
    let mut pulses = Vec::new();
    let enabled = ctx
        .discovery_cfg
        .public_bootstrap
        .auto_connect_discovered_peers;

    for candidate in peers {
        let peer = candidate.peer;
        if ctx.auto_dial_stats.is_suppressed(&peer) {
            continue;
        }
        let should_attempt = if candidate.address_updated {
            ctx.dht_state.should_auto_connect_after_addr_update(&peer)
        } else {
            ctx.dht_state.should_auto_connect_provider_result(&peer)
        };
        if !should_attempt {
            continue;
        }
        let released = if peer == ctx.local_peer {
            0
        } else {
            release_outbound_dht_infrastructure_for_app_dial(peer, swarm, ctx)
        };
        if released > 0 {
            pulses.push(format!(
                "dht provider auto-connect released {released} unrelated outbound DHT peer(s) because the outbound cap blocked application dial target={peer}"
            ));
        }
        let outcome = auto_dial_dht_provider(
            peer,
            ctx.local_peer,
            enabled,
            swarm,
            ctx.peer_book,
            ctx.pending_connections,
            ctx.dcutr_policy,
        );

        ctx.auto_dial_stats.record_outcome(&peer, &outcome);
        match &outcome {
            AutoDialOutcome::DialStarted(_) | AutoDialOutcome::AddressResolutionStarted(_) => {
                ctx.dht_state.mark_auto_connect_attempted(peer);
            }
            AutoDialOutcome::DialFailed(_) => {
                ctx.dht_state.mark_auto_connect_attempted(peer);
                ctx.dht_state.mark_auto_connect_failed(&peer);
            }
            AutoDialOutcome::AwaitingAddress => {
                ctx.dht_state.mark_auto_connect_waiting_for_addrs(peer);
            }
            _ => {}
        }
        if outcome.should_pulse() {
            pulses.push(format!("dht provider {}", outcome.describe(&peer)));
        }
    }
    pulses
}

fn release_outbound_dht_infrastructure_for_app_dial(
    target: PeerId,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) -> usize {
    let to_release = ctx
        .connection_caps
        .outgoing_connections_to_release(APPLICATION_DIAL_REQUIRED_HEADROOM);
    if to_release == 0 {
        return 0;
    }

    let mut seen = HashSet::new();
    let victims = ctx
        .connection_caps
        .outgoing_peers()
        .filter(|peer| *peer != target)
        // After the Identify retention fix, unrelated public DHT-routing
        // peers are precisely the outbound peers absent from PeerBook.
        .filter(|peer| ctx.peer_book.record(peer).is_none())
        .filter(|peer| !ctx.relay_state.relay_client_reservations.contains(peer))
        .filter(|peer| !ctx.relay_state.relay_client_attempted_peers.contains(peer))
        .filter(|peer| seen.insert(*peer))
        .take(to_release)
        .collect::<Vec<_>>();

    for peer in &victims {
        let _ = swarm.disconnect_peer_id(*peer);
    }
    victims.len()
}

pub(crate) fn handle_event(
    swarm: &mut Swarm<MeshBehaviour>,
    ev: &libp2p::kad::Event,
    ctx: &mut SwarmEventContext<'_>,
) {
    // libp2p has already serviced inbound Kademlia requests before emitting the
    // event. They do not alter p2p-net provider/app state, so keep the full
    // server capability while avoiding application-side bookkeeping per request.
    if matches!(
        ev,
        libp2p::kad::Event::InboundRequest { .. }
            | libp2p::kad::Event::ModeChanged { .. }
            | libp2p::kad::Event::UnroutablePeer { .. }
    ) {
        return;
    }

    let mut auto_dial_candidates = record_dht_provider_peers(ev, swarm, ctx);
    auto_dial_candidates.extend(record_kademlia_provider_addrs(ev, swarm, ctx));
    let peer_connectivity_changed = !auto_dial_candidates.is_empty();
    let auto_dial_pulses = maybe_auto_dial_dht_providers(auto_dial_candidates, swarm, ctx);

    if let Some(line) = on_kademlia_event(swarm, ev, ctx.dht_state) {
        ctx.observability.dht_dirty();
        ctx.observability.pulse(line);
    }
    if peer_connectivity_changed {
        ctx.observability.peer_connectivity_dirty();
    }
    for pulse in auto_dial_pulses {
        ctx.observability.pulse(pulse);
    }
}
