use p2p_net::NodeSnapshot;

use super::text::{failure_tone, status_tone};
use super::widgets::{panel_content, panel_footer, panel_header, push_bool, push_switch};
use super::{Line, Tone};

pub(super) fn append_services(lines: &mut Vec<Line>, snap: &NodeSnapshot, width: usize) {
    lines.push(panel_header(width, "DISCOVERY / RELAY / DHT"));

    let mut fallback = Line::default();
    fallback.push_bold("FALLBACK  ", Tone::Muted);
    fallback.push_bold(&snap.public_fallback_mode, Tone::Accent);
    fallback.push("   bootstrap=", Tone::Muted);
    push_bool(&mut fallback, snap.public_bootstrap_used);
    fallback.push("  rendezvous=", Tone::Muted);
    push_bool(&mut fallback, snap.public_rendezvous_used);
    fallback.push("  relay=", Tone::Muted);
    push_bool(&mut fallback, snap.public_relay_used);
    fallback.push("  rv_candidates=", Tone::Muted);
    fallback.push(
        snap.public_rendezvous_candidate_count.to_string(),
        Tone::Text,
    );
    fallback.push("   reason=", Tone::Muted);
    fallback.push(&snap.public_fallback_reason, Tone::Text);
    lines.push(panel_content(width, fallback));

    let mut relay = Line::default();
    relay.push_bold("RELAY SERVICE  ", Tone::Muted);
    push_switch(&mut relay, snap.relay_server_enabled);
    relay.push("  health=", Tone::Muted);
    relay.push(
        snap.relay_service_health.as_str(),
        status_tone(snap.relay_service_health.as_str()),
    );
    relay.push("  mediator=", Tone::Muted);
    push_bool(&mut relay, snap.mediator_enabled);
    relay.push("  server_res=", Tone::Muted);
    relay.push(snap.relay_reservations_accepted.to_string(), Tone::Text);
    relay.push("  accepted=", Tone::Muted);
    relay.push(
        snap.relay_reservations_accepted_total.to_string(),
        Tone::Text,
    );
    relay.push("  client_res=", Tone::Muted);
    relay.push(snap.relay_client_reservations.to_string(), Tone::Text);
    relay.push("  attempts=", Tone::Muted);
    relay.push(
        snap.relay_client_reservation_attempts.to_string(),
        Tone::Text,
    );
    relay.push("  failures=", Tone::Muted);
    relay.push(
        snap.relay_client_reservation_failures.to_string(),
        failure_tone(snap.relay_client_reservation_failures),
    );
    lines.push(panel_content(width, relay));

    let mut relay_discovery = Line::default();
    relay_discovery.push_bold("RELAY DISCOVERY  ", Tone::Muted);
    relay_discovery.push("selected=", Tone::Muted);
    relay_discovery.push(
        snap.relay_discovery_selected_relays.len().to_string(),
        Tone::Text,
    );
    relay_discovery.push("  candidates=", Tone::Muted);
    relay_discovery.push(snap.relay_discovery_candidate_count.to_string(), Tone::Text);
    relay_discovery.push("  failures=", Tone::Muted);
    relay_discovery.push(
        snap.relay_discovery_failures.to_string(),
        failure_tone(snap.relay_discovery_failures),
    );
    lines.push(panel_content(width, relay_discovery));

    let mut circuits = Line::default();
    circuits.push_bold("CIRCUITS  ", Tone::Muted);
    circuits.push(format!("active={}", snap.relay_active_circuits), Tone::Text);
    circuits.push("  denied=", Tone::Muted);
    circuits.push(
        snap.relay_denied_requests.to_string(),
        failure_tone(snap.relay_denied_requests),
    );
    circuits.push("  bytes_fwd=", Tone::Muted);
    circuits.push(snap.relay_bytes_forwarded.to_string(), Tone::Text);
    circuits.push("   ", Tone::Text);
    circuits.push_bold("DCUtR  ", Tone::Muted);
    push_switch(&mut circuits, snap.dcutr_enabled);
    circuits.push(
        format!(
            "  tries={} ok={} fail={} fallback={} suppressed={}",
            snap.dcutr_attempts,
            snap.dcutr_successes,
            snap.dcutr_failures,
            snap.dcutr_relay_fallbacks,
            snap.dcutr_retry_suppressed
        ),
        Tone::Text,
    );
    lines.push(panel_content(width, circuits));

    let mut dht = Line::default();
    dht.push_bold("DHT PROVIDER  ", Tone::Muted);
    push_switch(&mut dht, snap.dht_provider_enabled);
    dht.push(
        format!(
            "  announced={} queries={} found={} peers={} query_failures={}",
            snap.dht_provider_namespaces_announced,
            snap.dht_provider_queries,
            snap.dht_provider_records_found,
            snap.dht_provider_peers_discovered,
            snap.dht_provider_query_failures
        ),
        Tone::Text,
    );
    lines.push(panel_content(width, dht));

    lines.push(panel_footer(width));
}
