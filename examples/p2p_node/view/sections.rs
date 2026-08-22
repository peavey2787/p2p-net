use p2p_net::NodeSnapshot;

use super::text::{
    count_tone, event_tone, failure_tone, format_uptime, status_tone, wrap_terminal_text,
};
use super::widgets::{
    join_or_dash, local_listen_display, panel_content, panel_footer, panel_header,
    public_addr_display, push_bool, push_metric, push_switch,
};
use super::{Line, Tone};

pub(super) fn append_header(lines: &mut Vec<Line>, snap: &NodeSnapshot, width: usize) {
    lines.push(panel_header(width, "P2P-NET  /  FULL NODE"));

    let mut primary = Line::default();
    primary.push_bold("NETWORK  ", Tone::Muted);
    primary.push_bold(&snap.network_label, Tone::Accent);
    primary.push("   ", Tone::Text);
    primary.push_bold("SWARM  ", Tone::Muted);
    primary.push_bold(
        snap.all_swarm_connections.to_string(),
        count_tone(snap.all_swarm_connections),
    );
    primary.push("   ", Tone::Text);
    primary.push_bold("UPTIME  ", Tone::Muted);
    primary.push_bold(format_uptime(snap.uptime_secs), Tone::Text);
    primary.push("   q / Esc  quit", Tone::Muted);
    lines.push(panel_content(width, primary));

    let mut identity = Line::default();
    identity.push_bold("PEER  ", Tone::Muted);
    identity.push(&snap.peer_id, Tone::Text);
    lines.push(panel_content(width, identity));
    lines.push(panel_footer(width));
}

pub(super) fn append_reachability(lines: &mut Vec<Line>, snap: &NodeSnapshot, width: usize) {
    lines.push(panel_header(width, "REACHABILITY"));

    let mut nat = Line::default();
    nat.push_bold("NAT  ", Tone::Muted);
    nat.push_bold(&snap.nat_status, status_tone(&snap.nat_status));
    nat.push("   ", Tone::Text);
    nat.push_bold("PUBLIC  ", Tone::Muted);
    nat.push(public_addr_display(snap), Tone::Text);
    lines.push(panel_content(width, nat));

    let mut probe = Line::default();
    probe.push_bold("PUBLIC IP PROBE  ", Tone::Muted);
    push_switch(&mut probe, snap.public_ip_probe_enabled);
    probe.push("   status=", Tone::Muted);
    probe.push(
        &snap.public_ip_probe_status,
        status_tone(&snap.public_ip_probe_status),
    );
    probe.push("   ip=", Tone::Muted);
    probe.push(
        snap.public_ip_probe_addr.as_deref().unwrap_or("-"),
        Tone::Text,
    );
    probe.push("   external=", Tone::Muted);
    probe.push(
        snap.public_ip_probe_external_addresses.len().to_string(),
        Tone::Text,
    );
    lines.push(panel_content(width, probe));

    let mut listen = Line::default();
    listen.push_bold("LOCAL LISTEN  ", Tone::Muted);
    listen.push(local_listen_display(snap), Tone::Text);
    lines.push(panel_content(width, listen));

    let mut platform = Line::default();
    platform.push_bold("PLATFORM  ", Tone::Muted);
    platform.push(&snap.environment_platform, Tone::Text);
    platform.push("  /  ", Tone::Muted);
    platform.push(&snap.platform_runtime, Tone::Text);
    platform.push("  /  ", Tone::Muted);
    platform.push(&snap.platform_storage, Tone::Text);
    platform.push("   ", Tone::Text);
    platform.push_bold("TRANSPORTS  ", Tone::Muted);
    platform.push(join_or_dash(&snap.active_transports), Tone::Accent);
    lines.push(panel_content(width, platform));

    lines.push(panel_footer(width));
}

pub(super) fn append_peer_mesh(lines: &mut Vec<Line>, snap: &NodeSnapshot, width: usize) {
    lines.push(panel_header(width, "PEER MESH"));

    let mut peers = Line::default();
    push_metric(
        &mut peers,
        "APP",
        snap.application_peer_connections,
        Tone::Good,
    );
    push_metric(
        &mut peers,
        "INFRA",
        snap.infrastructure_peer_connections,
        Tone::Text,
    );
    push_metric(
        &mut peers,
        "DHT",
        snap.dht_routing_peer_connections,
        Tone::Text,
    );
    push_metric(&mut peers, "RELAY", snap.relay_peer_connections, Tone::Text);
    push_metric(
        &mut peers,
        "SWARM",
        snap.all_swarm_connections,
        Tone::Accent,
    );
    lines.push(panel_content(width, peers));

    let mut discovery = Line::default();
    push_metric(&mut discovery, "KNOWN", snap.peer_book_known_peers, Tone::Text);
    push_metric(
        &mut discovery,
        "DISCOVERED",
        snap.peer_book_discovered_peers,
        Tone::Text,
    );
    push_metric(
        &mut discovery,
        "PENDING",
        snap.connection_plan_pending_peers,
        Tone::Warn,
    );
    push_metric(
        &mut discovery,
        "AWAITING ADDRS",
        snap.auto_connect_awaiting_address_peers,
        Tone::Warn,
    );
    lines.push(panel_content(width, discovery));

    let mut auto = Line::default();
    auto.push_bold("AUTO CONNECT  ", Tone::Muted);
    push_switch(&mut auto, snap.auto_connect_enabled);
    auto.push("   attempts=", Tone::Muted);
    auto.push(snap.auto_connect_dial_attempts.to_string(), Tone::Text);
    auto.push("   failures=", Tone::Muted);
    auto.push(
        snap.auto_connect_dial_failures.to_string(),
        failure_tone(snap.auto_connect_dial_failures),
    );
    if let Some(error) = &snap.last_application_dial_error {
        auto.push("   last=", Tone::Muted);
        auto.push(error, Tone::Warn);
    }
    lines.push(panel_content(width, auto));

    let mut traffic = Line::default();
    traffic.push_bold("APP MSG  ", Tone::Muted);
    traffic.push(
        format!(
            "tx={} rx={}",
            snap.app_messages_sent, snap.app_messages_received
        ),
        Tone::Text,
    );
    traffic.push("  rejected=", Tone::Muted);
    traffic.push(
        snap.app_messages_rejected.to_string(),
        failure_tone(snap.app_messages_rejected),
    );
    traffic.push("   ", Tone::Text);
    traffic.push_bold("GOSSIP  ", Tone::Muted);
    traffic.push(
        format!("ok={}", snap.gossip_messages_accepted),
        Tone::Text,
    );
    traffic.push(" rejected=", Tone::Muted);
    traffic.push(
        snap.gossip_messages_rejected.to_string(),
        failure_tone(snap.gossip_messages_rejected),
    );
    lines.push(panel_content(width, traffic));

    lines.push(panel_footer(width));
}

pub(super) fn append_compact_summary(lines: &mut Vec<Line>, snap: &NodeSnapshot, width: usize) {
    lines.push(panel_header(width, "NODE SUMMARY"));

    let mut reach = Line::default();
    reach.push_bold("NAT  ", Tone::Muted);
    reach.push(&snap.nat_status, status_tone(&snap.nat_status));
    reach.push("   public=", Tone::Muted);
    reach.push(public_addr_display(snap), Tone::Text);
    lines.push(panel_content(width, reach));

    let mut peers = Line::default();
    push_metric(
        &mut peers,
        "APP",
        snap.application_peer_connections,
        Tone::Good,
    );
    push_metric(
        &mut peers,
        "INFRA",
        snap.infrastructure_peer_connections,
        Tone::Text,
    );
    push_metric(
        &mut peers,
        "DHT",
        snap.dht_routing_peer_connections,
        Tone::Text,
    );
    push_metric(&mut peers, "RELAY", snap.relay_peer_connections, Tone::Text);
    lines.push(panel_content(width, peers));

    let mut auto = Line::default();
    auto.push_bold("AUTO  ", Tone::Muted);
    push_switch(&mut auto, snap.auto_connect_enabled);
    auto.push("  attempts=", Tone::Muted);
    auto.push(snap.auto_connect_dial_attempts.to_string(), Tone::Text);
    auto.push("  failures=", Tone::Muted);
    auto.push(
        snap.auto_connect_dial_failures.to_string(),
        failure_tone(snap.auto_connect_dial_failures),
    );
    lines.push(panel_content(width, auto));

    let mut services = Line::default();
    services.push_bold("DHT  ", Tone::Muted);
    push_switch(&mut services, snap.dht_provider_enabled);
    services.push("   ", Tone::Text);
    services.push_bold("RELAY  ", Tone::Muted);
    push_switch(&mut services, snap.relay_server_enabled);
    services.push("   ", Tone::Text);
    services.push_bold("DCUtR  ", Tone::Muted);
    push_switch(&mut services, snap.dcutr_enabled);
    services.push("   transports=", Tone::Muted);
    services.push(join_or_dash(&snap.active_transports), Tone::Accent);
    lines.push(panel_content(width, services));

    lines.push(panel_footer(width));
}

pub(super) fn append_events(
    lines: &mut Vec<Line>,
    snap: &NodeSnapshot,
    width: usize,
    rows: usize,
) {
    if lines.len() >= rows {
        return;
    }

    lines.push(panel_header(width, "LIVE EVENT STREAM  /  NEWEST FIRST"));
    if lines.len() >= rows {
        return;
    }

    let rows_for_events = rows.saturating_sub(lines.len().saturating_add(1));
    if rows_for_events == 0 {
        lines.push(panel_footer(width));
        return;
    }

    let mut used = 0usize;
    if snap.pulses.is_empty() {
        let mut waiting = Line::default();
        waiting.push("Waiting for network events...", Tone::Muted);
        lines.push(panel_content(width, waiting));
        used = 1;
    } else {
        let inner_width = width.saturating_sub(4).max(1);
        let text_width = inner_width.saturating_sub(2).max(1);

        'events: for pulse in &snap.pulses {
            let tone = event_tone(pulse);
            for (index, wrapped) in wrap_terminal_text(pulse, text_width)
                .into_iter()
                .enumerate()
            {
                if used >= rows_for_events {
                    break 'events;
                }
                let mut event = Line::default();
                event.push(if index == 0 { "• " } else { "  " }, Tone::Accent);
                event.push(wrapped, tone);
                lines.push(panel_content(width, event));
                used = used.saturating_add(1);
            }
        }
    }

    while used < rows_for_events {
        lines.push(panel_content(width, Line::default()));
        used = used.saturating_add(1);
    }
    if lines.len() < rows {
        lines.push(panel_footer(width));
    }
}
