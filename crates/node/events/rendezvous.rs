use std::collections::HashSet;

use libp2p::Swarm;

use crate::api::PeerSource;
use crate::connectivity::rendezvous::peer_record_addrs;
use libp2p_rendezvous as rendezvous;

use crate::stack::{on_rendezvous_client_event, on_rendezvous_server_event, MeshBehaviour};

use super::super::dial::auto_dial_peer_from_book;
use super::super::push_pulse;
use super::SwarmEventContext;

pub(crate) async fn handle_client_event(
    swarm: &mut Swarm<MeshBehaviour>,
    ev: &rendezvous::client::Event,
    ctx: &mut SwarmEventContext<'_>,
) {
    let auto_dial_candidates = record_rendezvous_discovery(ev, ctx);
    let line = on_rendezvous_client_event(
        swarm,
        ev,
        ctx.discovery_cfg,
        ctx.storage,
        ctx.rendezvous_state,
    );
    let auto_dial_pulses = maybe_auto_dial_rendezvous_peers(auto_dial_candidates, swarm, ctx);
    let mut guard = ctx.snapshot.lock().await;
    sync_rendezvous_snapshot(&mut guard, ctx);
    push_pulse(&mut guard.pulses, line);
    for pulse in auto_dial_pulses {
        push_pulse(&mut guard.pulses, pulse);
    }
}

pub(crate) async fn handle_server_event(
    ev: &rendezvous::server::Event,
    ctx: &mut SwarmEventContext<'_>,
) {
    let line = on_rendezvous_server_event(ev, ctx.rendezvous_state);
    let mut guard = ctx.snapshot.lock().await;
    sync_rendezvous_snapshot(&mut guard, ctx);
    push_pulse(&mut guard.pulses, line);
}

fn record_rendezvous_discovery(
    ev: &rendezvous::client::Event,
    ctx: &mut SwarmEventContext<'_>,
) -> Vec<libp2p::PeerId> {
    let rendezvous::client::Event::Discovered {
        rendezvous_node,
        registrations,
        ..
    } = ev
    else {
        return Vec::new();
    };

    let source = discovered_peer_source(rendezvous_node, ctx);
    let mut peers = Vec::new();
    let mut seen = HashSet::new();
    for registration in registrations {
        let peer = registration.record.peer_id();
        let namespace = registration.namespace.to_string();
        ctx.peer_book.record_namespace(peer, namespace, source);
        for addr in peer_record_addrs(registration) {
            ctx.peer_book.record_addr(peer, addr, source);
        }
        if seen.insert(peer) {
            peers.push(peer);
        }
    }
    peers
}

fn discovered_peer_source(
    rendezvous_node: &libp2p::PeerId,
    ctx: &SwarmEventContext<'_>,
) -> PeerSource {
    let discovered_from_public_rendezvous = ctx
        .peer_book
        .record(rendezvous_node)
        .map(|record| record.sources.contains(&PeerSource::PublicRendezvous))
        .unwrap_or(false);
    if discovered_from_public_rendezvous {
        PeerSource::PublicRendezvous
    } else {
        PeerSource::Rendezvous
    }
}

fn maybe_auto_dial_rendezvous_peers(
    peers: impl IntoIterator<Item = libp2p::PeerId>,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) -> Vec<String> {
    let mut pulses = Vec::new();
    let enabled = ctx.discovery_cfg.public_bootstrap.auto_connect_discovered_peers;

    for peer in peers {
        let outcome = auto_dial_peer_from_book(
            peer,
            ctx.local_peer,
            enabled,
            swarm,
            ctx.peer_book,
            ctx.pending_connections,
            ctx.dcutr_policy,
        );
        if outcome.should_pulse() {
            pulses.push(format!("rendezvous {}", outcome.describe(&peer)));
        }
    }
    pulses
}

fn sync_rendezvous_snapshot(
    snapshot: &mut super::super::snapshot::NodeSnapshot,
    ctx: &SwarmEventContext<'_>,
) {
    snapshot.rendezvous_registered_with = ctx.rendezvous_state.registered_with.len();
    snapshot.rendezvous_discovered_peers = ctx.rendezvous_state.discovered_peers.len();
    snapshot.rendezvous_register_attempts = ctx.rendezvous_state.register_attempts;
    snapshot.rendezvous_register_failures = ctx.rendezvous_state.register_failures;
    snapshot.rendezvous_discover_attempts = ctx.rendezvous_state.discover_attempts;
    snapshot.rendezvous_discover_failures = ctx.rendezvous_state.discover_failures;
    snapshot.rendezvous_server_registrations = ctx.rendezvous_state.server_registrations;
    snapshot.rendezvous_server_discoveries_served = ctx.rendezvous_state.server_discoveries_served;
    snapshot.rendezvous_server_errors = ctx.rendezvous_state.server_errors;
    snapshot.peer_book_known_peers = ctx.peer_book.len();
    snapshot.peer_book_discovered_peers = ctx.peer_book.discovered_count();
}
