use std::collections::VecDeque;

use p2p_net::NodeSnapshot;

#[path = "../../../examples/p2p_node/view/mod.rs"]
mod dashboard_view;

#[test]
fn dashboard_layout_is_bounded_across_terminal_sizes() {
    let mut snapshot = NodeSnapshot {
        network_label: "public-testnet".to_string(),
        peer_id: "12D3KooWExamplePeerIdForDashboardLayout".to_string(),
        active_transports: vec![
            "quic".to_string(),
            "tcp".to_string(),
            "websocket".to_string(),
            "webrtc-direct".to_string(),
        ],
        ..NodeSnapshot::default()
    };
    snapshot.pulses = VecDeque::from([
        "relay client inbound relayed circuit established src=12D3KooWExample".to_string(),
        "relayed connection pending application verification peer=12D3KooWExample".to_string(),
    ]);

    for columns in [0usize, 1, 6, 24, 40, 63, 64, 100, 160, 220] {
        for rows in [0usize, 1, 8, 16, 23, 24, 38, 60] {
            let lines = dashboard_view::dashboard_lines(&snapshot, columns, rows);
            assert!(lines.len() <= rows, "rows overflow at {columns}x{rows}");
            for line in lines {
                assert!(
                    line.visible_width() <= columns.min(160),
                    "column overflow at {columns}x{rows}: {}",
                    line.plain_text()
                );
                for span in line.spans() {
                    assert!(!span.text().contains('\x1b'));
                    let _ = span.tone();
                    let _ = span.bold();
                }
            }
        }
    }
}

#[test]
fn dashboard_neutralizes_terminal_escape_and_bidi_controls() {
    let mut snapshot = NodeSnapshot {
        network_label: "main\x1b[31mnet\u{202e}spoof".to_string(),
        peer_id: "peer\nsecond-line\u{2066}hidden".to_string(),
        public_addr: Some("/ip4/1.2.3.4/tcp/4001\r\x1b]0;owned\x07".to_string()),
        ..NodeSnapshot::default()
    };
    snapshot.pulses =
        VecDeque::from(["incoming error peer=evil\x1b[2J\x1b[H\u{200b}masked".to_string()]);

    assert_eq!(
        dashboard_view::sanitize_terminal_text("raw\x1b[31m\u{202e}text"),
        "raw?[31m?text"
    );

    let text = dashboard_view::dashboard_lines(&snapshot, 120, 40)
        .into_iter()
        .map(|line| line.plain_text())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(!text.contains('\x1b'));
    assert!(!text.contains('\r'));
    assert!(!text.contains('\u{202e}'));
    assert!(!text.contains('\u{2066}'));
    assert!(!text.contains('\u{200b}'));
    assert!(text.contains("main?[31mnet?spoof"));
    assert!(text.contains("peer?second-line?hidden"));
    assert!(text.contains("evil?[2J?[H?masked"));
}

#[test]
fn dashboard_wraps_long_event_lines_inside_the_event_panel() {
    let mut snapshot = NodeSnapshot::default();
    snapshot.pulses.push_front(format!(
        "incoming connection error peer={} error={}",
        "p".repeat(140),
        "x".repeat(140)
    ));

    let lines = dashboard_view::dashboard_lines(&snapshot, 80, 32);
    let event_rows = lines
        .iter()
        .map(|line| line.plain_text())
        .filter(|line| line.contains("incoming connection") || line.contains("pppppppppp"))
        .collect::<Vec<_>>();

    assert!(event_rows.len() >= 2, "long pulse should wrap cleanly");
    assert!(lines.iter().all(|line| line.visible_width() <= 80));
}

#[test]
fn dashboard_keeps_distinct_peer_and_discovery_counters_visible() {
    let snapshot = NodeSnapshot {
        application_peer_connections: 2,
        infrastructure_peer_connections: 3,
        dht_routing_peer_connections: 4,
        relay_peer_connections: 1,
        all_swarm_connections: 6,
        peer_book_known_peers: 9,
        peer_book_discovered_peers: 7,
        connection_plan_pending_peers: 2,
        auto_connect_awaiting_address_peers: 5,
        ..NodeSnapshot::default()
    };

    let text = dashboard_view::dashboard_lines(&snapshot, 120, 40)
        .into_iter()
        .map(|line| line.plain_text())
        .collect::<Vec<_>>()
        .join("\n");

    for expected in [
        "APP 2",
        "INFRA 3",
        "DHT 4",
        "RELAY 1",
        "SWARM 6",
        "KNOWN 9",
        "DISCOVERED 7",
        "PENDING 2",
        "AWAITING ADDRS 5",
    ] {
        assert!(
            text.contains(expected),
            "missing dashboard counter: {expected}"
        );
    }
}
