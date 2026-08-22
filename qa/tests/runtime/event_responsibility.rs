use std::fs;
use std::path::Path;

#[test]
fn swarm_event_handlers_are_split_by_responsibility() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for relative in [
        "crates/node/events/connection.rs",
        "crates/node/events/connection/errors.rs",
        "crates/node/events/relay_client.rs",
        "crates/node/events/relay_server.rs",
        "crates/node/events/dcutr.rs",
        "crates/node/events/rendezvous.rs",
        "crates/node/events/kademlia.rs",
        "crates/node/events/gossip.rs",
        "crates/node/events/app.rs",
    ] {
        assert!(root.join(relative).exists(), "missing {relative}");
    }

    let dispatcher = fs::read_to_string(root.join("crates/node/events.rs"))
        .expect("dispatcher source is readable");
    for module in [
        "mod connection;",
        "mod relay_client;",
        "mod relay_server;",
        "mod dcutr;",
        "mod rendezvous;",
        "mod kademlia;",
        "mod gossip;",
        "mod app;",
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

#[test]
fn signed_gossipsub_identity_and_manual_validation_are_enforced() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dispatcher = fs::read_to_string(root.join("crates/node/events.rs"))
        .expect("dispatcher source is readable");
    let heartbeat = fs::read_to_string(root.join("crates/node/events/gossip.rs"))
        .expect("heartbeat handler source is readable");
    let app = fs::read_to_string(root.join("crates/node/events/app.rs"))
        .expect("app handler source is readable");
    let behaviour = fs::read_to_string(root.join("crates/stack/behaviour.rs"))
        .expect("behaviour source is readable");

    assert!(
        dispatcher.contains("message.source") && dispatcher.contains("propagation_source"),
        "dispatcher must preserve signed author separately from the immediate forwarder"
    );
    assert!(
        heartbeat.contains("validate_heartbeat_wire(")
            && heartbeat.contains("author,")
            && heartbeat.contains("&propagation_source"),
        "heartbeat validation must authenticate the signed author while reporting against the forwarder"
    );
    assert!(
        app.contains("validate_app_message_authentication")
            && app.contains("validate_app_message_security")
            && app.contains("MessageAcceptance::Accept")
            && app.contains("MessageAcceptance::Ignore")
            && app.contains("MessageAcceptance::Reject")
            && app.contains("&propagation_source"),
        "application gossip must bind author/topic, enforce replay protection, and report every manual validation decision"
    );
    assert!(
        behaviour.contains("msg.source")
            && behaviour.contains("msg.topic.as_str()")
            && behaviour.contains("p2p-net/gossipsub-message-id/v2"),
        "gossipsub message IDs must bind signed author and outer topic as well as payload bytes"
    );
}
