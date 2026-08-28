use std::fs;

#[test]
fn dashboard_is_event_driven_and_avoids_periodic_full_snapshot_hashing() {
    let source = fs::read_to_string("apps/windows/p2p_node.rs").expect("read dashboard example");
    let handle = fs::read_to_string("crates/node/handle.rs").expect("read node handle");

    assert!(source.contains("EventStream::new()"));
    assert!(source.contains("render_if_changed"));
    assert!(source.contains("handle.snapshot_revision()"));
    assert!(source.contains("MissedTickBehavior::Skip"));
    assert!(handle.contains("pub fn snapshot_revision(&self) -> u64"));
    assert!(!source.contains("snapshot_signature"));
    assert!(!source.contains("DefaultHasher"));
    assert!(!source.contains("event::poll("));
}

#[test]
fn dashboard_panic_path_restores_style_and_sanitizes_dynamic_text() {
    let source = fs::read_to_string("apps/windows/p2p_node.rs").expect("read dashboard example");

    assert!(source.contains("SetAttribute(Attribute::Reset)"));
    assert!(source.contains("ResetColor"));
    assert!(source.contains("view::sanitize_terminal_text(&format!(\"{info}\"))"));
    assert!(!source.contains("default_hook(info)"));
}

#[test]
fn dashboard_default_is_full_capability_without_example_specific_throttles() {
    let source = fs::read_to_string("apps/windows/p2p_node.rs").expect("read dashboard example");

    assert!(source.contains("profile: NodeProfile::Full"));
    assert!(source.contains("#[tokio::main]"));
    assert!(!source.contains("worker_threads = 1"));
    assert!(!source.contains("--low-cpu"));
    assert!(!source.contains("NodeProfile::Lite"));
    assert!(!source.contains("max_established, 12"));
    assert!(!source.contains("gossipsub_heartbeat_interval_secs.max(10)"));
    assert!(!source.contains("ping_interval_secs.max(60)"));
}

#[test]
fn dashboard_handles_console_close_and_always_runs_node_shutdown() {
    let dashboard = fs::read_to_string("apps/windows/p2p_node.rs").expect("read dashboard example");
    let handle = fs::read_to_string("crates/node/handle.rs").expect("read node handle");

    assert!(dashboard.contains("ctrl_close"));
    assert!(dashboard.contains("ctrl_logoff"));
    assert!(dashboard.contains("ctrl_shutdown"));
    assert!(dashboard.contains("SignalKind::terminate()"));
    assert!(dashboard.contains("SignalKind::hangup()"));
    assert!(dashboard.contains("handle.shutdown().await;"));
    assert!(handle.contains("shutdown_tx.try_send(())"));
    assert!(handle.contains("NODE_SHUTDOWN_GRACE: Duration = Duration::from_secs(1)"));
}

#[test]
fn full_node_protocol_cadences_and_dht_controls_remain_wired_to_libp2p() {
    let config = fs::read_to_string("crates/node/config.rs").expect("read node config");
    let dht = fs::read_to_string("crates/connectivity/dht.rs").expect("read DHT config");
    let behaviour = fs::read_to_string("crates/stack/behaviour.rs").expect("read behaviour");

    assert!(config.contains("gossipsub_heartbeat_interval_secs: 5"));
    assert!(config.contains("ping_interval_secs: 15"));
    assert!(dht.contains("periodic_bootstrap_interval_secs: Some(300)"));
    assert!(dht.contains("query_parallelism: 3"));
    assert!(dht.contains("provider_key_replicas: 3"));
    assert!(behaviour.contains("Duration::from_secs(gossipsub_heartbeat_interval_secs)"));
    assert!(behaviour.contains("with_interval(Duration::from_secs(ping_interval_secs))"));
    assert!(behaviour.contains("set_periodic_bootstrap_interval"));
    assert!(behaviour.contains("set_parallelism"));
}

#[test]
fn full_node_hot_paths_are_optimized_without_reducing_capability() {
    let runtime = fs::read_to_string("crates/node/runtime.rs").expect("read runtime");
    let runtime_driver =
        fs::read_to_string("crates/node/runtime/driver.rs").expect("read runtime driver");
    let dht_schedule = fs::read_to_string("crates/node/runtime/dht_schedule.rs")
        .expect("read DHT refresh schedule");
    let cache = fs::read_to_string("crates/connectivity/peer_cache/store.rs").expect("read cache");
    let kademlia = fs::read_to_string("crates/node/events/kademlia.rs").expect("read kad events");

    let events = fs::read_to_string("crates/node/events.rs").expect("read event batching");
    let identify = fs::read_to_string("crates/node/events/connection/identify.rs")
        .expect("read identify event handling");

    assert!(dht_schedule.contains("request_connectivity_recovery_refresh"));
    assert!(runtime_driver.contains("swarm.connected_peers().take(2).count() == 1"));
    assert!(runtime.contains("PEER_CACHE_FLUSH_INTERVAL"));
    assert!(runtime.contains("from_secs(5)"));
    assert!(runtime.contains("peer_cache_writes"));
    assert!(cache.contains("PeerCacheWriteBatch"));
    assert!(cache.contains("pending_seen: HashSet"));
    assert!(cache.contains("apply_mutations_with_storage"));
    assert!(kademlia.contains("libp2p::kad::Event::InboundRequest { .. }"));
    assert!(kademlia.contains("APPLICATION_DIAL_REQUIRED_HEADROOM: u32 = 1"));
    assert!(kademlia.contains("ctx.observability.peer_connectivity_dirty()"));
    assert!(!kademlia.contains("ctx.snapshot.lock().await"));
    assert!(events.contains("pub(crate) fn dht_dirty(&mut self)"));
    assert!(identify.contains("record_observed_local_addr(observed_addr)"));
}
