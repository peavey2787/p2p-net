use crate::api::PeerSource;
use crate::connectivity::dht::on_kademlia_event;
use crate::connectivity::peer_cache;
use crate::connectivity::relay::is_p2p_circuit_addr;

use super::super::dial::{auto_dial_dht_provider, AutoDialOutcome};
use super::super::push_pulse;
use super::{sync_peer_connectivity_snapshot, SwarmEventContext};
use crate::stack::{add_peer_address_to_discovery, allow_dcutr_peer, MeshBehaviour};
use libp2p::{Multiaddr, PeerId, Swarm};
use std::collections::HashSet;

const APPLICATION_DIAL_OUTGOING_HEADROOM: u32 = 8;

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
        if *provider != ctx.local_peer {
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
        peer_cache::record_seen_peer_addr_with_storage(ctx.discovery_cfg, peer, &addr, ctx.storage);
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
                "dht provider auto-connect released {released} unrelated outbound DHT peer(s) for application dial headroom target={peer}"
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
        .outgoing_connections_to_release(APPLICATION_DIAL_OUTGOING_HEADROOM);
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

pub(crate) async fn handle_event(
    swarm: &mut Swarm<MeshBehaviour>,
    ev: &libp2p::kad::Event,
    ctx: &mut SwarmEventContext<'_>,
) {
    let mut auto_dial_candidates = record_dht_provider_peers(ev, swarm, ctx);
    auto_dial_candidates.extend(record_kademlia_provider_addrs(ev, swarm, ctx));
    let auto_dial_pulses = maybe_auto_dial_dht_providers(auto_dial_candidates, swarm, ctx);

    let Some(line) = on_kademlia_event(swarm, ev, ctx.dht_state) else {
        if !auto_dial_pulses.is_empty() {
            let mut guard = ctx.snapshot.lock().await;
            sync_dht_snapshot(&mut guard, ctx);
            for pulse in auto_dial_pulses {
                push_pulse(&mut guard.pulses, pulse);
            }
        }
        return;
    };

    let mut guard = ctx.snapshot.lock().await;
    sync_dht_snapshot(&mut guard, ctx);
    push_pulse(&mut guard.pulses, line);
    for pulse in auto_dial_pulses {
        push_pulse(&mut guard.pulses, pulse);
    }
}

fn sync_dht_snapshot(
    snapshot: &mut super::super::snapshot::NodeSnapshot,
    ctx: &SwarmEventContext<'_>,
) {
    snapshot.dht_provider_announce_attempts = ctx.dht_state.announce_attempts;
    snapshot.dht_provider_announce_failures = ctx.dht_state.announce_failures;
    snapshot.dht_provider_namespaces_announced = ctx.dht_state.namespaces_announced.len();
    snapshot.dht_provider_queries = ctx.dht_state.provider_queries;
    snapshot.dht_provider_query_failures = ctx.dht_state.provider_query_failures;
    snapshot.dht_provider_records_found = ctx.dht_state.provider_records_found;
    snapshot.dht_provider_queries_finished = ctx.dht_state.provider_queries_finished;
    snapshot.dht_provider_peers_discovered = ctx.dht_state.provider_peer_count();
    sync_peer_connectivity_snapshot(snapshot, ctx);
}
