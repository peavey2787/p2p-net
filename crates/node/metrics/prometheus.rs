//! Snapshot-to-metrics rendering for operator observability.

use std::fmt::Display;

use crate::node::snapshot::NodeSnapshot;

/// Export operator counters in Prometheus text exposition format without opening an HTTP port.
/// Embedders that want an HTTP endpoint can serve this string from their own trusted admin server.
pub(crate) fn snapshot_to_prometheus_metrics(snapshot: &NodeSnapshot) -> String {
    let s = snapshot;
    let mut out = String::new();

    emit(&mut out, "p2p_connected_peers", s.connected_peers);
    emit(&mut out, "p2p_peer_book_known_peers", s.peer_book_known_peers);
    emit(&mut out, "p2p_peer_book_discovered_peers", s.peer_book_discovered_peers);
    emit(&mut out, "p2p_discovery_namespace_count", s.discovery_namespace_count);
    emit(&mut out, "p2p_dht_provider_enabled", flag(s.dht_provider_enabled));
    emit(&mut out, "p2p_dht_provider_announce_attempts", s.dht_provider_announce_attempts);
    emit(&mut out, "p2p_dht_provider_announce_failures", s.dht_provider_announce_failures);
    emit(&mut out, "p2p_dht_provider_queries", s.dht_provider_queries);
    emit(&mut out, "p2p_dht_provider_query_failures", s.dht_provider_query_failures);
    emit(&mut out, "p2p_dht_provider_records_found", s.dht_provider_records_found);
    emit(&mut out, "p2p_dht_provider_queries_finished", s.dht_provider_queries_finished);
    emit(&mut out, "p2p_dht_provider_peers_discovered", s.dht_provider_peers_discovered);
    emit(&mut out, "p2p_public_fallback_used", flag(s.public_fallback_used));
    emit(&mut out, "p2p_public_bootstrap_seed_count", s.public_bootstrap_seed_count);
    emit(&mut out, "p2p_public_relay_candidate_count", s.public_relay_candidate_count);
    emit(&mut out, "p2p_api_commands_processed", s.api_commands_processed);
    emit(&mut out, "p2p_api_command_failures", s.api_command_failures);
    emit(&mut out, "p2p_app_subscriptions", s.app_subscriptions.len());
    emit(&mut out, "p2p_app_messages_sent", s.app_messages_sent);
    emit(&mut out, "p2p_app_messages_received", s.app_messages_received);
    emit(&mut out, "p2p_app_messages_ignored", s.app_messages_ignored);
    emit(&mut out, "p2p_app_messages_rejected", s.app_messages_rejected);
    emit(&mut out, "p2p_platform_can_listen_tcp", flag(s.platform_can_listen_tcp));
    emit(&mut out, "p2p_platform_can_listen_quic", flag(s.platform_can_listen_quic));
    emit(&mut out, "p2p_relay_server_enabled", flag(s.relay_server_enabled));
    emit(&mut out, "p2p_mediator_enabled", flag(s.mediator_enabled));
    emit(&mut out, "p2p_mediator_active_reservations", s.mediator_active_reservations);
    emit(&mut out, "p2p_mediator_active_circuits", s.mediator_active_circuits);
    emit(&mut out, "p2p_mediator_dcutr_attempts_observed", s.mediator_dcutr_attempts_observed);
    emit(&mut out, "p2p_mediator_denied_reservations", s.mediator_denied_reservations);
    emit(&mut out, "p2p_mediator_denied_circuits", s.mediator_denied_circuits);
    emit(&mut out, "p2p_mediator_abuse_rate_limit_events", s.mediator_abuse_rate_limit_events);
    emit(&mut out, "p2p_relay_reservations_accepted", s.relay_reservations_accepted);
    emit(&mut out, "p2p_relay_client_reservations", s.relay_client_reservations);
    emit(&mut out, "p2p_relay_active_circuits", s.relay_active_circuits);
    emit(&mut out, "p2p_relay_denied_requests", s.relay_denied_requests);
    emit(&mut out, "p2p_relay_bytes_forwarded", s.relay_bytes_forwarded);
    emit(&mut out, "p2p_relay_discovery_enabled", flag(s.relay_discovery_enabled));
    emit(&mut out, "p2p_relay_discovery_selected_relays", s.relay_discovery_selected_relays.len());
    emit(&mut out, "p2p_relay_discovery_candidate_count", s.relay_discovery_candidate_count);
    emit(&mut out, "p2p_relay_discovery_public_candidates", s.relay_discovery_public_candidates);
    emit(&mut out, "p2p_relay_discovery_failures", s.relay_discovery_failures);
    emit(&mut out, "p2p_relay_discovery_replacements", s.relay_discovery_replacements);
    emit(&mut out, "p2p_dcutr_enabled", flag(s.dcutr_enabled));
    emit(&mut out, "p2p_dcutr_attempts", s.dcutr_attempts);
    emit(&mut out, "p2p_dcutr_successes", s.dcutr_successes);
    emit(&mut out, "p2p_dcutr_failures", s.dcutr_failures);
    emit(&mut out, "p2p_dcutr_relay_fallbacks", s.dcutr_relay_fallbacks);
    emit(&mut out, "p2p_dcutr_upgrade_eligible_connections", s.dcutr_upgrade_eligible_connections);
    emit(&mut out, "p2p_dcutr_retry_suppressed", s.dcutr_retry_suppressed);
    emit(&mut out, "p2p_gossip_messages_accepted", s.gossip_messages_accepted);
    emit(&mut out, "p2p_gossip_messages_ignored", s.gossip_messages_ignored);
    emit(&mut out, "p2p_gossip_messages_rejected", s.gossip_messages_rejected);

    out
}

fn emit(out: &mut String, name: &str, value: impl Display) {
    out.push_str(name);
    out.push(' ');
    out.push_str(&value.to_string());
    out.push('\n');
}

fn flag(value: bool) -> u8 {
    u8::from(value)
}
