use crate::api::PeerSource;
use crate::connectivity::dht::on_kademlia_event;

use super::super::push_pulse;
use super::SwarmEventContext;
use crate::stack::MeshBehaviour;
use libp2p::Swarm;


fn record_dht_provider_peers(ev: &libp2p::kad::Event, ctx: &mut SwarmEventContext<'_>) {
    let libp2p::kad::Event::OutboundQueryProgressed { id, result, .. } = ev else {
        return;
    };
    let libp2p::kad::QueryResult::GetProviders(Ok(
        libp2p::kad::GetProvidersOk::FoundProviders { providers, .. },
    )) = result else {
        return;
    };
    let Some(namespace) = ctx.dht_state.provider_namespace(id) else {
        return;
    };

    for provider in providers {
        ctx.peer_book
            .record_namespace(provider.to_owned(), namespace.clone(), PeerSource::DhtProvider);
    }
}

pub(crate) async fn handle_event(
    swarm: &mut Swarm<MeshBehaviour>,
    ev: &libp2p::kad::Event,
    ctx: &mut SwarmEventContext<'_>,
) {
    record_dht_provider_peers(ev, ctx);
    let Some(line) = on_kademlia_event(
        swarm,
        ev,
        ctx.discovery_cfg,
        ctx.storage,
        ctx.dht_state,
    ) else {
        return;
    };

    let mut guard = ctx.snapshot.lock().await;
    guard.dht_provider_announce_attempts = ctx.dht_state.announce_attempts;
    guard.dht_provider_announce_failures = ctx.dht_state.announce_failures;
    guard.dht_provider_namespaces_announced = ctx.dht_state.namespaces_announced.len();
    guard.dht_provider_queries = ctx.dht_state.provider_queries;
    guard.dht_provider_query_failures = ctx.dht_state.provider_query_failures;
    guard.dht_provider_records_found = ctx.dht_state.provider_records_found;
    guard.dht_provider_queries_finished = ctx.dht_state.provider_queries_finished;
    guard.dht_provider_peers_discovered = ctx.dht_state.provider_peer_count();
    guard.peer_book_known_peers = ctx.peer_book.len();
    guard.peer_book_discovered_peers = ctx.peer_book.discovered_count();
    push_pulse(&mut guard.pulses, line);
}
