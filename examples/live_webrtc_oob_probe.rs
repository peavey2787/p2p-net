//! Live WebRTC out-of-band signaling probe.
//!
//! This is the browser-facing next step after the libp2p DCUtR probe: exchange
//! one human-copyable offer string and one human-copyable answer string, then
//! let WebRTC ICE attempt a direct data-channel connection for up to 60s.
//!
//! Two-terminal example:
//!
//! ```text
//! cargo run --example live_webrtc_oob_probe -- --role answer --exchange-dir .live-webrtc --session demo
//! cargo run --example live_webrtc_oob_probe -- --role offer --exchange-dir .live-webrtc --session demo
//! ```
//!
//! The automation writes the strings to files under `--exchange-dir`, but those
//! file contents are exactly what a human could copy/paste between machines.

use std::env;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::time::{sleep, Instant};
use webrtc::api::{setting_engine::SettingEngine, APIBuilder};
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

const OFFER_PREFIX: &str = "P2PNET-WEBRTC-OFFER-V1:";
const ANSWER_PREFIX: &str = "P2PNET-WEBRTC-ANSWER-V1:";
const CHANNEL_LABEL: &str = "p2p-net-webrtc-oob";
const DEFAULT_STUN: &str = "stun:stun.l.google.com:19302";
const PING_PREFIX: &str = "P2PNET_WEBRTC_PING:";
const PONG_PREFIX: &str = "P2PNET_WEBRTC_PONG:";
const CANDIDATE_PREFIX: &str = "a=candidate:";

type AnyError = Box<dyn std::error::Error + Send + Sync + 'static>;
type AnyResult<T> = Result<T, AnyError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Offer,
    Answer,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Self::Offer => "offer",
            Self::Answer => "answer",
        }
    }

    fn signal_prefix(self) -> &'static str {
        match self {
            Self::Offer => OFFER_PREFIX,
            Self::Answer => ANSWER_PREFIX,
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidatePolicy {
    All,
    SrflxOnly,
    HostOnly,
}

impl CandidatePolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::SrflxOnly => "srflx-only",
            Self::HostOnly => "host-only",
        }
    }

    fn allows(self, line: &str) -> bool {
        match self {
            Self::All => true,
            Self::SrflxOnly => line.contains(" typ srflx "),
            Self::HostOnly => line.contains(" typ host "),
        }
    }
}

impl fmt::Display for CandidatePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
struct Cli {
    role: Role,
    exchange_dir: PathBuf,
    session: String,
    timeout: Duration,
    stun: Option<String>,
    candidate_policy: CandidatePolicy,
}

#[derive(Debug, Serialize, Deserialize)]
struct HumanSignal {
    version: u8,
    session: String,
    role: String,
    created_unix_ms: u128,
    description: RTCSessionDescription,
}

#[derive(Debug)]
enum Event {
    DataChannelOpen,
    PingReceived,
    PongReceived,
    IceState(RTCIceConnectionState),
    PeerState(RTCPeerConnectionState),
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let start = Instant::now();
    let result = async {
        let cli = parse_cli()?;
        let session_dir = cli.exchange_dir.join(&cli.session);
        tokio::fs::create_dir_all(&session_dir).await?;
        println!(
            "webrtc_oob_start role={} session={} exchange_dir={} timeout_secs={} stun={} candidate_policy={}",
            cli.role,
            cli.session,
            session_dir.display(),
            cli.timeout.as_secs(),
            cli.stun.as_deref().unwrap_or("disabled"),
            cli.candidate_policy,
        );

        match cli.role {
            Role::Offer => run_offer(cli, session_dir, start).await,
            Role::Answer => run_answer(cli, session_dir, start).await,
        }
    }
    .await;

    if let Err(err) = result {
        eprintln!("webrtc_oob_failed error={err}");
        std::process::exit(1);
    }
}

async fn run_offer(cli: Cli, session_dir: PathBuf, start: Instant) -> AnyResult<()> {
    let deadline = start + cli.timeout;
    let peer = Arc::new(build_peer_connection(&cli).await?);
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    install_peer_handlers(&peer, event_tx.clone());

    let data_channel = peer.create_data_channel(CHANNEL_LABEL, None).await?;
    install_data_channel_handlers(Role::Offer, data_channel, event_tx);

    let offer = peer.create_offer(None).await?;
    let mut gather_complete = peer.gathering_complete_promise().await;
    peer.set_local_description(offer).await?;
    wait_for_gathering(&mut gather_complete, deadline).await?;

    let offer_desc = peer
        .local_description()
        .await
        .ok_or_else(|| err("offerer has no local description after ICE gathering"))?;
    let offer_desc = filter_description_candidates(offer_desc, cli.candidate_policy)?;
    let offer_string = encode_signal(Role::Offer, &cli.session, offer_desc)?;
    write_signal(&session_dir.join("offer.txt"), &offer_string).await?;
    println!("webrtc_oob_offer_string {offer_string}");

    let answer_string = wait_for_file(&session_dir.join("answer.txt"), deadline).await?;
    let answer_desc = decode_signal(Role::Answer, &cli.session, &answer_string)?;
    println!(
        "webrtc_oob_answer_string_received bytes={}",
        answer_string.len()
    );
    peer.set_remote_description(answer_desc).await?;

    wait_for_success(Role::Offer, &mut event_rx, deadline).await?;
    write_success(
        &session_dir,
        Role::Offer,
        start.elapsed(),
        "pong_received",
        cli.candidate_policy,
    )
    .await?;
    peer.close().await?;
    Ok(())
}

async fn run_answer(cli: Cli, session_dir: PathBuf, start: Instant) -> AnyResult<()> {
    let deadline = start + cli.timeout;
    let peer = Arc::new(build_peer_connection(&cli).await?);
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    install_peer_handlers(&peer, event_tx.clone());

    peer.on_data_channel(Box::new(move |data_channel: Arc<RTCDataChannel>| {
        let event_tx = event_tx.clone();
        Box::pin(async move {
            println!(
                "webrtc_oob_remote_data_channel label={}",
                data_channel.label()
            );
            install_data_channel_handlers(Role::Answer, data_channel, event_tx);
        })
    }));

    let offer_string = wait_for_file(&session_dir.join("offer.txt"), deadline).await?;
    let offer_desc = decode_signal(Role::Offer, &cli.session, &offer_string)?;
    println!(
        "webrtc_oob_offer_string_received bytes={}",
        offer_string.len()
    );
    peer.set_remote_description(offer_desc).await?;

    let answer = peer.create_answer(None).await?;
    let mut gather_complete = peer.gathering_complete_promise().await;
    peer.set_local_description(answer).await?;
    wait_for_gathering(&mut gather_complete, deadline).await?;

    let answer_desc = peer
        .local_description()
        .await
        .ok_or_else(|| err("answerer has no local description after ICE gathering"))?;
    let answer_desc = filter_description_candidates(answer_desc, cli.candidate_policy)?;
    let answer_string = encode_signal(Role::Answer, &cli.session, answer_desc)?;
    write_signal(&session_dir.join("answer.txt"), &answer_string).await?;
    println!("webrtc_oob_answer_string {answer_string}");

    wait_for_success(Role::Answer, &mut event_rx, deadline).await?;
    write_success(
        &session_dir,
        Role::Answer,
        start.elapsed(),
        "ping_received_and_pong_sent",
        cli.candidate_policy,
    )
    .await?;
    peer.close().await?;
    Ok(())
}

async fn build_peer_connection(cli: &Cli) -> AnyResult<webrtc::peer_connection::RTCPeerConnection> {
    let ice_servers = cli
        .stun
        .as_ref()
        .map(|stun| {
            vec![RTCIceServer {
                urls: vec![stun.clone()],
                ..Default::default()
            }]
        })
        .unwrap_or_default();

    let mut setting_engine = SettingEngine::default();
    setting_engine.set_srtp_protection_profiles(vec![
        SrtpProtectionProfile::Srtp_Aead_Aes_128_Gcm,
        SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80,
        SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_32,
    ]);
    let api = APIBuilder::new()
        .with_setting_engine(setting_engine)
        .build();
    let config = RTCConfiguration {
        ice_servers,
        ..Default::default()
    };

    Ok(api.new_peer_connection(config).await?)
}

fn install_peer_handlers(
    peer: &Arc<webrtc::peer_connection::RTCPeerConnection>,
    event_tx: mpsc::UnboundedSender<Event>,
) {
    let ice_tx = event_tx.clone();
    peer.on_ice_connection_state_change(Box::new(move |state| {
        let ice_tx = ice_tx.clone();
        Box::pin(async move {
            println!("webrtc_oob_ice_state state={state}");
            let _ = ice_tx.send(Event::IceState(state));
        })
    }));

    peer.on_peer_connection_state_change(Box::new(move |state| {
        let event_tx = event_tx.clone();
        Box::pin(async move {
            println!("webrtc_oob_peer_state state={state}");
            let _ = event_tx.send(Event::PeerState(state));
        })
    }));
}

fn install_data_channel_handlers(
    role: Role,
    data_channel: Arc<RTCDataChannel>,
    event_tx: mpsc::UnboundedSender<Event>,
) {
    let open_tx = event_tx.clone();
    let open_channel = data_channel.clone();
    data_channel.on_open(Box::new(move || {
        let open_tx = open_tx.clone();
        let open_channel = open_channel.clone();
        Box::pin(async move {
            println!("webrtc_oob_data_channel_open role={role}");
            let _ = open_tx.send(Event::DataChannelOpen);
            if role == Role::Offer {
                let ping = format!("{PING_PREFIX}{}", now_unix_ms());
                if let Err(err) = open_channel.send_text(ping).await {
                    eprintln!("webrtc_oob_ping_send_failed error={err}");
                }
            }
        })
    }));

    let message_tx = event_tx;
    let message_channel = data_channel.clone();
    data_channel.on_message(Box::new(move |message: DataChannelMessage| {
        let message_tx = message_tx.clone();
        let message_channel = message_channel.clone();
        Box::pin(async move {
            let text = String::from_utf8_lossy(&message.data);
            println!("webrtc_oob_data_channel_message role={role} text={text}");
            if role == Role::Answer && text.starts_with(PING_PREFIX) {
                let pong = format!("{PONG_PREFIX}{}", now_unix_ms());
                match message_channel.send_text(pong).await {
                    Ok(_) => {
                        let _ = message_tx.send(Event::PingReceived);
                    }
                    Err(err) => eprintln!("webrtc_oob_pong_send_failed error={err}"),
                }
            } else if role == Role::Offer && text.starts_with(PONG_PREFIX) {
                let _ = message_tx.send(Event::PongReceived);
            }
        })
    }));
}

async fn wait_for_gathering(
    gather_complete: &mut tokio::sync::mpsc::Receiver<()>,
    deadline: Instant,
) -> AnyResult<()> {
    let remaining = time_left(deadline)?;
    tokio::time::timeout(remaining, gather_complete.recv())
        .await
        .map_err(|_| err("timed out waiting for WebRTC ICE gathering to complete"))?;
    Ok(())
}

async fn wait_for_success(
    role: Role,
    event_rx: &mut mpsc::UnboundedReceiver<Event>,
    deadline: Instant,
) -> AnyResult<()> {
    loop {
        let remaining = time_left(deadline)?;
        let event = tokio::time::timeout(remaining, event_rx.recv())
            .await
            .map_err(|_| err("timed out waiting for WebRTC data-channel ping/pong"))?
            .ok_or_else(|| err("event channel closed before WebRTC success"))?;

        match event {
            Event::PongReceived if role == Role::Offer => {
                println!("webrtc_oob_connected role={role} evidence=pong_received");
                return Ok(());
            }
            Event::PingReceived if role == Role::Answer => {
                println!("webrtc_oob_connected role={role} evidence=ping_received_and_pong_sent");
                return Ok(());
            }
            Event::PeerState(RTCPeerConnectionState::Failed) => {
                return Err(err("WebRTC peer connection entered failed state").into());
            }
            Event::PeerState(RTCPeerConnectionState::Closed) => {
                return Err(err("WebRTC peer connection closed before success").into());
            }
            Event::IceState(RTCIceConnectionState::Failed) => {
                return Err(err("WebRTC ICE connection entered failed state").into());
            }
            Event::IceState(RTCIceConnectionState::Disconnected) => {
                println!("webrtc_oob_ice_disconnected_waiting_for_recovery");
            }
            _ => {}
        }
    }
}

fn encode_signal(
    role: Role,
    session: &str,
    description: RTCSessionDescription,
) -> AnyResult<String> {
    let signal = HumanSignal {
        version: 1,
        session: session.to_owned(),
        role: role.as_str().to_owned(),
        created_unix_ms: now_unix_ms(),
        description,
    };
    let json = serde_json::to_vec(&signal)?;
    Ok(format!(
        "{}{}",
        role.signal_prefix(),
        URL_SAFE_NO_PAD.encode(json)
    ))
}

fn decode_signal(role: Role, session: &str, value: &str) -> AnyResult<RTCSessionDescription> {
    let trimmed = value.trim();
    let payload = trimmed
        .strip_prefix(role.signal_prefix())
        .ok_or_else(|| err(format!("expected {} signal prefix", role.signal_prefix())))?;
    let json = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|e| err(format!("invalid base64 signal payload: {e}")))?;
    let signal: HumanSignal = serde_json::from_slice(&json)?;
    if signal.version != 1 {
        return Err(err(format!(
            "unsupported WebRTC OOB signal version {}",
            signal.version
        ))
        .into());
    }
    if signal.session != session {
        return Err(err(format!(
            "signal session mismatch: expected {session}, got {}",
            signal.session
        ))
        .into());
    }
    if signal.role != role.as_str() {
        return Err(err(format!(
            "signal role mismatch: expected {role}, got {}",
            signal.role
        ))
        .into());
    }
    Ok(signal.description)
}

fn filter_description_candidates(
    mut description: RTCSessionDescription,
    policy: CandidatePolicy,
) -> AnyResult<RTCSessionDescription> {
    if policy == CandidatePolicy::All {
        return Ok(description);
    }

    let had_trailing_crlf = description.sdp.ends_with("\r\n");
    let mut kept_candidates = 0usize;
    let mut dropped_candidates = 0usize;
    let mut lines = Vec::new();
    for line in description.sdp.trim_end_matches("\r\n").split("\r\n") {
        if line.starts_with(CANDIDATE_PREFIX) {
            if policy.allows(line) {
                kept_candidates += 1;
            } else {
                dropped_candidates += 1;
                continue;
            }
        }
        lines.push(line);
    }

    if kept_candidates == 0 {
        return Err(err(format!(
            "candidate policy {policy} removed every ICE candidate"
        ))
        .into());
    }

    description.sdp = lines.join("\r\n");
    if had_trailing_crlf {
        description.sdp.push_str("\r\n");
    }
    println!(
        "webrtc_oob_candidate_policy policy={policy} kept={kept_candidates} dropped={dropped_candidates}"
    );
    Ok(description)
}

async fn write_signal(path: &Path, value: &str) -> AnyResult<()> {
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, format!("{value}\n")).await?;
    tokio::fs::rename(tmp, path).await?;
    Ok(())
}

async fn wait_for_file(path: &Path, deadline: Instant) -> AnyResult<String> {
    loop {
        match tokio::fs::read_to_string(path).await {
            Ok(value) if !value.trim().is_empty() => return Ok(value),
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }

        let remaining = time_left(deadline)?;
        sleep(remaining.min(Duration::from_millis(200))).await;
    }
}

async fn write_success(
    session_dir: &Path,
    role: Role,
    elapsed: Duration,
    evidence: &str,
    candidate_policy: CandidatePolicy,
) -> AnyResult<()> {
    let summary = serde_json::json!({
        "ok": true,
        "role": role.as_str(),
        "elapsed_ms": elapsed.as_millis(),
        "evidence": evidence,
        "relay": "none-configured",
        "stun_only": true,
        "candidate_policy": candidate_policy.as_str(),
        "data_channel": CHANNEL_LABEL,
    });
    let path = session_dir.join(format!("success_{}.json", role.as_str()));
    tokio::fs::write(&path, serde_json::to_vec_pretty(&summary)?).await?;
    println!("webrtc_oob_success {summary}");
    Ok(())
}

fn parse_cli() -> AnyResult<Cli> {
    let mut role = None;
    let mut exchange_dir = PathBuf::from(".live-webrtc-oob");
    let mut session = format!("session-{}", now_unix_ms());
    let mut timeout = Duration::from_secs(60);
    let mut stun = Some(DEFAULT_STUN.to_owned());
    let mut candidate_policy = CandidatePolicy::All;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--role" => {
                let value = args
                    .next()
                    .ok_or_else(|| err("--role requires offer or answer"))?;
                role = Some(match value.as_str() {
                    "offer" => Role::Offer,
                    "answer" => Role::Answer,
                    _ => return Err(err("--role must be offer or answer").into()),
                });
            }
            "--exchange-dir" => {
                exchange_dir = PathBuf::from(
                    args.next()
                        .ok_or_else(|| err("--exchange-dir requires a path"))?,
                );
            }
            "--session" => {
                session = args
                    .next()
                    .ok_or_else(|| err("--session requires a value"))?;
            }
            "--timeout-secs" => {
                let value = args
                    .next()
                    .ok_or_else(|| err("--timeout-secs requires a number"))?;
                let secs = value
                    .parse::<u64>()
                    .map_err(|e| err(format!("invalid --timeout-secs value {value:?}: {e}")))?;
                timeout = Duration::from_secs(secs);
            }
            "--stun" => {
                stun = Some(args.next().ok_or_else(|| err("--stun requires a URI"))?);
            }
            "--no-stun" => {
                stun = None;
            }
            "--candidate-policy" => {
                let value = args.next().ok_or_else(|| {
                    err("--candidate-policy requires all, srflx-only, or host-only")
                })?;
                candidate_policy = match value.as_str() {
                    "all" => CandidatePolicy::All,
                    "srflx-only" => CandidatePolicy::SrflxOnly,
                    "host-only" => CandidatePolicy::HostOnly,
                    _ => {
                        return Err(
                            err("--candidate-policy must be all, srflx-only, or host-only").into(),
                        )
                    }
                };
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(err(format!("unknown argument {other:?}; use --help")).into()),
        }
    }

    Ok(Cli {
        role: role.ok_or_else(|| err("--role offer|answer is required"))?,
        exchange_dir,
        session,
        timeout,
        stun,
        candidate_policy,
    })
}

fn print_help() {
    println!(
        "\
live_webrtc_oob_probe

Required:
  --role offer|answer

Options:
  --exchange-dir PATH     Directory used to exchange human strings [default: .live-webrtc-oob]
  --session NAME          Session namespace [default: generated]
  --timeout-secs SECS     End-to-end success limit [default: 60]
  --stun URI              STUN server URI [default: stun:stun.l.google.com:19302]
  --no-stun               Disable STUN and use host candidates only
  --candidate-policy MODE Candidate filter: all, srflx-only, host-only [default: all]

Files:
  offer.txt               P2PNET-WEBRTC-OFFER-V1:<base64-json>
  answer.txt              P2PNET-WEBRTC-ANSWER-V1:<base64-json>
  success_offer.json      Written only after offerer receives pong
  success_answer.json     Written only after answerer receives ping and sends pong
"
    );
}

fn time_left(deadline: Instant) -> AnyResult<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| err("60 second WebRTC OOB probe timeout elapsed").into())
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn err(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}
