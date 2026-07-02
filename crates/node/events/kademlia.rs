use crate::api::PeerSource;
use crate::connectivity::dht::on_kademlia_event;

use super::super::dial::{auto_dial_peer_from_book, AutoDialOutcome};
use super::super::push_pulse;
use super::SwarmEventContext;
use crate::stack::MeshBehaviour;
use libp2p::{Multiaddr, PeerId, Swarm};

#[derive(Debug, Clone, Copy)]
struct AutoDialCandidate {
    peer: PeerId,
    address_updated: bool,
}

fn record_dht_provider_peers(
    ev: &libp2p::kad::Event,
    ctx: &mut SwarmEventContext<'_>,
) -> Vec<AutoDialCandidate> {
    let libp2p::kad::Event::OutboundQueryProgressed { id, result, .. } = ev else {
        return Vec::new();
    };
    let libp2p::kad::QueryResult::GetProviders(Ok(
        libp2p::kad::GetProvidersOk::FoundProviders { providers, .. },
    )) = result else {
        return Vec::new();
    };
    let Some(namespace) = ctx.dht_state.provider_namespace(id) else {
        return Vec::new();
    };

    let mut learned = Vec::new();
    for provider in providers {
        ctx.peer_book
            .record_namespace(*provider, namespace.clone(), PeerSource::DhtProvider);
        learned.push(AutoDialCandidate {
            peer: *provider,
            address_updated: false,
        });
    }
    learned
}

fn record_kademlia_provider_addrs(
    ev: &libp2p::kad::Event,
    ctx: &mut SwarmEventContext<'_>,
) -> Vec<AutoDialCandidate> {
    match ev {
        libp2p::kad::Event::RoutingUpdated {
            peer, addresses, ..
        } => record_known_provider_addrs(peer, addresses.iter().cloned(), ctx),
        libp2p::kad::Event::RoutablePeer { peer, address }
        | libp2p::kad::Event::PendingRoutablePeer { peer, address } => {
            record_known_provider_addrs(peer, std::iter::once(address.clone()), ctx)
        }
        _ => Vec::new(),
    }
}

fn record_known_provider_addrs<I>(
    peer: &PeerId,
    addrs: I,
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

    for addr in addrs {
        ctx.peer_book
            .record_addr(*peer, addr, PeerSource::DhtProvider);
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
    let enabled = ctx.discovery_cfg.public_bootstrap.auto_connect_discovered_peers;

    for candidate in peers {
        let peer = candidate.peer;
        let should_attempt = if candidate.address_updated {
            ctx.dht_state.should_auto_connect_after_addr_update(&peer)
        } else {
            ctx.dht_state.should_auto_connect_provider_result(&peer)
        };
        if !should_attempt {
            continue;
        }
        let outcome = auto_dial_peer_from_book(
            peer,
            ctx.local_peer,
            enabled,
            swarm,
            ctx.peer_book,
            ctx.pending_connections,
            ctx.dcutr_policy,
        );

        match &outcome {
            AutoDialOutcome::DialStarted(_) | AutoDialOutcome::DialFailed(_) => {
                ctx.dht_state.mark_auto_connect_attempted(peer);
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

pub(crate) async fn handle_event(
    swarm: &mut Swarm<MeshBehaviour>,
    ev: &libp2p::kad::Event,
    ctx: &mut SwarmEventContext<'_>,
) {
    let mut auto_dial_candidates = record_dht_provider_peers(ev, ctx);
    auto_dial_candidates.extend(record_kademlia_provider_addrs(ev, ctx));
    let auto_dial_pulses = maybe_auto_dial_dht_providers(auto_dial_candidates, swarm, ctx);

    let Some(line) = on_kademlia_event(
        swarm,
        ev,
        ctx.discovery_cfg,
        ctx.storage,
        ctx.dht_state,
    ) else {
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
    snapshot.peer_book_known_peers = ctx.peer_book.len();
    snapshot.peer_book_discovered_peers = ctx.peer_book.discovered_count();
}
