//! TURN service through a real node: a `turn`-profile node runs the built-in
//! TURN server and credential broker; a libp2p client obtains credentials
//! over `/p2p-net/turn-credentials/1.0.0` and a real TURN client allocates a
//! relay with them. The relay must forward to public peers only, unless the
//! operator explicitly allows private ones.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use libp2p::request_response::{self, json, ProtocolSupport};
use libp2p::swarm::SwarmEvent;
use libp2p::{noise, tcp, yamux, Multiaddr, StreamProtocol, Swarm};
use p2p_net::{
    start_node, CredentialRequest, CredentialResponse, DhtDiscoveryConfig, DiscoveryConfig,
    NodeConfig, NodeHandle, NodeProfile, PublicBootstrapConfig, PublicIpProbeConfig,
    RendezvousConfig, TurnCredentials, TurnServerState, TURN_CREDENTIALS_PROTOCOL,
};
use tokio::net::UdpSocket;
use turn::client::{Client, ClientConfig};
use webrtc_util::Conn;

const STEP: Duration = Duration::from_secs(30);
const NO_DELIVERY_WINDOW: Duration = Duration::from_secs(2);

type CredentialClient = json::Behaviour<CredentialRequest, CredentialResponse>;

#[tokio::test]
async fn broker_credentials_allocate_a_working_relay() {
    let (node, turn_port) = start_turn_node("turn-relay", true).await;
    let credentials = fetch_credentials(&node).await;
    assert!(credentials.has_turn());
    assert!(credentials.ice_servers[0]
        .urls
        .contains(&format!("turn:127.0.0.1:{turn_port}?transport=udp")));

    let echo = UdpSocket::bind("127.0.0.1:0").await.expect("echo socket");
    let relay = allocate(&credentials, turn_port).await;
    relay
        .send_to(b"through-turn", echo.local_addr().expect("echo addr"))
        .await
        .expect("send through relay");
    let mut buf = [0u8; 64];
    let (len, from) = tokio::time::timeout(STEP, echo.recv_from(&mut buf))
        .await
        .expect("relayed datagram arrives")
        .expect("recv");
    assert_eq!(&buf[..len], b"through-turn");
    assert_eq!(from, relay.local_addr().expect("relay addr"));

    let snapshot = node.snapshot.lock().await.turn.clone();
    assert_eq!(snapshot.server_state, TurnServerState::Running);
    assert!(snapshot.broker_granted >= 1);
    assert_eq!(snapshot.broker_sources, vec!["shared_secret".to_string()]);
    node.shutdown().await;
}

#[tokio::test]
async fn relay_refuses_private_peers_by_default() {
    let (node, turn_port) = start_turn_node("turn-private", false).await;
    let credentials = fetch_credentials(&node).await;
    let echo = UdpSocket::bind("127.0.0.1:0").await.expect("echo socket");
    let relay = allocate(&credentials, turn_port).await;
    // The send itself may succeed locally; the server must not forward it.
    let _ = relay
        .send_to(b"must-not-arrive", echo.local_addr().expect("echo addr"))
        .await;
    let mut buf = [0u8; 64];
    assert!(
        tokio::time::timeout(NO_DELIVERY_WINDOW, echo.recv_from(&mut buf))
            .await
            .is_err(),
        "TURN relayed a datagram to a loopback peer"
    );
    node.shutdown().await;
}

async fn start_turn_node(label: &str, allow_private_peers: bool) -> (NodeHandle, u16) {
    let turn_port = free_udp_port();
    let relay_port = free_udp_port();
    let mut cfg = node_config(label);
    cfg.profile = NodeProfile::Turn;
    cfg.turn.server.bind_ip = "127.0.0.1".to_string();
    cfg.turn.server.public_ip = Some("127.0.0.1".to_string());
    cfg.turn.server.listen_port = turn_port;
    cfg.turn.server.relay_port_min = relay_port;
    cfg.turn.server.relay_port_max = relay_port;
    cfg.turn.server.allow_private_peers = allow_private_peers;
    let node = start_node(cfg).await.expect("start TURN node");
    (node, turn_port)
}

/// Ask the node's broker until its credential cache is ready.
async fn fetch_credentials(node: &NodeHandle) -> TurnCredentials {
    let addr = wait_for_tcp_dial_addr(node).await;
    let mut swarm = credential_client();
    swarm.dial(addr).expect("dial TURN node");
    tokio::time::timeout(STEP, async {
        loop {
            match swarm.select_next_some().await {
                SwarmEvent::ConnectionEstablished { .. } => {
                    swarm.behaviour_mut().send_request(&node.peer_id, request());
                }
                SwarmEvent::Behaviour(request_response::Event::Message {
                    message: request_response::Message::Response { response, .. },
                    ..
                }) => match response {
                    CredentialResponse::Granted(credentials) => return credentials,
                    CredentialResponse::Unavailable | CredentialResponse::RateLimited => {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        swarm.behaviour_mut().send_request(&node.peer_id, request());
                    }
                },
                SwarmEvent::Behaviour(request_response::Event::OutboundFailure {
                    error, ..
                }) => panic!("credential request failed: {error}"),
                _ => {}
            }
        }
    })
    .await
    .expect("broker grants credentials")
}

fn request() -> CredentialRequest {
    CredentialRequest { ttl_secs: 600 }
}

fn credential_client() -> Swarm<CredentialClient> {
    libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .expect("tcp transport")
        .with_behaviour(|_| {
            CredentialClient::new(
                [(
                    StreamProtocol::new(TURN_CREDENTIALS_PROTOCOL),
                    ProtocolSupport::Outbound,
                )],
                request_response::Config::default(),
            )
        })
        .expect("behaviour")
        .build()
}

async fn allocate(credentials: &TurnCredentials, turn_port: u16) -> impl Conn {
    let server = &credentials.ice_servers[0];
    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("client socket");
    let client = Client::new(ClientConfig {
        stun_serv_addr: String::new(),
        turn_serv_addr: format!("127.0.0.1:{turn_port}"),
        username: server.username.clone().expect("username"),
        password: server.credential.clone().expect("credential"),
        realm: "p2p-net".to_string(),
        software: String::new(),
        rto_in_ms: 0,
        conn: Arc::new(socket),
        vnet: None,
    })
    .await
    .expect("TURN client");
    client.listen().await.expect("TURN client listen");
    tokio::time::timeout(STEP, client.allocate())
        .await
        .expect("allocation in time")
        .expect("allocation granted")
}

async fn wait_for_tcp_dial_addr(handle: &NodeHandle) -> Multiaddr {
    tokio::time::timeout(STEP, async {
        loop {
            let snapshot = handle.snapshot.lock().await;
            if let Some(addr) = snapshot
                .local_listen_addresses
                .iter()
                .find(|addr| addr.contains("/tcp/"))
            {
                return format!("{addr}/p2p/{}", handle.peer_id)
                    .parse()
                    .expect("dial multiaddr");
            }
            drop(snapshot);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("TURN node TCP listen address")
}

fn free_udp_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .and_then(|socket| socket.local_addr())
        .map(|addr| addr.port())
        .expect("free UDP port")
}

fn node_config(label: &str) -> NodeConfig {
    let temp = |suffix: &str| {
        std::env::temp_dir()
            .join(format!(
                "p2p-net-{label}-{}.{suffix}",
                libp2p::PeerId::random()
            ))
            .to_string_lossy()
            .to_string()
    };
    NodeConfig {
        identity_key_path: temp("key"),
        webrtc_certificate_path: temp("webrtc-cert.pem"),
        listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".to_string()],
        discovery: DiscoveryConfig {
            peer_cache_path: temp("cache.json"),
            public_bootstrap: PublicBootstrapConfig::private_infrastructure_only(),
            rendezvous: RendezvousConfig {
                client_enabled: false,
                server_enabled: false,
                ..RendezvousConfig::default()
            },
            dht: DhtDiscoveryConfig {
                enabled: false,
                announce: false,
                discover: false,
                ..DhtDiscoveryConfig::default()
            },
            ..DiscoveryConfig::default()
        },
        public_ip_probe: PublicIpProbeConfig {
            enabled: false,
            ..PublicIpProbeConfig::default()
        },
        ..NodeConfig::default()
    }
}
