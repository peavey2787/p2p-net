use std::fs;

use p2p_net::connectivity::dht::{dht_record_key, DhtDiscoveryConfig};
use p2p_net::DiscoveryConfig;

#[test]
fn dht_discovery_defaults_to_run_alongside_rendezvous() {
    let cfg = DhtDiscoveryConfig::default();

    assert!(cfg.enabled);
    assert!(cfg.announce);
    assert!(cfg.discover);
    assert!(cfg.discover_with_rendezvous_peers);
    assert_eq!(cfg.refresh_interval_secs, 300);
    assert!(cfg.should_discover(0));
    assert!(cfg.should_discover(1));
}

#[test]
fn dht_discovery_can_run_alongside_rendezvous_when_explicit() {
    let cfg = DhtDiscoveryConfig {
        discover_with_rendezvous_peers: true,
        ..DhtDiscoveryConfig::default()
    };

    assert!(cfg.should_discover(1));
}

#[test]
fn dht_discovery_validation_rejects_inert_or_unbounded_config() {
    let inert = DhtDiscoveryConfig {
        enabled: true,
        announce: false,
        discover: false,
        ..DhtDiscoveryConfig::default()
    };
    assert!(inert.validate().is_err());

    let unbounded = DhtDiscoveryConfig {
        max_namespaces_per_refresh: 0,
        ..DhtDiscoveryConfig::default()
    };
    assert!(unbounded.validate().is_err());

    let hot_loop = DhtDiscoveryConfig {
        refresh_interval_secs: 0,
        ..DhtDiscoveryConfig::default()
    };
    assert!(hot_loop.validate().is_err());
}

#[test]
fn discovery_config_exposes_dht_provider_policy() {
    let mut cfg = DiscoveryConfig::default();
    cfg.dht.enabled = true;
    cfg.dht.announce = true;
    cfg.dht.discover = true;

    assert!(cfg.validate().is_ok());
}

#[test]
fn dht_record_keys_are_derived_from_namespaces() {
    let first = dht_record_key("p2p-net/1/hydra-msg/abc");
    let same = dht_record_key("p2p-net/1/hydra-msg/abc");
    let different = dht_record_key("p2p-net/1/hydra-msg/def");

    assert_eq!(first, same);
    assert_ne!(first, different);
}

#[test]
fn dht_provider_discovery_is_wired_through_startup_and_events() {
    let dht_rs = fs::read_to_string("crates/connectivity/dht.rs").expect("read dht module");
    assert!(dht_rs.contains("start_providing"));
    assert!(dht_rs.contains("get_providers"));
    assert!(dht_rs.contains("QueryResult::GetProviders"));
    assert!(dht_rs.contains("DhtProviderState"));

    let node_rs = fs::read_to_string("crates/node/mod.rs").expect("read node module");
    assert!(node_rs.contains("start_dht_namespace_discovery"));
    assert!(node_rs.contains("DhtProviderState::default"));
    assert!(node_rs.contains("dht_provider_peers_discovered"));

    let events_rs = fs::read_to_string("crates/node/events.rs").expect("read event dispatcher");
    assert!(events_rs.contains("MeshEvent::Kademlia"));
    assert!(events_rs.contains("dht_state"));
}

#[test]
fn dht_provider_auto_connect_is_policy_gated_and_deduped() {
    let dht_rs = fs::read_to_string("crates/connectivity/dht.rs").expect("read dht module");
    assert!(dht_rs.contains("auto_connect_attempted_peers"));
    assert!(dht_rs.contains("auto_connect_waiting_for_addrs"));
    assert!(dht_rs.contains("should_auto_connect_provider_result"));
    assert!(dht_rs.contains("mark_auto_connect_failed"));

    let kademlia_rs =
        fs::read_to_string("crates/node/events/kademlia.rs").expect("read Kademlia event module");
    assert!(kademlia_rs.contains("auto_dial_dht_provider"));
    assert!(kademlia_rs.contains("auto_connect_discovered_peers"));
    assert!(kademlia_rs.contains("record_dht_provider_peers"));
    assert!(kademlia_rs.contains("record_kademlia_provider_addrs"));
    assert!(kademlia_rs.contains("mark_auto_connect_waiting_for_addrs"));
    assert!(kademlia_rs.contains("mark_auto_connect_attempted"));

    let strategy_rs = fs::read_to_string("crates/connectivity/connection_strategy.rs")
        .expect("read connection strategy");
    assert!(strategy_rs.contains("build_peer_book_connection_plan"));
    assert!(strategy_rs.contains("is_pending"));

    let dial_rs = fs::read_to_string("crates/node/dial.rs").expect("read node dial module");
    assert!(dial_rs.contains("source=kademlia address_resolution=behaviour"));
    assert!(dial_rs.contains("swarm.dial(peer)"));

    let connection_rs =
        fs::read_to_string("crates/node/events/connection.rs").expect("read connection events");
    assert!(connection_rs.contains("dht provider auto-connect retry scheduled"));
}
