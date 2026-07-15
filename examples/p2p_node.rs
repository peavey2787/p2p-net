//! Live standalone P2P node dashboard.
//!
//! Run:
//! `cargo run --features dashboard --example p2p_node`

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::time::SystemTime;

use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use p2p_net::{start_node, NodeConfig, NodeSnapshot};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Terminal;

static BACKGROUND_WORKER_PANICKED: AtomicBool = AtomicBool::new(false);
const PANIC_LOG_PATH: &str = "p2p-node-panic.log";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    install_dashboard_panic_hook();

    let args: Vec<String> = std::env::args().collect();

    if let Some(path) = arg_value(&args, "--write-default-config") {
        std::fs::write(path, NodeConfig::default().to_pretty_json()?)?;
        println!("wrote default config to {path}");
        return Ok(());
    }

    let cfg = if let Some(path) = arg_value(&args, "--config") {
        NodeConfig::load_json_file(path)?
    } else {
        NodeConfig::default()
    };

    let handle = start_node(cfg).await?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let res = run_ui(&mut terminal, handle).await;
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    res
}

async fn run_ui(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    handle: p2p_net::NodeHandle,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        if BACKGROUND_WORKER_PANICKED.load(Ordering::Relaxed) {
            handle.shutdown().await;
            return Err(format!(
                "background worker panicked; terminal restored; see {PANIC_LOG_PATH}"
            )
            .into());
        }

        let snap = handle.snapshot.lock().await.clone();
        terminal.draw(|f| draw_dashboard(f, &snap))?;
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') || key.code == KeyCode::Esc {
                    handle.shutdown().await;
                    break;
                }
            }
        }
    }
    Ok(())
}

fn public_addr_display(snap: &NodeSnapshot) -> String {
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
            "none yet; public_ip_probe status={} ip={}",
            snap.public_ip_probe_status,
            snap.public_ip_probe_addr.as_deref().unwrap_or("-")
        );
    }
    "none yet; no public direct/relayed addr".to_string()
}

fn public_ip_probe_display(snap: &NodeSnapshot) -> String {
    format!(
        "enabled={} status={} ip={} external_addrs={}",
        snap.public_ip_probe_enabled,
        snap.public_ip_probe_status,
        snap.public_ip_probe_addr.as_deref().unwrap_or("-"),
        snap.public_ip_probe_external_addresses.len()
    )
}

fn local_listen_display(snap: &NodeSnapshot) -> String {
    if snap.local_listen_addresses.is_empty() {
        "-".to_string()
    } else {
        snap.local_listen_addresses.join(", ")
    }
}

fn draw_dashboard(frame: &mut ratatui::Frame<'_>, snap: &NodeSnapshot) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Length(4),
            Constraint::Length(12),
            Constraint::Min(6),
        ])
        .split(frame.area());

    let status = Paragraph::new(format!(
        "Network: {}\nPeerID: {}\nNAT/Public: {} / {}\nPublic IP Probe: {}\nLocal Listen: {}\nPlatform: {} runtime={} storage={}",
        snap.network_label,
        snap.peer_id,
        snap.nat_status,
        public_addr_display(snap),
        public_ip_probe_display(snap),
        local_listen_display(snap),
        snap.environment_platform,
        snap.platform_runtime,
        snap.platform_storage
    ))
    .block(
        Block::default()
            .title("PeerID & Status")
            .borders(Borders::ALL),
    );
    frame.render_widget(status, chunks[0]);

    let transports = Paragraph::new(snap.active_transports.join(", ")).block(
        Block::default()
            .title("Transports Active")
            .borders(Borders::ALL),
    );
    frame.render_widget(transports, chunks[1]);

    let mesh = Paragraph::new(format!(
        "Application Peers: {} | Infrastructure Peers: {} | DHT Routing Peers: {} | Relay Peers: {} | All Swarm Connections: {}\nPeerBook: known {} discovered {}\nAuto-Connect: enabled={} dial_attempts={} failures={} pending_plans={} awaiting_addrs={}\nPublic Fallback: mode={} bootstrap={} rendezvous={} relay={} public_rv_candidates={} reason={}\nRelay Server: {} ({}) | Mediator: {}\nServer Reservations: {} | Client Reservations: {} / attempts {} failures {}\nRelay Discovery: selected {} / candidates {} failures {}\nDHT Provider: enabled={} announced={} queries={} found={} peers={}\nActive Circuits: {} | Denied Requests: {} | Bytes Fwd: {} | DCUtR: enabled={} attempts={} successes={} failures={} fallback={} suppressed={}",
        snap.application_peer_connections,
        snap.infrastructure_peer_connections,
        snap.dht_routing_peer_connections,
        snap.relay_peer_connections,
        snap.all_swarm_connections,
        snap.peer_book_known_peers,
        snap.peer_book_discovered_peers,
        snap.auto_connect_enabled,
        snap.auto_connect_dial_attempts,
        snap.auto_connect_dial_failures,
        snap.connection_plan_pending_peers,
        snap.auto_connect_awaiting_address_peers,
        snap.public_fallback_mode,
        snap.public_bootstrap_used,
        snap.public_rendezvous_used,
        snap.public_relay_used,
        snap.public_rendezvous_candidate_count,
        snap.public_fallback_reason,
        snap.relay_server_enabled,
        snap.relay_service_health.as_str(),
        snap.mediator_enabled,
        snap.relay_reservations_accepted,
        snap.relay_client_reservations,
        snap.relay_client_reservation_attempts,
        snap.relay_client_reservation_failures,
        snap.relay_discovery_selected_relays.len(),
        snap.relay_discovery_candidate_count,
        snap.relay_discovery_failures,
        snap.dht_provider_enabled,
        snap.dht_provider_namespaces_announced,
        snap.dht_provider_queries,
        snap.dht_provider_records_found,
        snap.dht_provider_peers_discovered,
        snap.relay_active_circuits,
        snap.relay_denied_requests,
        snap.relay_bytes_forwarded,
        snap.dcutr_enabled,
        snap.dcutr_attempts,
        snap.dcutr_successes,
        snap.dcutr_failures,
        snap.dcutr_relay_fallbacks,
        snap.dcutr_retry_suppressed
    ))
    .block(Block::default().title("Mesh / Relay").borders(Borders::ALL));
    frame.render_widget(mesh, chunks[2]);

    let pulse_items: Vec<ListItem<'_>> = snap
        .pulses
        .iter()
        .map(|s| ListItem::new(s.clone()))
        .collect();
    let pulse = List::new(pulse_items).block(
        Block::default()
            .title("Heartbeat Gossip")
            .borders(Borders::ALL),
    );
    frame.render_widget(pulse, chunks[3]);
}

fn arg_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
}

fn install_dashboard_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        BACKGROUND_WORKER_PANICKED.store(true, Ordering::Relaxed);

        let _ = disable_raw_mode();
        let mut stderr = io::stderr();
        let _ = execute!(stderr, LeaveAlternateScreen);

        if let Ok(mut log) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(PANIC_LOG_PATH)
        {
            let _ = writeln!(log, "{:?}: {info}", SystemTime::now());
        }

        let _ = writeln!(
            stderr,
            "\nbackground worker panicked; terminal restored; see {PANIC_LOG_PATH}"
        );
        default_hook(info);
    }));
}
