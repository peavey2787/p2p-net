//! Runtime select-loop driver and LAN event handling.

use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::StreamExt;
use libp2p::swarm::SwarmEvent;
use libp2p::{PeerId, Swarm};
use tokio::time::MissedTickBehavior;

use crate::api::PeerSource;
use crate::connectivity::dht::publish_local_peer_address_records;
use crate::connectivity::lan::{LanDiscoveryReceive, LanDiscoverySocket, LanPeerAnnouncement};
use crate::stack::{add_peer_address_to_discovery, allow_dcutr_peer, MeshBehaviour};

use super::super::commands::{self, NodeCommandContext};
use super::super::config::NodeConfig;
use super::super::dial::auto_dial_peer_from_book;
use super::super::events::{self, SwarmEventContext};
use super::super::public_ip;
use super::super::runtime_tasks::apply_public_ip_probe_result;
use super::NodeRuntimeContext;
use super::RuntimeState;
use super::{observability, periodic};

const OBSERVABILITY_FLUSH_INTERVAL: Duration = Duration::from_secs(1);

pub(super) async fn run_node_runtime(ctx: NodeRuntimeContext) {
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
    } = ctx;

    let heartbeat_interval = Duration::from_secs(cfg.heartbeat_interval_secs.max(1));
    let mut ticker = tokio::time::interval(heartbeat_interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut observability_ticker = tokio::time::interval(OBSERVABILITY_FLUSH_INTERVAL);
    observability_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
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
    let lan_socket = if cfg.discovery.lan.enabled {
        match LanDiscoverySocket::bind(&cfg.discovery.lan) {
            Ok(socket) => Some(socket),
            Err(err) => {
                runtime_state.observability.pulse(format!(
                    "lan discovery unavailable port={} error={err}",
                    cfg.discovery.lan.port
                ));
                None
            }
        }
    } else {
        None
    };
    let mut lan_ticker = tokio::time::interval(Duration::from_secs(
        cfg.discovery.lan.announce_interval_secs.max(1),
    ));
    lan_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let started_at = std::time::Instant::now();
    let enabled_listen_addresses = cfg
        .enabled_listen_addresses()
        .unwrap_or_default()
        .into_iter()
        .map(|addr| addr.to_string())
        .collect();
    let mut public_ip_probe = Box::pin(public_ip::probe_public_addresses(
        cfg.public_ip_probe.clone(),
        enabled_listen_addresses,
    ));
    let mut public_ip_probe_done = false;
    let mut dht_refresh_sleep = Box::pin(tokio::time::sleep_until(
        runtime_state.dht_refresh_schedule.next_due(),
    ));

    loop {
        tokio::select! {
            biased;
            _ = ticker.tick() => {
                periodic::tick_runtime(
                    &cfg,
                    &mut swarm,
                    local_peer,
                    &heartbeat_topic,
                    &snapshot,
                    &mut runtime_state,
                    started_at,
                ).await;
                snapshot_revision.fetch_add(1, Ordering::Relaxed);
            }
            _ = observability_ticker.tick() => {
                if observability::flush_observability(
                    &cfg,
                    &snapshot,
                    storage.as_ref(),
                    &mut runtime_state,
                )
                .await
                {
                    snapshot_revision.fetch_add(1, Ordering::Relaxed);
                }
            }
            _ = &mut dht_refresh_sleep => {
                periodic::refresh_dht(
                    &cfg,
                    &mut swarm,
                    &discovery_signing_key,
                    &snapshot,
                    &mut runtime_state,
                    "scheduled",
                ).await;
                snapshot_revision.fetch_add(1, Ordering::Relaxed);
                dht_refresh_sleep
                    .as_mut()
                    .reset(runtime_state.dht_refresh_schedule.next_due());
            }
            _ = lan_ticker.tick(), if lan_socket.is_some() => {
                let addresses = swarm.listeners().cloned().collect::<Vec<_>>();
                if let Some(socket) = lan_socket.as_ref() {
                    if let Err(err) = socket
                        .announce(
                            cfg.network_id,
                            &application_protocol_version,
                            local_peer,
                            addresses,
                        )
                        .await
                    {
                        runtime_state.observability.pulse(format!(
                            "lan discovery announce error={err}"
                        ));
                    }
                }
            }
            lan_result = recv_lan_announcement(
                lan_socket.as_ref(),
                local_peer,
                cfg.network_id,
                &application_protocol_version,
            ), if lan_socket.is_some() => {
                match lan_result {
                    Ok(Some(received)) => {
                        if let Some(reply_to) = received.reply_to {
                            let addresses = swarm.listeners().cloned().collect::<Vec<_>>();
                            if let Some(socket) = lan_socket.as_ref() {
                                if let Err(err) = socket
                                    .respond(
                                        reply_to,
                                        cfg.network_id,
                                        &application_protocol_version,
                                        local_peer,
                                        addresses,
                                    )
                                    .await
                                {
                                    runtime_state.observability.pulse(format!(
                                        "lan discovery unicast reply error target={reply_to} error={err}"
                                    ));
                                }
                            }
                        }
                        if let Some(announcement) = received.announcement {
                            handle_lan_announcement(
                                announcement,
                                &cfg,
                                &application_namespaces,
                                &mut swarm,
                                local_peer,
                                &mut runtime_state,
                            );
                        }
                    }
                    Ok(None) => {}
                    Err(err) => runtime_state.observability.pulse(format!(
                        "lan discovery receive error={err}"
                    )),
                }
            }
            public_ip_result = &mut public_ip_probe, if !public_ip_probe_done => {
                public_ip_probe_done = true;
                let refreshed_dht = !public_ip_result.external_addresses.is_empty();
                apply_public_ip_probe_result(
                    public_ip_result,
                    &cfg,
                    &mut swarm,
                    &snapshot,
                    &mut runtime_state.dht_state,
                    rendezvous_peers.len(),
                ).await;
                snapshot_revision.fetch_add(1, Ordering::Relaxed);
                if refreshed_dht {
                    let publish = publish_local_peer_address_records(
                        &mut swarm,
                        &discovery_signing_key,
                        cfg.network_id,
                        &cfg.discovery,
                        &mut runtime_state.dht_state,
                    );
                    for err in publish.errors {
                        runtime_state.observability.pulse(format!(
                            "dht signed peer address publish error: {err}"
                        ));
                    }
                    runtime_state.dht_refresh_schedule.record_refresh();
                    dht_refresh_sleep
                        .as_mut()
                        .reset(runtime_state.dht_refresh_schedule.next_due());
                }
            }
            maybe_shutdown = shutdown_rx.recv() => {
                let _ = maybe_shutdown;
                break;
            }
            maybe_command = command_rx.recv() => {
                if let Some(command) = maybe_command {
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
                    )
                    .await;
                    snapshot_revision.fetch_add(1, Ordering::Relaxed);
                } else {
                    break;
                }
            }
            evt = swarm.select_next_some() => {
                let connectivity_recovered = matches!(
                    &evt,
                    SwarmEvent::ConnectionEstablished { num_established, .. }
                        if num_established.get() == 1
                            && swarm.connected_peers().take(2).count() == 1
                );
                let snapshot_update_deferred = events::snapshot_update_deferred(&evt);
                {
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
                        heartbeat_topic_hash: &heartbeat_topic_hash,
                        app_topic_hashes: &runtime_state.app_topic_hashes,
                        app_messages: &messages_tx,
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
                }
                if !snapshot_update_deferred {
                    snapshot_revision.fetch_add(1, Ordering::Relaxed);
                }
                if connectivity_recovered
                    && runtime_state
                        .dht_refresh_schedule
                        .request_connectivity_recovery_refresh()
                {
                    dht_refresh_sleep
                        .as_mut()
                        .reset(runtime_state.dht_refresh_schedule.next_due());
                }
            }
        }
    }
    runtime_state.flush_peer_cache(&cfg, storage.as_ref());
    observability::flush_observability(&cfg, &snapshot, storage.as_ref(), &mut runtime_state).await;
}

async fn recv_lan_announcement(
    socket: Option<&LanDiscoverySocket>,
    local_peer: PeerId,
    network_id: u32,
    application_protocol: &str,
) -> std::io::Result<Option<LanDiscoveryReceive>> {
    match socket {
        Some(socket) => {
            socket
                .recv(local_peer, network_id, application_protocol)
                .await
        }
        None => std::future::pending().await,
    }
}

fn handle_lan_announcement(
    announcement: LanPeerAnnouncement,
    cfg: &NodeConfig,
    application_namespaces: &[String],
    swarm: &mut Swarm<MeshBehaviour>,
    local_peer: PeerId,
    runtime_state: &mut RuntimeState,
) {
    const MAX_LAN_DISCOVERED_PEERS: usize = 2048;
    let peer = announcement.peer_id;
    if peer == local_peer
        || (runtime_state.peer_book.record(&peer).is_none()
            && runtime_state.peer_book.len() >= MAX_LAN_DISCOVERED_PEERS)
    {
        return;
    }
    allow_dcutr_peer(swarm, peer);
    for namespace in application_namespaces {
        runtime_state
            .peer_book
            .record_namespace(peer, namespace.clone(), PeerSource::LanDiscovery);
    }
    for addr in announcement.addresses {
        add_peer_address_to_discovery(swarm, peer, addr.clone());
        runtime_state
            .peer_book
            .record_addr(peer, addr.clone(), PeerSource::LanDiscovery);
        runtime_state.peer_cache_writes.record_seen(peer, addr);
    }
    if runtime_state.auto_dial_stats.is_suppressed(&peer) {
        return;
    }
    let outcome = auto_dial_peer_from_book(
        peer,
        local_peer,
        cfg.discovery.public_bootstrap.auto_connect_discovered_peers,
        swarm,
        &runtime_state.peer_book,
        &mut runtime_state.pending_connections,
        &cfg.dcutr,
    );
    runtime_state
        .auto_dial_stats
        .record_outcome(&peer, &outcome);
    runtime_state.observability.peer_connectivity_dirty();
    if outcome.should_pulse() {
        runtime_state
            .observability
            .pulse(format!("lan discovery {}", outcome.describe(&peer)));
    }
}
