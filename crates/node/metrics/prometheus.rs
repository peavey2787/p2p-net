//! Snapshot-to-metrics rendering for operator observability.

use crate::node::snapshot::NodeSnapshot;

/// Export operator counters in Prometheus text exposition format without opening an HTTP port.
/// Embedders that want an HTTP endpoint can serve this string from their own trusted admin server.
pub(crate) fn snapshot_to_prometheus_metrics(snapshot: &NodeSnapshot) -> String {
    fn line(name: &str, value: impl std::fmt::Display, out: &mut String) {
        out.push_str(name);
        out.push(' ');
        out.push_str(&value.to_string());
        out.push('\n');
    }

    let mut out = String::new();
    line("p2p_connected_peers", snapshot.connected_peers, &mut out);
    line("p2p_peer_book_known_peers", snapshot.peer_book_known_peers, &mut out);
    line("p2p_peer_book_discovered_peers", snapshot.peer_book_discovered_peers, &mut out);
    line("p2p_discovery_namespace_count", snapshot.discovery_namespace_count, &mut out);
    line(
        "p2p_dht_provider_enabled",
        if snapshot.dht_provider_enabled { 1 } else { 0 },
        &mut out,
    );
    line(
        "p2p_dht_provider_announce_attempts",
        snapshot.dht_provider_announce_attempts,
        &mut out,
    );
    line(
        "p2p_dht_provider_announce_failures",
        snapshot.dht_provider_announce_failures,
        &mut out,
    );
    line("p2p_dht_provider_queries", snapshot.dht_provider_queries, &mut out);
    line(
        "p2p_dht_provider_query_failures",
        snapshot.dht_provider_query_failures,
        &mut out,
    );
    line(
        "p2p_dht_provider_records_found",
        snapshot.dht_provider_records_found,
        &mut out,
    );
    line(
        "p2p_dht_provider_queries_finished",
        snapshot.dht_provider_queries_finished,
        &mut out,
    );
    line(
        "p2p_dht_provider_peers_discovered",
        snapshot.dht_provider_peers_discovered,
        &mut out,
    );
    line(
        "p2p_public_fallback_used",
        if snapshot.public_fallback_used { 1 } else { 0 },
        &mut out,
    );
    line(
        "p2p_public_bootstrap_seed_count",
        snapshot.public_bootstrap_seed_count,
        &mut out,
    );
    line(
        "p2p_public_relay_candidate_count",
        snapshot.public_relay_candidate_count,
        &mut out,
    );
    line("p2p_api_commands_processed", snapshot.api_commands_processed, &mut out);
    line("p2p_api_command_failures", snapshot.api_command_failures, &mut out);
    line("p2p_app_subscriptions", snapshot.app_subscriptions.len(), &mut out);
    line("p2p_app_messages_sent", snapshot.app_messages_sent, &mut out);
    line("p2p_app_messages_received", snapshot.app_messages_received, &mut out);
    line("p2p_app_messages_ignored", snapshot.app_messages_ignored, &mut out);
    line("p2p_app_messages_rejected", snapshot.app_messages_rejected, &mut out);
    line(
        "p2p_platform_can_listen_tcp",
        if snapshot.platform_can_listen_tcp { 1 } else { 0 },
        &mut out,
    );
    line(
        "p2p_platform_can_listen_quic",
        if snapshot.platform_can_listen_quic { 1 } else { 0 },
        &mut out,
    );
    line(
        "p2p_relay_server_enabled",
        if snapshot.relay_server_enabled { 1 } else { 0 },
        &mut out,
    );
    line(
        "p2p_mediator_enabled",
        if snapshot.mediator_enabled { 1 } else { 0 },
        &mut out,
    );
    line(
        "p2p_mediator_active_reservations",
        snapshot.mediator_active_reservations,
        &mut out,
    );
    line(
        "p2p_mediator_active_circuits",
        snapshot.mediator_active_circuits,
        &mut out,
    );
    line(
        "p2p_mediator_dcutr_attempts_observed",
        snapshot.mediator_dcutr_attempts_observed,
        &mut out,
    );
    line(
        "p2p_mediator_denied_reservations",
        snapshot.mediator_denied_reservations,
        &mut out,
    );
    line(
        "p2p_mediator_denied_circuits",
        snapshot.mediator_denied_circuits,
        &mut out,
    );
    line(
        "p2p_mediator_abuse_rate_limit_events",
        snapshot.mediator_abuse_rate_limit_events,
        &mut out,
    );
    line(
        "p2p_relay_reservations_accepted",
        snapshot.relay_reservations_accepted,
        &mut out,
    );
    line(
        "p2p_relay_client_reservations",
        snapshot.relay_client_reservations,
        &mut out,
    );
    line(
        "p2p_relay_active_circuits",
        snapshot.relay_active_circuits,
        &mut out,
    );
    line(
        "p2p_relay_denied_requests",
        snapshot.relay_denied_requests,
        &mut out,
    );
    line(
        "p2p_relay_bytes_forwarded",
        snapshot.relay_bytes_forwarded,
        &mut out,
    );
    line(
        "p2p_relay_discovery_enabled",
        if snapshot.relay_discovery_enabled { 1 } else { 0 },
        &mut out,
    );
    line(
        "p2p_relay_discovery_selected_relays",
        snapshot.relay_discovery_selected_relays.len(),
        &mut out,
    );
    line(
        "p2p_relay_discovery_candidate_count",
        snapshot.relay_discovery_candidate_count,
        &mut out,
    );
    line(
        "p2p_relay_discovery_public_candidates",
        snapshot.relay_discovery_public_candidates,
        &mut out,
    );
    line(
        "p2p_relay_discovery_failures",
        snapshot.relay_discovery_failures,
        &mut out,
    );
    line(
        "p2p_relay_discovery_replacements",
        snapshot.relay_discovery_replacements,
        &mut out,
    );
    line(
        "p2p_dcutr_enabled",
        if snapshot.dcutr_enabled { 1 } else { 0 },
        &mut out,
    );
    line("p2p_dcutr_attempts", snapshot.dcutr_attempts, &mut out);
    line("p2p_dcutr_successes", snapshot.dcutr_successes, &mut out);
    line("p2p_dcutr_failures", snapshot.dcutr_failures, &mut out);
    line(
        "p2p_dcutr_relay_fallbacks",
        snapshot.dcutr_relay_fallbacks,
        &mut out,
    );
    line(
        "p2p_dcutr_upgrade_eligible_connections",
        snapshot.dcutr_upgrade_eligible_connections,
        &mut out,
    );
    line(
        "p2p_dcutr_retry_suppressed",
        snapshot.dcutr_retry_suppressed,
        &mut out,
    );
    line(
        "p2p_gossip_messages_accepted",
        snapshot.gossip_messages_accepted,
        &mut out,
    );
    line(
        "p2p_gossip_messages_ignored",
        snapshot.gossip_messages_ignored,
        &mut out,
    );
    line(
        "p2p_gossip_messages_rejected",
        snapshot.gossip_messages_rejected,
        &mut out,
    );
    out
}
