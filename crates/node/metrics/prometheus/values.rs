//! Metric value extraction for Prometheus rendering.

use crate::node::snapshot::NodeSnapshot;

use super::CONNECTED_PEERS_METRIC;

pub(super) fn snapshot_metric_values(s: &NodeSnapshot) -> Vec<(&'static str, String)> {
    vec![
        metric(CONNECTED_PEERS_METRIC, s.connected_peers),
        metric("p2p_peer_book_known_peers", s.peer_book_known_peers),
        metric(
            "p2p_peer_book_discovered_peers",
            s.peer_book_discovered_peers,
        ),
        metric("p2p_auto_connect_enabled", flag(s.auto_connect_enabled)),
        metric(
            "p2p_auto_connect_dial_attempts",
            s.auto_connect_dial_attempts,
        ),
        metric(
            "p2p_auto_connect_dial_failures",
            s.auto_connect_dial_failures,
        ),
        metric(
            "p2p_auto_connect_awaiting_address_peers",
            s.auto_connect_awaiting_address_peers,
        ),
        metric(
            "p2p_connection_plan_pending_peers",
            s.connection_plan_pending_peers,
        ),
        metric("p2p_discovery_namespace_count", s.discovery_namespace_count),
        metric("p2p_dht_provider_enabled", flag(s.dht_provider_enabled)),
        metric(
            "p2p_dht_provider_announce_attempts",
            s.dht_provider_announce_attempts,
        ),
        metric(
            "p2p_dht_provider_announce_failures",
            s.dht_provider_announce_failures,
        ),
        metric("p2p_dht_provider_queries", s.dht_provider_queries),
        metric(
            "p2p_dht_provider_query_failures",
            s.dht_provider_query_failures,
        ),
        metric(
            "p2p_dht_provider_records_found",
            s.dht_provider_records_found,
        ),
        metric(
            "p2p_dht_provider_queries_finished",
            s.dht_provider_queries_finished,
        ),
        metric(
            "p2p_dht_provider_peers_discovered",
            s.dht_provider_peers_discovered,
        ),
        metric("p2p_public_fallback_used", flag(s.public_fallback_used)),
        metric("p2p_public_bootstrap_used", flag(s.public_bootstrap_used)),
        metric("p2p_public_rendezvous_used", flag(s.public_rendezvous_used)),
        metric("p2p_public_relay_used", flag(s.public_relay_used)),
        metric(
            "p2p_public_bootstrap_seed_count",
            s.public_bootstrap_seed_count,
        ),
        metric(
            "p2p_public_rendezvous_candidate_count",
            s.public_rendezvous_candidate_count,
        ),
        metric(
            "p2p_public_relay_candidate_count",
            s.public_relay_candidate_count,
        ),
        metric("p2p_api_commands_processed", s.api_commands_processed),
        metric("p2p_api_command_failures", s.api_command_failures),
        metric("p2p_app_subscriptions", s.app_subscriptions.len()),
        metric("p2p_app_messages_sent", s.app_messages_sent),
        metric("p2p_app_messages_received", s.app_messages_received),
        metric("p2p_app_messages_ignored", s.app_messages_ignored),
        metric("p2p_app_messages_rejected", s.app_messages_rejected),
        metric(
            "p2p_platform_can_listen_tcp",
            flag(s.platform_can_listen_tcp),
        ),
        metric(
            "p2p_platform_can_listen_quic",
            flag(s.platform_can_listen_quic),
        ),
        metric("p2p_relay_server_enabled", flag(s.relay_server_enabled)),
        metric("p2p_mediator_enabled", flag(s.mediator_enabled)),
        metric(
            "p2p_mediator_active_reservations",
            s.mediator_active_reservations,
        ),
        metric("p2p_mediator_active_circuits", s.mediator_active_circuits),
        metric(
            "p2p_mediator_dcutr_attempts_observed",
            s.mediator_dcutr_attempts_observed,
        ),
        metric(
            "p2p_mediator_denied_reservations",
            s.mediator_denied_reservations,
        ),
        metric("p2p_mediator_denied_circuits", s.mediator_denied_circuits),
        metric(
            "p2p_mediator_abuse_rate_limit_events",
            s.mediator_abuse_rate_limit_events,
        ),
        metric(
            "p2p_relay_reservations_accepted",
            s.relay_reservations_accepted,
        ),
        metric("p2p_relay_client_reservations", s.relay_client_reservations),
        metric("p2p_relay_active_circuits", s.relay_active_circuits),
        metric("p2p_relay_denied_requests", s.relay_denied_requests),
        metric("p2p_relay_bytes_forwarded", s.relay_bytes_forwarded),
        metric(
            "p2p_relay_discovery_enabled",
            flag(s.relay_discovery_enabled),
        ),
        metric(
            "p2p_relay_discovery_selected_relays",
            s.relay_discovery_selected_relays.len(),
        ),
        metric(
            "p2p_relay_discovery_candidate_count",
            s.relay_discovery_candidate_count,
        ),
        metric(
            "p2p_relay_discovery_public_candidates",
            s.relay_discovery_public_candidates,
        ),
        metric("p2p_relay_discovery_failures", s.relay_discovery_failures),
        metric(
            "p2p_relay_discovery_replacements",
            s.relay_discovery_replacements,
        ),
        metric("p2p_dcutr_enabled", flag(s.dcutr_enabled)),
        metric("p2p_dcutr_attempts", s.dcutr_attempts),
        metric("p2p_dcutr_successes", s.dcutr_successes),
        metric("p2p_dcutr_failures", s.dcutr_failures),
        metric("p2p_dcutr_relay_fallbacks", s.dcutr_relay_fallbacks),
        metric(
            "p2p_dcutr_upgrade_eligible_connections",
            s.dcutr_upgrade_eligible_connections,
        ),
        metric("p2p_dcutr_retry_suppressed", s.dcutr_retry_suppressed),
        metric("p2p_gossip_messages_accepted", s.gossip_messages_accepted),
        metric("p2p_gossip_messages_ignored", s.gossip_messages_ignored),
        metric("p2p_gossip_messages_rejected", s.gossip_messages_rejected),
    ]
}

fn metric(name: &'static str, value: impl ToString) -> (&'static str, String) {
    (name, value.to_string())
}

fn flag(value: bool) -> u8 {
    u8::from(value)
}
