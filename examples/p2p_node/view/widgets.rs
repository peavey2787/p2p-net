use p2p_net::NodeSnapshot;

use super::text::{char_width, clip_text, sanitize_terminal_text};
use super::{Line, Span, Tone};

pub(super) fn panel_header(width: usize, title: &str) -> Line {
    if width < 7 {
        let mut line = Line::default();
        line.push("─".repeat(width), Tone::Muted);
        return line;
    }

    let safe_title = sanitize_terminal_text(title);
    let title = clip_text(&safe_title, width.saturating_sub(6));
    let occupied = 4usize.saturating_add(char_width(&title));
    let fill = width.saturating_sub(occupied.saturating_add(1));
    let mut line = Line::default();
    line.push("╭─ ", Tone::Muted);
    line.push_bold(title, Tone::Accent);
    line.push(" ", Tone::Muted);
    line.push("─".repeat(fill), Tone::Muted);
    line.push("╮", Tone::Muted);
    line
}

pub(super) fn panel_footer(width: usize) -> Line {
    let mut line = Line::default();
    match width {
        0 => {}
        1 => line.push("╯", Tone::Muted),
        _ => {
            line.push("╰", Tone::Muted);
            line.push("─".repeat(width.saturating_sub(2)), Tone::Muted);
            line.push("╯", Tone::Muted);
        }
    }
    line
}

pub(super) fn panel_content(width: usize, content: Line) -> Line {
    if width < 4 {
        return clip_line(content, width);
    }

    let inner_width = width - 4;
    let clipped = clip_line(content, inner_width);
    let used = clipped.visible_width();
    let mut line = Line::default();
    line.push("│ ", Tone::Muted);
    line.spans.extend(clipped.spans);
    line.push(" ".repeat(inner_width.saturating_sub(used)), Tone::Text);
    line.push(" │", Tone::Muted);
    line
}

fn clip_line(line: Line, width: usize) -> Line {
    let mut remaining = width;
    let mut clipped = Line::default();
    for span in line.spans {
        if remaining == 0 {
            break;
        }
        let text = clip_text(&span.text, remaining);
        let used = char_width(&text);
        if used == 0 {
            continue;
        }
        clipped.spans.push(Span {
            text,
            tone: span.tone,
            bold: span.bold,
        });
        remaining = remaining.saturating_sub(used);
    }
    clipped
}

pub(super) fn push_metric(line: &mut Line, label: &str, value: usize, nonzero_tone: Tone) {
    if !line.spans.is_empty() {
        line.push("   ", Tone::Text);
    }
    line.push_bold(format!("{label} "), Tone::Muted);
    line.push_bold(
        value.to_string(),
        if value == 0 {
            Tone::Muted
        } else {
            nonzero_tone
        },
    );
}

pub(super) fn push_switch(line: &mut Line, enabled: bool) {
    if enabled {
        line.push_bold("ON", Tone::Good);
    } else {
        line.push_bold("OFF", Tone::Muted);
    }
}

pub(super) fn push_bool(line: &mut Line, value: bool) {
    line.push_bold(
        if value { "yes" } else { "no" },
        if value { Tone::Good } else { Tone::Muted },
    );
}

pub(super) fn public_addr_display(snap: &NodeSnapshot) -> String {
    if let Some(addr) = &snap.public_addr {
        return addr.clone();
    }
    if !snap.relay_discovery_selected_relays.is_empty() {
        return "none yet; relay fallback selected".to_string();
    }
    if snap.public_relay_candidate_count > 0 {
        return "none yet; public relay candidates available".to_string();
    }
    if snap.public_ip_probe_enabled {
        return format!(
            "none yet; probe={} ip={}",
            snap.public_ip_probe_status,
            snap.public_ip_probe_addr.as_deref().unwrap_or("-")
        );
    }
    "none yet; no public direct/relayed addr".to_string()
}

pub(super) fn local_listen_display(snap: &NodeSnapshot) -> String {
    join_or_dash(&snap.local_listen_addresses)
}

pub(super) fn join_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join("  •  ")
    }
}
