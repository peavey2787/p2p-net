//! Browser runtime driver. No UDP discovery, public-IP probing, native sockets,
//! or Tokio timer driver is required on wasm32.

use std::sync::atomic::Ordering;
use std::time::Duration;
use web_time::Instant;

use futures::{FutureExt, StreamExt};
use libp2p::swarm::SwarmEvent;

use crate::connectivity::dht::publish_local_peer_address_records;
use crate::runtime;

use super::super::super::commands::{self, NodeCommandContext};
use super::super::super::events::{self, SwarmEventContext};
use super::super::{observability, periodic, NodeRuntimeContext, RuntimeState};

const OBSERVABILITY_FLUSH_INTERVAL: Duration = Duration::from_secs(1);

fn until(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

pub(in crate::node::runtime) async fn run_node_runtime(ctx: NodeRuntimeContext) {
    let NodeRuntimeContext {
        cfg,
        resolved_config,
        mut swarm,
        local_peer,
        discovery_signing_key,
        heartbeat_topic,
        snapshot,
        snapshot_revision,
        storage,
        rendezvous_peers,
        relay_reservation_plan,
        relay_selection_plan,
        rendezvous_state,
        dht_state,
        peer_book,
        mut shutdown_rx,
        mut command_rx,
        messages_tx,
        events_tx,
    } = ctx;

    let heartbeat_interval = Duration::from_secs(cfg.heartbeat_interval_secs.max(1));
    let started_at = Instant::now();
    let mut heartbeat_due = Instant::now() + heartbeat_interval;
    let mut observability_due = Instant::now() + OBSERVABILITY_FLUSH_INTERVAL;
    let mut runtime_state = RuntimeState::new(
        &cfg,
        &resolved_config,
        relay_reservation_plan,
        relay_selection_plan,
        rendezvous_state,
        dht_state,
        peer_book,
    );
    let heartbeat_topic_hash = heartbeat_topic.hash().clone();
    let application_protocol_version = cfg
        .discovery
        .application_protocol_version(cfg.network_id)
        .expect("validated discovery namespace configuration");
    let application_namespaces = cfg
        .discovery
        .rendezvous_namespaces(cfg.network_id)
        .expect("validated discovery namespace configuration");

    loop {
        let heartbeat = runtime::sleep(until(heartbeat_due)).fuse();
        let observable = runtime::sleep(until(observability_due)).fuse();
        let dht = runtime::sleep(until(runtime_state.dht_refresh_schedule.next_due())).fuse();
        let shutdown = shutdown_rx.recv().fuse();
        let command = command_rx.recv().fuse();
        let swarm_event = swarm.select_next_some().fuse();
        futures::pin_mut!(heartbeat, observable, dht, shutdown, command, swarm_event);

        futures::select_biased! {
            _ = shutdown => break,
            maybe_command = command => {
                let Some(command) = maybe_command else { break; };
                commands::handle_node_command(
                    command,
                    NodeCommandContext {
                        swarm: &mut swarm,
                        local_peer,
                        network_id: cfg.network_id,
                        app_topic_hashes: &mut runtime_state.app_topic_hashes,
                        snapshot: &snapshot,
                        peer_book: &mut runtime_state.peer_book,
                        pending_connections: &mut runtime_state.pending_connections,
                        auto_dial_stats: &mut runtime_state.auto_dial_stats,
                        dcutr_policy: &cfg.dcutr,
                        metrics: &mut runtime_state.metrics,
                    },
                ).await;
                snapshot_revision.fetch_add(1, Ordering::Relaxed);
            },
            evt = swarm_event => {
                let connectivity_recovered = matches!(
                    &evt,
                    SwarmEvent::ConnectionEstablished { num_established, .. }
                        if num_established.get() == 1 && swarm.connected_peers().take(2).count() == 1
                );
                let snapshot_update_deferred = events::snapshot_update_deferred(&evt);
                let mut event_ctx = SwarmEventContext {
                    snapshot: &snapshot,
                    rep: &mut runtime_state.rep,
                    relay_state: &mut runtime_state.relay_state,
                    rendezvous_state: &mut runtime_state.rendezvous_state,
                    dht_state: &mut runtime_state.dht_state,
                    peer_book: &mut runtime_state.peer_book,
                    pending_connections: &mut runtime_state.pending_connections,
                    auto_dial_stats: &mut runtime_state.auto_dial_stats,
                    connection_caps: &mut runtime_state.connection_caps,
                    relay_cfg: &cfg.relay,
                    dcutr_policy: &cfg.dcutr,
                    discovery_cfg: &cfg.discovery,
                    peer_cache_writes: &mut runtime_state.peer_cache_writes,
                    rendezvous_peers: &rendezvous_peers,
                    message_security: &cfg.message_security,
                    replay_cache: &mut runtime_state.replay_cache,
                    app_replay_cache: &mut runtime_state.app_replay_cache,
                    app_fragment_reassembler: &mut runtime_state.app_fragment_reassembler,
                    heartbeat_topic_hash: &heartbeat_topic_hash,
                    app_topic_hashes: &runtime_state.app_topic_hashes,
                    app_messages: &messages_tx,
                    node_events: &events_tx,
                    metrics: &mut runtime_state.metrics,
                    identify_addresses: &mut runtime_state.identify_addresses,
                    observability: &mut runtime_state.observability,
                    local_peer,
                    local_key: &discovery_signing_key,
                    network_id: cfg.network_id,
                    application_protocol_version: &application_protocol_version,
                    application_namespaces: &application_namespaces,
                };
                events::handle_swarm_event(evt, &mut swarm, &mut event_ctx).await;
                if !snapshot_update_deferred {
                    snapshot_revision.fetch_add(1, Ordering::Relaxed);
                }
                if connectivity_recovered {
                    let _ = runtime_state.dht_refresh_schedule.request_connectivity_recovery_refresh();
                }
            },
            _ = heartbeat => {
                periodic::tick_runtime(
                    &cfg, &mut swarm, local_peer, &heartbeat_topic, &snapshot,
                    &mut runtime_state, started_at,
                ).await;
                heartbeat_due = Instant::now() + heartbeat_interval;
                snapshot_revision.fetch_add(1, Ordering::Relaxed);
            },
            _ = dht => {
                periodic::refresh_dht(
                    &cfg, &mut swarm, &discovery_signing_key, &snapshot,
                    &mut runtime_state, "scheduled",
                ).await;
                snapshot_revision.fetch_add(1, Ordering::Relaxed);
            },
            _ = observable => {
                if observability::flush_observability(&cfg, &snapshot, storage.as_ref(), &mut runtime_state).await {
                    snapshot_revision.fetch_add(1, Ordering::Relaxed);
                }
                observability_due = Instant::now() + OBSERVABILITY_FLUSH_INTERVAL;
            },
        }
    }

    // Browser persistence uses the same synchronous journal as native core;
    // the WASM facade durably flushes the journal to IndexedDB on shutdown/pagehide.
    runtime_state.flush_peer_cache(&cfg, storage.as_ref());
    observability::flush_observability(&cfg, &snapshot, storage.as_ref(), &mut runtime_state).await;

    // Keep DHT address publication semantics common; browser role simply has
    // no raw local listener addresses to publish.
    let publish = publish_local_peer_address_records(
        &mut swarm,
        &discovery_signing_key,
        cfg.network_id,
        &cfg.discovery,
        &mut runtime_state.dht_state,
    );
    for err in publish.errors {
        runtime_state
            .observability
            .pulse(format!("shutdown dht publish error: {err}"));
    }
}
