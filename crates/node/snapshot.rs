//! Live node snapshot state and snapshot update helpers.

use std::collections::VecDeque;

use crate::connectivity::relay::{RelayServiceHealth, RelayState};

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct NodeSnapshot {
    pub network_id: u32,
    pub network_label: String,
    pub peer_id: String,
    pub nat_status: String,
    pub public_addr: Option<String>,
    pub public_direct_listen_addresses: Vec<String>,
    pub local_listen_addresses: Vec<String>,
    pub environment_platform: String,
    pub environment_reachability: String,
    pub environment_nat_status: String,
    pub environment_can_accept_inbound: bool,
    pub environment_likely_cgnat: bool,
    pub environment_battery_sensitive: bool,
    pub environment_background_restricted: bool,
    pub platform_runtime: String,
    pub platform_storage: String,
    pub platform_default_data_dir: Option<String>,
    pub platform_can_listen_tcp: bool,
    pub platform_can_listen_quic: bool,
    pub active_transports: Vec<String>,
    pub discovery_namespace_mode: String,
    pub discovery_namespaces: Vec<String>,
    pub discovery_namespace_count: usize,
    pub dht_provider_enabled: bool,
    pub dht_provider_announce_enabled: bool,
    pub dht_provider_discover_enabled: bool,
    pub dht_provider_namespaces_announced: usize,
    pub dht_provider_announce_attempts: usize,
    pub dht_provider_announce_failures: usize,
    pub dht_provider_queries: usize,
    pub dht_provider_query_failures: usize,
    pub dht_provider_records_found: usize,
    pub dht_provider_queries_finished: usize,
    pub dht_provider_peers_discovered: usize,
    pub public_fallback_mode: String,
    pub public_fallback_used: bool,
    pub public_fallback_reason: String,
    pub public_bootstrap_used: bool,
    pub public_bootstrap_reason: String,
    pub public_rendezvous_used: bool,
    pub public_rendezvous_reason: String,
    pub public_relay_used: bool,
    pub public_relay_reason: String,
    pub public_bootstrap_seed_count: usize,
    pub public_rendezvous_candidate_count: usize,
    pub public_relay_candidate_count: usize,
    pub public_ip_probe_enabled: bool,
    pub public_ip_probe_status: String,
    pub public_ip_probe_addr: Option<String>,
    pub public_ip_probe_external_addresses: Vec<String>,
    pub connected_peers: usize,
    pub peer_book_known_peers: usize,
    pub peer_book_discovered_peers: usize,
    pub auto_connect_enabled: bool,
    pub auto_connect_dial_attempts: usize,
    pub auto_connect_dial_failures: usize,
    pub auto_connect_awaiting_address_peers: usize,
    pub connection_plan_pending_peers: usize,
    pub relay_server_enabled: bool,
    pub mediator_enabled: bool,
    pub mediator_advertise_for_dcutr: bool,
    pub mediator_require_authenticated_peers: bool,
    pub mediator_active_reservations: usize,
    pub mediator_active_circuits: usize,
    pub mediator_dcutr_attempts_observed: usize,
    pub mediator_denied_reservations: usize,
    pub mediator_denied_circuits: usize,
    pub mediator_abuse_rate_limit_events: usize,
    pub relay_service_health: RelayServiceHealth,
    /// ACL scope is intentionally connection-level for now because rust-libp2p's stock relay server
    /// does not expose a relay-only reservation hook.
    pub relay_acl_scope: String,
    /// Active reservations accepted by this node while acting as a relay server.
    pub relay_reservations_accepted: usize,
    /// Active circuits currently served by this node as a relay server.
    /// rust-libp2p relay-client events do not expose a matching close event, so client-side
    /// circuit establishment pulses are logged but not counted as active here.
    pub relay_active_circuits: usize,
    /// Total relay requests denied by this relay server, across reservations and circuits.
    pub relay_denied_requests: usize,
    /// Bytes forwarded by this relay server when byte accounting is available.
    /// rust-libp2p relay events currently do not expose per-circuit byte totals, so this remains zero unless a future accounting layer updates it.
    pub relay_bytes_forwarded: u64,
    pub relay_denied_reservations: usize,
    pub relay_denied_circuits: usize,
    pub relay_rate_limited_events: usize,
    pub relay_at_capacity_events: usize,
    pub relay_server_errors: usize,
    pub connection_limit_events: usize,
    pub connection_cap_disconnects: usize,
    /// Relay reservations this node has established as a relay client.
    pub relay_client_reservations: usize,
    /// Relay reservation attempts this node has initiated as a relay client.
    pub relay_client_reservation_attempts: usize,
    /// Relay reservation failures observed locally while setting up configured relays.
    pub relay_client_reservation_failures: usize,
    /// Whether automatic relay discovery/selection is enabled.
    pub relay_discovery_enabled: bool,
    /// Minimum desired relay reservations for lite/mobile nodes.
    pub relay_discovery_min_reservations: usize,
    /// Maximum relay candidates/reservation attempts selected at once.
    pub relay_discovery_max_reservations: usize,
    /// Relay addresses selected from configured/cache/rendezvous sources.
    pub relay_discovery_selected_relays: Vec<String>,
    pub relay_discovery_candidate_count: usize,
    pub relay_discovery_configured_candidates: usize,
    pub relay_discovery_cached_candidates: usize,
    pub relay_discovery_rendezvous_candidates: usize,
    pub relay_discovery_public_candidates: usize,
    pub relay_discovery_ignored_candidates: usize,
    pub relay_discovery_failures: usize,
    pub relay_discovery_replacements: usize,
    /// Confirmed `/p2p-circuit` listen addresses for this node.
    pub relayed_listen_addresses: Vec<String>,
    pub dcutr_enabled: bool,
    pub dcutr_attempt_after_relay_connection: bool,
    pub dcutr_keep_relay_fallback: bool,
    pub dcutr_retry_interval_secs: u64,
    pub dcutr_max_attempts_per_peer: u32,
    /// DCUtR upgrade attempts/events observed.
    pub dcutr_attempts: usize,
    /// DCUtR events that look successful by event debug output.
    pub dcutr_successes: usize,
    pub dcutr_failures: usize,
    pub dcutr_relay_fallbacks: usize,
    pub dcutr_upgrade_eligible_connections: usize,
    pub dcutr_retry_suppressed: usize,
    pub rendezvous_client_enabled: bool,
    pub rendezvous_server_enabled: bool,
    pub rendezvous_registered_with: usize,
    pub rendezvous_discovered_peers: usize,
    pub rendezvous_register_attempts: usize,
    pub rendezvous_register_failures: usize,
    pub rendezvous_discover_attempts: usize,
    pub rendezvous_discover_failures: usize,
    pub rendezvous_server_registrations: usize,
    pub rendezvous_server_discoveries_served: usize,
    pub rendezvous_server_errors: usize,
    pub app_subscriptions: Vec<String>,
    pub app_messages_sent: usize,
    pub app_messages_received: usize,
    pub app_messages_ignored: usize,
    pub app_messages_rejected: usize,
    pub api_commands_processed: usize,
    pub api_command_failures: usize,
    pub gossip_messages_rejected: usize,
    pub gossip_messages_ignored: usize,
    pub gossip_messages_accepted: usize,
    pub pulses: VecDeque<String>,
    pub uptime_secs: u64,
}

impl NodeSnapshot {
    pub(crate) fn record_public_external_addr(&mut self, addr: impl Into<String>) {
        let addr = addr.into();
        if !self.public_direct_listen_addresses.contains(&addr) {
            self.public_direct_listen_addresses.push(addr.clone());
        }
        if !self.public_ip_probe_external_addresses.contains(&addr) {
            self.public_ip_probe_external_addresses.push(addr.clone());
        }
        if self.public_addr.is_none() {
            self.public_addr = Some(addr);
        }
    }

    /// Apply relay/DCUtR counters from the authoritative event-loop relay state.
    pub fn apply_relay_state(&mut self, relay_state: &RelayState) {
        self.relay_server_enabled = relay_state.server_enabled;
        self.relay_service_health = relay_state.health;
        if self.mediator_enabled {
            self.mediator_active_reservations = relay_state.accepted_reservations;
            self.mediator_active_circuits = relay_state.active_circuits;
            self.mediator_dcutr_attempts_observed = relay_state.dcutr_attempts;
            self.mediator_denied_reservations = relay_state.denied_reservations;
            self.mediator_denied_circuits = relay_state.denied_circuits;
            self.mediator_abuse_rate_limit_events = relay_state
                .rate_limited_events
                .saturating_add(relay_state.at_capacity_events);
        }
        self.relay_reservations_accepted = relay_state.accepted_reservations;
        self.relay_client_reservations = relay_state.relay_client_reservations.len();
        self.relay_client_reservation_attempts = relay_state.relay_client_reservation_attempts;
        self.relay_client_reservation_failures = relay_state.relay_client_reservation_failures;
        self.relay_discovery_selected_relays = relay_state
            .relay_discovery_selected_relays
            .iter()
            .cloned()
            .collect();
        self.relay_discovery_candidate_count = relay_state.relay_discovery_candidate_count;
        self.relay_discovery_configured_candidates =
            relay_state.relay_discovery_configured_candidates;
        self.relay_discovery_cached_candidates = relay_state.relay_discovery_cached_candidates;
        self.relay_discovery_rendezvous_candidates =
            relay_state.relay_discovery_rendezvous_candidates;
        self.relay_discovery_public_candidates = relay_state.relay_discovery_public_candidates;
        self.relay_discovery_ignored_candidates = relay_state.relay_discovery_ignored_candidates;
        self.relay_discovery_failures = relay_state.relay_discovery_failures;
        self.relay_discovery_replacements = relay_state.relay_discovery_replacements;
        self.relay_active_circuits = relay_state.active_circuits;
        self.relay_denied_reservations = relay_state.denied_reservations;
        self.relay_denied_circuits = relay_state.denied_circuits;
        self.relay_denied_requests = relay_state
            .denied_reservations
            .saturating_add(relay_state.denied_circuits);
        self.relay_rate_limited_events = relay_state.rate_limited_events;
        self.relay_at_capacity_events = relay_state.at_capacity_events;
        self.relay_server_errors = relay_state.server_errors;
        self.relay_bytes_forwarded = relay_state.relay_bytes_forwarded;
        self.relayed_listen_addresses = relay_state.relayed_listen_addrs.iter().cloned().collect();
        if self.public_addr.is_none() {
            self.public_addr = self.relayed_listen_addresses.first().cloned();
        }
        self.dcutr_enabled = relay_state.dcutr_enabled;
        self.dcutr_attempts = relay_state.dcutr_attempts;
        self.dcutr_successes = relay_state.dcutr_successes;
        self.dcutr_failures = relay_state.dcutr_failures;
        self.dcutr_relay_fallbacks = relay_state.dcutr_relay_fallbacks;
        self.dcutr_upgrade_eligible_connections = relay_state.dcutr_upgrade_eligible_connections;
        self.dcutr_retry_suppressed = relay_state.dcutr_retry_suppressed;
    }
}

pub(crate) fn network_label(network_id: u32) -> String {
    if network_id == 0 {
        "MAINNET".to_string()
    } else {
        format!("TESTNET-{network_id}")
    }
}
