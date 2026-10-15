//! Built-in TURN server (RFC 5766 over UDP) for nodes running the `turn`
//! profile or `turn.server.enabled`.
//!
//! Authentication is the TURN REST API shared-secret scheme, so the node's
//! own broker (and any coturn-style tooling holding the secret) mints
//! credentials. Relay sockets come from a bounded port range, which also caps
//! concurrent allocations, and refuse to send to private, loopback or
//! link-local addresses unless `allow_private_peers` is set.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use async_trait::async_trait;
use turn::auth::LongTermAuthHandler;
use turn::relay::relay_range::RelayAddressGeneratorRanges;
use turn::relay::RelayAddressGenerator;
use turn::server::config::{ConnConfig, ServerConfig};
use turn::server::Server;
use webrtc_util::vnet::net::Net;
use webrtc_util::Conn;

use crate::connectivity::addr::is_public_ip;

use super::config::TurnServerConfig;
use super::provider::SharedSecretProviderConfig;
use super::secret::SecretRef;

pub(crate) struct RunningTurnServer {
    server: Server,
    listen: SocketAddr,
    credentials: SharedSecretProviderConfig,
}

impl RunningTurnServer {
    /// Bind the UDP listener and start serving. `relay_ip` is the address
    /// clients are told to send relayed traffic to.
    pub(crate) async fn start(cfg: &TurnServerConfig, relay_ip: IpAddr) -> Result<Self, String> {
        let secret = match &cfg.shared_secret {
            Some(secret) => secret.resolve()?,
            None => random_secret(),
        };
        let bind_ip: IpAddr = cfg
            .bind_ip
            .parse()
            .map_err(|_| format!("invalid turn.server.bind_ip {}", cfg.bind_ip))?;
        let socket = tokio::net::UdpSocket::bind((bind_ip, cfg.listen_port))
            .await
            .map_err(|err| format!("cannot bind TURN UDP {bind_ip}:{}: {err}", cfg.listen_port))?;
        let listen = socket.local_addr().map_err(|err| err.to_string())?;
        let relay_addr_generator = Box::new(PeerFilteringRelayGenerator {
            inner: RelayAddressGeneratorRanges {
                relay_address: relay_ip,
                min_port: cfg.relay_port_min,
                max_port: cfg.relay_port_max,
                max_retries: 0,
                address: bind_ip.to_string(),
                net: Arc::new(Net::new(None)),
            },
            allow_private_peers: cfg.allow_private_peers,
        });
        let server = Server::new(ServerConfig {
            conn_configs: vec![ConnConfig {
                conn: Arc::new(socket),
                relay_addr_generator,
            }],
            realm: cfg.realm.clone(),
            auth_handler: Arc::new(LongTermAuthHandler::new(secret.clone())),
            channel_bind_timeout: std::time::Duration::ZERO,
            alloc_close_notify: None,
        })
        .await
        .map_err(|err| format!("cannot start TURN server: {err}"))?;
        let host = advertised_host(cfg, relay_ip);
        let port = listen.port();
        Ok(Self {
            server,
            listen,
            credentials: SharedSecretProviderConfig {
                urls: vec![
                    format!("stun:{host}:{port}"),
                    format!("turn:{host}:{port}?transport=udp"),
                ],
                secret: SecretRef::Value(secret),
            },
        })
    }

    pub(crate) fn listen_addr(&self) -> SocketAddr {
        self.listen
    }

    /// Credential source for this node's broker.
    pub(crate) fn credential_provider(&self) -> SharedSecretProviderConfig {
        self.credentials.clone()
    }

    pub(crate) async fn allocation_count(&self) -> usize {
        self.server
            .get_allocations_info(None)
            .await
            .map(|allocations| allocations.len())
            .unwrap_or(0)
    }

    pub(crate) async fn close(self) {
        let _ = self.server.close().await;
    }
}

fn advertised_host(cfg: &TurnServerConfig, relay_ip: IpAddr) -> String {
    match (&cfg.public_host, relay_ip) {
        (Some(host), _) => host.clone(),
        (None, IpAddr::V6(ip)) => format!("[{ip}]"),
        (None, IpAddr::V4(ip)) => ip.to_string(),
    }
}

fn random_secret() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Relay generator whose sockets drop sends to non-public peers.
struct PeerFilteringRelayGenerator {
    inner: RelayAddressGeneratorRanges,
    allow_private_peers: bool,
}

#[async_trait]
impl RelayAddressGenerator for PeerFilteringRelayGenerator {
    fn validate(&self) -> Result<(), turn::Error> {
        self.inner.validate()
    }

    async fn allocate_conn(
        &self,
        use_ipv4: bool,
        requested_port: u16,
    ) -> Result<(Arc<dyn Conn + Send + Sync>, SocketAddr), turn::Error> {
        let (conn, relay_addr) = self.inner.allocate_conn(use_ipv4, requested_port).await?;
        if self.allow_private_peers {
            return Ok((conn, relay_addr));
        }
        Ok((Arc::new(PublicPeersOnly { inner: conn }), relay_addr))
    }
}

struct PublicPeersOnly {
    inner: Arc<dyn Conn + Send + Sync>,
}

#[async_trait]
impl Conn for PublicPeersOnly {
    async fn connect(&self, addr: SocketAddr) -> webrtc_util::Result<()> {
        ensure_public(addr)?;
        self.inner.connect(addr).await
    }

    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        self.inner.recv(buf).await
    }

    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        self.inner.recv_from(buf).await
    }

    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        self.inner.send(buf).await
    }

    async fn send_to(&self, buf: &[u8], target: SocketAddr) -> webrtc_util::Result<usize> {
        ensure_public(target)?;
        self.inner.send_to(buf, target).await
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        self.inner.local_addr()
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        self.inner.remote_addr()
    }

    async fn close(&self) -> webrtc_util::Result<()> {
        self.inner.close().await
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

fn ensure_public(addr: SocketAddr) -> webrtc_util::Result<()> {
    if is_public_ip(addr.ip()) {
        Ok(())
    } else {
        Err(webrtc_util::Error::Other(format!(
            "TURN relay to non-public address {addr} refused"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loopback_config() -> TurnServerConfig {
        TurnServerConfig {
            enabled: true,
            bind_ip: "127.0.0.1".to_string(),
            listen_port: free_udp_port(),
            relay_port_min: 50100,
            relay_port_max: 50120,
            ..TurnServerConfig::default()
        }
    }

    fn free_udp_port() -> u16 {
        std::net::UdpSocket::bind("127.0.0.1:0")
            .and_then(|socket| socket.local_addr())
            .map(|addr| addr.port())
            .expect("free UDP port")
    }

    #[tokio::test]
    async fn starts_and_advertises_shared_secret_credentials() {
        let cfg = loopback_config();
        let server = RunningTurnServer::start(&cfg, "198.51.100.7".parse().unwrap())
            .await
            .expect("server starts");
        assert_eq!(server.listen_addr().port(), cfg.listen_port);
        let provider = server.credential_provider();
        assert_eq!(
            provider.urls,
            vec![
                format!("stun:198.51.100.7:{}", cfg.listen_port),
                format!("turn:198.51.100.7:{}?transport=udp", cfg.listen_port),
            ]
        );
        assert_eq!(server.allocation_count().await, 0);
        server.close().await;
    }

    #[tokio::test]
    async fn public_host_and_ipv6_shape_advertised_urls() {
        let mut cfg = loopback_config();
        cfg.public_host = Some("turn.example.org".to_string());
        assert_eq!(
            advertised_host(&cfg, "2001:db8::1".parse().unwrap()),
            "turn.example.org"
        );
        cfg.public_host = None;
        assert_eq!(
            advertised_host(&cfg, "2001:db8::1".parse().unwrap()),
            "[2001:db8::1]"
        );
    }

    #[test]
    fn relay_refuses_private_targets() {
        assert!(ensure_public("10.0.0.5:80".parse().unwrap()).is_err());
        assert!(ensure_public("127.0.0.1:22".parse().unwrap()).is_err());
        assert!(ensure_public("[fe80::1]:22".parse().unwrap()).is_err());
        assert!(ensure_public("8.8.8.8:53".parse().unwrap()).is_ok());
    }
}
