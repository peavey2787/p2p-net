use libp2p::{PeerId, Swarm};

use crate::stack::MeshBehaviour;

use super::super::super::push_pulse;
use super::super::{sync_swarm_connection_snapshot, SwarmEventContext};
use super::should_track_peer_in_peer_book;

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
    let mut application_dial_error = None;
    if let Some(peer) = peer_id.as_ref() {
        if should_track_peer_in_peer_book(*peer, ctx) {
            application_dial_error = Some(format!("peer={peer} error={error_debug}"));
        }
        if ctx.peer_book.record(peer).is_some() {
            ctx.peer_cache_writes.record_failure(*peer);
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
    if application_dial_error.is_some() {
        guard.last_application_dial_error = application_dial_error;
    }
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
