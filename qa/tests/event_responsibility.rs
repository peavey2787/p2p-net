use std::fs;
use std::path::Path;

#[test]
fn swarm_event_handlers_are_split_by_responsibility() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for relative in [
        "crates/p2p-net/src/node/events/connection.rs",
        "crates/p2p-net/src/node/events/relay_client.rs",
        "crates/p2p-net/src/node/events/relay_server.rs",
        "crates/p2p-net/src/node/events/dcutr.rs",
        "crates/p2p-net/src/node/events/rendezvous.rs",
        "crates/p2p-net/src/node/events/gossip.rs",
    ] {
        assert!(root.join(relative).exists(), "missing {relative}");
    }

    let dispatcher = fs::read_to_string(root.join("crates/p2p-net/src/node/events.rs"))
        .expect("dispatcher source is readable");
    for module in [
        "mod connection;",
        "mod relay_client;",
        "mod relay_server;",
        "mod dcutr;",
        "mod rendezvous;",
        "mod gossip;",
    ] {
        assert!(dispatcher.contains(module), "dispatcher missing {module}");
    }

    for moved_handler in [
        "fn process_relay_client_event",
        "fn process_relay_server_event",
        "fn process_dcutr_event",
        "fn process_inbound_heartbeat",
        "fn apply_denial_health",
    ] {
        assert!(
            !dispatcher.contains(moved_handler),
            "dispatcher still owns moved handler {moved_handler}"
        );
    }
}
