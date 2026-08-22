//! Live standalone P2P node dashboard.
//!
//! Run:
//! `cargo run --release --features dashboard --example p2p_node`
//!
//! The example starts as a full-capability node by default: full Kademlia server,
//! normal protocol cadences, all configured inbound transports, relay/rendezvous
//! capabilities, and the production connection policy. Advanced users can copy
//! the generated config and selectively tune capabilities for their deployment.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use crossterm::event::{Event, EventStream, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use p2p_net::{start_node, NodeConfig, NodeProfile, NodeSnapshot};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Terminal;
use tokio::time::MissedTickBehavior;

static BACKGROUND_WORKER_PANICKED: AtomicBool = AtomicBool::new(false);
const PANIC_LOG_PATH: &str = "p2p-node-panic.log";
const DASHBOARD_SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

type DashboardTerminal = Terminal<CrosstermBackend<std::io::Stdout>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    install_dashboard_panic_hook();

    let args: Vec<String> = std::env::args().collect();
    if let Some(path) = arg_value(&args, "--write-default-config") {
        std::fs::write(path, full_node_default_config().to_pretty_json()?)?;
        println!("wrote full-node example config to {path}");
        return Ok(());
    }
    if let Some(path) = arg_value(&args, "--write-library-default-config") {
        std::fs::write(path, NodeConfig::default().to_pretty_json()?)?;
        println!("wrote library default config to {path}");
        return Ok(());
    }

    let cfg = if let Some(path) = arg_value(&args, "--config") {
        NodeConfig::load_json_file(path)?
    } else {
        full_node_default_config()
    };

    println!(
        "starting p2p_node profile={} tcp={} quic={} websocket={} webrtc_direct={} gossip={}s ping={}s dht_parallel={} dht_replicas={} max_connections={:?}",
        cfg.profile.as_str(),
        cfg.listeners.tcp,
        cfg.listeners.quic,
        cfg.listeners.websocket,
        cfg.listeners.webrtc_direct,
        cfg.gossipsub_heartbeat_interval_secs,
        cfg.ping_interval_secs,
        cfg.discovery.dht.query_parallelism,
        cfg.discovery.dht.provider_key_replicas,
        cfg.connection_limits.max_established
    );

    let handle = start_node(cfg).await?;
    let mut terminal = match setup_terminal() {
        Ok(terminal) => terminal,
        Err(err) => {
            handle.shutdown().await;
            return Err(err.into());
        }
    };

    let ui_result = run_ui(&mut terminal, &handle).await;
    // Stop network work first. On Windows, CTRL_CLOSE/LOGOFF/SHUTDOWN handlers
    // receive only a bounded cleanup window, so terminal cosmetics must never
    // delay swarm cancellation. The node handle itself has a one-second abort
    // fallback if graceful shutdown stalls.
    handle.shutdown().await;
    // Never use `?` before shutdown: the console may already be disappearing.
    let restore_result = restore_terminal(&mut terminal);

    if let Err(err) = restore_result {
        if ui_result.is_ok() {
            return Err(err.into());
        }
    }
    ui_result
}

fn full_node_default_config() -> NodeConfig {
    let mut cfg = NodeConfig::default();
    cfg.profile = NodeProfile::Full;
    cfg
}

async fn run_ui(
    terminal: &mut DashboardTerminal,
    handle: &p2p_net::NodeHandle,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut events = EventStream::new();
    let mut sample = tokio::time::interval(DASHBOARD_SAMPLE_INTERVAL);
    sample.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    let mut last_revision = None;

    loop {
        if BACKGROUND_WORKER_PANICKED.load(Ordering::Relaxed) {
            return Err(format!(
                "background worker panicked; terminal restored; see {PANIC_LOG_PATH}"
            )
            .into());
        }

        tokio::select! {
            signal = &mut shutdown => {
                signal?;
                break;
            }
            _ = sample.tick() => {
                render_if_changed(terminal, handle, &mut last_revision, false).await?;
            }
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key)))
                        if key.code == KeyCode::Char('q') || key.code == KeyCode::Esc => break,
                    Some(Ok(Event::Resize(_, _))) => {
                        render_if_changed(terminal, handle, &mut last_revision, true).await?;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => return Err(err.into()),
                    None => return Err("terminal event stream ended unexpectedly".into()),
                }
            }
        }
    }
    Ok(())
}

async fn render_if_changed(
    terminal: &mut DashboardTerminal,
    handle: &p2p_net::NodeHandle,
    last_revision: &mut Option<u64>,
    force: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let revision = handle.snapshot_revision();
    if !force && *last_revision == Some(revision) {
        return Ok(());
    }
    let snapshot = handle.snapshot.lock().await.clone();
    *last_revision = Some(revision);
    terminal.draw(|frame| draw_dashboard(frame, &snapshot))?;
    Ok(())
}

fn setup_terminal() -> io::Result<DashboardTerminal> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    if let Err(err) = execute!(stdout, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(err);
    }
    let backend = CrosstermBackend::new(stdout);
    match Terminal::new(backend) {
        Ok(terminal) => Ok(terminal),
        Err(err) => {
            let _ = disable_raw_mode();
            let mut stderr = io::stderr();
            let _ = execute!(stderr, LeaveAlternateScreen);
            Err(err)
        }
    }
}

fn restore_terminal(terminal: &mut DashboardTerminal) -> io::Result<()> {
    let raw_result = disable_raw_mode();
    let screen_result = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let cursor_result = terminal.show_cursor();
    raw_result?;
    screen_result?;
    cursor_result
}

#[cfg(windows)]
async fn shutdown_signal() -> io::Result<()> {
    use tokio::signal::windows::{ctrl_break, ctrl_c, ctrl_close, ctrl_logoff, ctrl_shutdown};

    let mut ctrl_c = ctrl_c()?;
    let mut ctrl_break = ctrl_break()?;
    let mut ctrl_close = ctrl_close()?;
    let mut ctrl_logoff = ctrl_logoff()?;
    let mut ctrl_shutdown = ctrl_shutdown()?;
    tokio::select! {
        _ = ctrl_c.recv() => {}
        _ = ctrl_break.recv() => {}
        _ = ctrl_close.recv() => {}
        _ = ctrl_logoff.recv() => {}
        _ = ctrl_shutdown.recv() => {}
    }
    Ok(())
}

#[cfg(unix)]
async fn shutdown_signal() -> io::Result<()> {
    use tokio::signal::unix::{signal, SignalKind};

    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        _ = terminate.recv() => {}
        _ = hangup.recv() => {}
    }
    Ok(())
}

#[cfg(not(any(windows, unix)))]
async fn shutdown_signal() -> io::Result<()> {
    tokio::signal::ctrl_c().await
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
    .block(Block::default().title("PeerID & Status").borders(Borders::ALL));
    frame.render_widget(status, chunks[0]);

    let transports = Paragraph::new(snap.active_transports.join(", ")).block(
        Block::default()
            .title("Transports Active")
            .borders(Borders::ALL),
    );
    frame.render_widget(transports, chunks[1]);

    let mesh = Paragraph::new(format!(
        "Application Peers: {} | Infrastructure Peers: {} | DHT Routing Peers: {} | Relay Peers: {} | All Swarm Connections: {}\nPeerBook: known {} discovered {}\nAuto-Connect: enabled={} dial_attempts={} failures={} pending_plans={} awaiting_addrs={}\nPublic Fallback: mode={} bootstrap={} rendezvous={} relay={} public_rv_candidates={} reason={}\nRelay Server: {} ({}) | Mediator: {}\nServer Reservations: {} | Client Reservations: {} / attempts {} failures {}\nRelay Discovery: selected {} / candidates {} failures={}\nDHT Provider: enabled={} announced={} queries={} found={} peers={}\nActive Circuits: {} | Denied Requests: {} | Bytes Fwd: {} | DCUtR: enabled={} attempts={} successes={} failures={} fallback={} suppressed={}",
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
        .map(|line| ListItem::new(line.as_str()))
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
