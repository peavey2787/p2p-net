use libp2p::Swarm;

use crate::api::PeerSource;
use crate::connectivity::rendezvous::peer_record_addrs;
use libp2p_rendezvous as rendezvous;

use crate::stack::{on_rendezvous_client_event, on_rendezvous_server_event, MeshBehaviour};

use super::super::push_pulse;
use super::SwarmEventContext;

pub(crate) async fn handle_client_event(
    swarm: &mut Swarm<MeshBehaviour>,
    ev: &rendezvous::client::Event,
    ctx: &mut SwarmEventContext<'_>,
) {
    record_rendezvous_discovery(ev, ctx);
    let line = on_rendezvous_client_event(
        swarm,
        ev,
        ctx.discovery_cfg,
        ctx.storage,
        ctx.rendezvous_state,
    );
    let mut guard = ctx.snapshot.lock().await;
    sync_rendezvous_snapshot(&mut guard, ctx);
    push_pulse(&mut guard.pulses, line);
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
) {
    let rendezvous::client::Event::Discovered { registrations, .. } = ev else {
        return;
    };

    for registration in registrations {
        let peer = registration.record.peer_id();
        let namespace = registration.namespace.to_string();
        ctx.peer_book
            .record_namespace(peer, namespace, PeerSource::Rendezvous);
        for addr in peer_record_addrs(registration) {
            ctx.peer_book.record_addr(peer, addr, PeerSource::Rendezvous);
        }
    }
}

fn sync_rendezvous_snapshot(
    snapshot: &mut super::super::types::NodeSnapshot,
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
