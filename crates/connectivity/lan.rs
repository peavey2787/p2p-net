//! Lightweight same-LAN discovery without the libp2p mDNS dependency.
//!
//! A small UDP beacon gives fresh nodes a fast path to one another on the same
//! LAN. Beacons are not trusted identity assertions: they are scoped by the
//! exact application compatibility protocol and every resulting libp2p
//! connection still has to authenticate through Noise + Identify before it is
//! promoted to an application peer.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol as SocketProtocol, Socket, Type};
use tokio::net::UdpSocket;

use crate::common::error::config_error;

pub const LAN_DISCOVERY_MULTICAST_V4: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 99);
pub const MAX_LAN_BEACON_BYTES: usize = 8192;
pub const MAX_LAN_ADVERTISED_ADDRS: usize = 16;
const LAN_BEACON_VERSION: u8 = 1;
#[cfg(any(target_os = "android", test))]
const ANDROID_EMULATOR_HOST_V4: Ipv4Addr = Ipv4Addr::new(10, 0, 2, 2);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LanDiscoveryConfig {
    /// Enable same-LAN zero-configuration discovery.
    pub enabled: bool,
    /// Shared UDP port used for multicast/broadcast beacons.
    pub port: u16,
    /// Seconds between announcements. Two seconds keeps local discovery well
    /// below the public-fallback timeout without creating meaningful traffic.
    pub announce_interval_secs: u64,
}

impl Default for LanDiscoveryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 44_777,
            announce_interval_secs: 2,
        }
    }
}

impl LanDiscoveryConfig {
    pub fn validate(&self) -> Result<(), crate::common::error::NetError> {
        if self.enabled && self.port == 0 {
            return Err(config_error("discovery.lan.port must be at least 1"));
        }
        if self.enabled && self.announce_interval_secs == 0 {
            return Err(config_error(
                "discovery.lan.announce_interval_secs must be at least 1",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LanBeacon {
    version: u8,
    network_id: u32,
    application_protocol: String,
    peer_id: String,
    addresses: Vec<String>,
    /// Official Android Emulator networking does not forward LAN multicast.
    /// An emulator therefore sends a compatibility-scoped unicast probe to
    /// its special host alias and asks the host node to answer once by unicast.
    #[serde(default)]
    reply_requested: bool,
}

#[derive(Debug, Clone)]
pub struct LanPeerAnnouncement {
    pub peer_id: PeerId,
    pub addresses: Vec<Multiaddr>,
}

#[derive(Debug, Clone)]
pub struct LanDiscoveryReceive {
    pub announcement: Option<LanPeerAnnouncement>,
    pub reply_to: Option<SocketAddr>,
}

pub struct LanDiscoverySocket {
    socket: UdpSocket,
    multicast_target: SocketAddr,
    broadcast_target: SocketAddr,
    emulator_host_target: Option<SocketAddr>,
}

impl LanDiscoverySocket {
    pub fn bind(cfg: &LanDiscoveryConfig) -> io::Result<Self> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(SocketProtocol::UDP))?;
        socket.set_reuse_address(true)?;
        // macOS needs SO_REUSEPORT for independent multicast listeners that
        // share the configured LAN discovery port. Without it, a second node
        // can start while never receiving the other node's discovery beacons.
        #[cfg(target_os = "macos")]
        socket.set_reuse_port(true)?;
        socket.set_broadcast(true)?;
        socket.set_multicast_loop_v4(true)?;
        socket.set_nonblocking(true)?;
        socket.bind(&SocketAddr::from(([0, 0, 0, 0], cfg.port)).into())?;
        socket.join_multicast_v4(&LAN_DISCOVERY_MULTICAST_V4, &Ipv4Addr::UNSPECIFIED)?;
        let socket = UdpSocket::from_std(socket.into())?;
        Ok(Self {
            socket,
            multicast_target: SocketAddr::new(IpAddr::V4(LAN_DISCOVERY_MULTICAST_V4), cfg.port),
            broadcast_target: SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), cfg.port),
            emulator_host_target: android_emulator_host_target(cfg.port),
        })
    }

    pub async fn announce(
        &self,
        network_id: u32,
        application_protocol: &str,
        peer_id: PeerId,
        addresses: impl IntoIterator<Item = Multiaddr>,
    ) -> io::Result<()> {
        let addresses = collect_advertised_addresses(addresses);
        // Discovery requests are useful even when this node has no inbound
        // listener (for example mobile-lite). A listenerless node can still
        // discover a reachable LAN peer and establish the outbound side.
        let payload = encode_beacon(network_id, application_protocol, peer_id, &addresses, false)?;

        // Multicast is the primary path. Broadcast is an intentional secondary
        // path for mobile/VM networks that filter IPv4 multicast.
        let multicast = self.socket.send_to(&payload, self.multicast_target).await;
        let broadcast = self.socket.send_to(&payload, self.broadcast_target).await;
        let mut any_success = multicast.is_ok() || broadcast.is_ok();
        let mut first_error = multicast.err().or_else(|| broadcast.err());

        if let Some(target) = self.emulator_host_target {
            // Use the same authenticated discovery payload, only with an
            // explicit one-shot reply request. The receiver replies to the UDP
            // source tuple, so the Android Emulator NAT can route the response
            // back to the guest without requiring the host to dial 10.0.2.15.
            let probe = encode_beacon(network_id, application_protocol, peer_id, &addresses, true)?;
            match self.socket.send_to(&probe, target).await {
                Ok(_) => any_success = true,
                Err(err) if first_error.is_none() => first_error = Some(err),
                Err(_) => {}
            }
        }

        if any_success {
            Ok(())
        } else {
            Err(first_error
                .unwrap_or_else(|| io::Error::other("LAN discovery announcement failed")))
        }
    }

    pub async fn respond(
        &self,
        target: SocketAddr,
        network_id: u32,
        application_protocol: &str,
        peer_id: PeerId,
        addresses: impl IntoIterator<Item = Multiaddr>,
    ) -> io::Result<()> {
        if !is_local_source(target.ip()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "LAN discovery reply target is not local",
            ));
        }
        let addresses = collect_advertised_addresses(addresses);
        if addresses.is_empty() {
            return Ok(());
        }
        let payload = encode_beacon(network_id, application_protocol, peer_id, &addresses, false)?;
        self.socket.send_to(&payload, target).await.map(|_| ())
    }

    pub async fn recv(
        &self,
        local_peer: PeerId,
        network_id: u32,
        application_protocol: &str,
    ) -> io::Result<Option<LanDiscoveryReceive>> {
        let mut buf = [0_u8; MAX_LAN_BEACON_BYTES];
        let (len, source) = self.socket.recv_from(&mut buf).await?;
        Ok(decode_beacon(
            &buf[..len],
            source,
            local_peer,
            network_id,
            application_protocol,
        ))
    }
}

fn decode_beacon(
    payload: &[u8],
    source: SocketAddr,
    local_peer: PeerId,
    network_id: u32,
    application_protocol: &str,
) -> Option<LanDiscoveryReceive> {
    if !is_local_source(source.ip()) {
        return None;
    }
    let beacon: LanBeacon = serde_json::from_slice(payload).ok()?;
    if beacon.version != LAN_BEACON_VERSION
        || beacon.network_id != network_id
        || beacon.application_protocol != application_protocol
    {
        return None;
    }
    let peer_id: PeerId = beacon.peer_id.parse().ok()?;
    if peer_id == local_peer {
        return None;
    }

    let reply_to = beacon.reply_requested.then_some(source);
    let mut addresses = Vec::new();
    if !beacon.reply_requested {
        for encoded in beacon.addresses.into_iter().take(MAX_LAN_ADVERTISED_ADDRS) {
            let Ok(addr) = encoded.parse::<Multiaddr>() else {
                continue;
            };
            if let Some(addr) = normalize_received_addr(addr, source.ip(), peer_id) {
                if !addresses.contains(&addr) {
                    addresses.push(addr);
                }
            }
        }
    }
    // Emulator probes are request-only. QEMU user-mode NAT may present a
    // host-side source address that is not dialable back to the guest. The
    // host answers the source tuple once; the guest then dials the host from
    // that authenticated compatibility-scoped reply.
    let announcement =
        (!addresses.is_empty()).then_some(LanPeerAnnouncement { peer_id, addresses });
    if announcement.is_none() && reply_to.is_none() {
        return None;
    }
    Some(LanDiscoveryReceive {
        announcement,
        reply_to,
    })
}

fn collect_advertised_addresses(addresses: impl IntoIterator<Item = Multiaddr>) -> Vec<String> {
    addresses
        .into_iter()
        .filter(is_lan_advertisable_addr)
        .take(MAX_LAN_ADVERTISED_ADDRS)
        .map(|addr| addr.to_string())
        .collect()
}

fn encode_beacon(
    network_id: u32,
    application_protocol: &str,
    peer_id: PeerId,
    addresses: &[String],
    reply_requested: bool,
) -> io::Result<Vec<u8>> {
    let payload = serde_json::to_vec(&LanBeacon {
        version: LAN_BEACON_VERSION,
        network_id,
        application_protocol: application_protocol.to_string(),
        peer_id: peer_id.to_string(),
        addresses: addresses.to_vec(),
        reply_requested,
    })
    .map_err(io::Error::other)?;
    if payload.len() > MAX_LAN_BEACON_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "LAN discovery beacon exceeds size bound",
        ));
    }
    Ok(payload)
}

fn is_lan_advertisable_addr(addr: &Multiaddr) -> bool {
    !addr.iter().any(|p| matches!(p, Protocol::P2pCircuit))
        && addr.iter().any(|p| {
            matches!(
                p,
                Protocol::Tcp(_) | Protocol::Udp(_) | Protocol::QuicV1 | Protocol::Ws(_)
            )
        })
}

fn normalize_received_addr(addr: Multiaddr, source_ip: IpAddr, peer: PeerId) -> Option<Multiaddr> {
    if addr.iter().any(|p| matches!(p, Protocol::P2pCircuit)) {
        return None;
    }
    let mut out = Multiaddr::empty();
    let mut has_local_ip = false;
    let mut peer_component = None;
    for protocol in addr.iter() {
        match protocol {
            Protocol::Ip4(ip) if ip.is_unspecified() => {
                let IpAddr::V4(source) = source_ip else {
                    return None;
                };
                has_local_ip = is_local_source(IpAddr::V4(source));
                out.push(Protocol::Ip4(source));
            }
            Protocol::Ip4(ip) => {
                has_local_ip = is_local_source(IpAddr::V4(ip));
                out.push(Protocol::Ip4(ip));
            }
            Protocol::Ip6(ip) if ip.is_unspecified() => return None,
            Protocol::Ip6(ip) => {
                has_local_ip = is_local_source(IpAddr::V6(ip));
                out.push(Protocol::Ip6(ip));
            }
            Protocol::P2p(found) => {
                peer_component = Some(found);
            }
            other => out.push(other),
        }
    }
    if !has_local_ip || peer_component.is_some_and(|found| found != peer) {
        return None;
    }
    out.push(Protocol::P2p(peer));
    Some(out)
}

fn is_local_source(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local(),
    }
}

#[cfg(target_os = "android")]
fn android_emulator_host_target(port: u16) -> Option<SocketAddr> {
    let probe = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    probe.connect((ANDROID_EMULATOR_HOST_V4, port)).ok()?;
    let IpAddr::V4(local_ip) = probe.local_addr().ok()?.ip() else {
        return None;
    };
    is_official_android_emulator_guest(local_ip)
        .then_some(SocketAddr::new(IpAddr::V4(ANDROID_EMULATOR_HOST_V4), port))
}

#[cfg(not(target_os = "android"))]
fn android_emulator_host_target(_port: u16) -> Option<SocketAddr> {
    None
}

#[cfg(any(target_os = "android", test))]
fn is_official_android_emulator_guest(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 10 && octets[1] == 0 && octets[2] == 2 && ip != ANDROID_EMULATOR_HOST_V4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_allows_multiple_lan_discovery_sockets_on_same_port() {
        let probe = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind probe");
        let port = probe.local_addr().expect("probe addr").port();
        drop(probe);

        let cfg = LanDiscoveryConfig {
            enabled: true,
            port,
            announce_interval_secs: 1,
        };

        let first = LanDiscoverySocket::bind(&cfg).expect("bind first LAN discovery socket");
        let second = LanDiscoverySocket::bind(&cfg)
            .expect("macOS LAN discovery must permit multiple listeners on the shared port");
        drop((first, second));
    }

    #[test]
    fn wildcard_listener_is_bound_to_beacon_source_and_peer() {
        let peer = PeerId::random();
        let input: Multiaddr = "/ip4/0.0.0.0/udp/4001/quic-v1".parse().unwrap();
        let output =
            normalize_received_addr(input, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 44)), peer)
                .expect("normalized");
        assert_eq!(
            output.to_string(),
            format!("/ip4/192.168.1.44/udp/4001/quic-v1/p2p/{peer}")
        );
    }

    #[test]
    fn remote_public_source_is_not_accepted_as_lan_discovery() {
        assert!(!is_local_source(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }

    #[test]
    fn official_android_emulator_subnet_is_detected_without_matching_host_alias() {
        assert!(is_official_android_emulator_guest(Ipv4Addr::new(
            10, 0, 2, 15
        )));
        assert!(is_official_android_emulator_guest(Ipv4Addr::new(
            10, 0, 2, 16
        )));
        assert!(!is_official_android_emulator_guest(
            ANDROID_EMULATOR_HOST_V4
        ));
        assert!(!is_official_android_emulator_guest(Ipv4Addr::new(
            192, 168, 1, 20
        )));
    }

    #[test]
    fn reply_requested_is_backward_compatible_and_explicit() {
        let peer = PeerId::random();
        let old_payload = format!(
            r#"{{"version":1,"network_id":1,"application_protocol":"/p2p-net/1","peer_id":"{peer}","addresses":[]}}"#
        );
        let old: LanBeacon = serde_json::from_str(&old_payload).unwrap();
        assert!(!old.reply_requested);

        let encoded = encode_beacon(
            1,
            "/p2p-net/1",
            peer,
            &["/ip4/0.0.0.0/tcp/4001".to_string()],
            true,
        )
        .unwrap();
        let decoded: LanBeacon = serde_json::from_slice(&encoded).unwrap();
        assert!(decoded.reply_requested);
    }

    #[test]
    fn emulator_probe_is_request_only_and_host_reply_is_dialable() {
        let local_peer = PeerId::random();
        let remote_peer = PeerId::random();
        let source = SocketAddr::from(([127, 0, 0, 1], 50_000));
        let request = encode_beacon(
            7,
            "/p2p-net/test",
            remote_peer,
            &["/ip4/0.0.0.0/tcp/4001".to_string()],
            true,
        )
        .unwrap();
        let decoded =
            decode_beacon(&request, source, local_peer, 7, "/p2p-net/test").expect("request");
        assert_eq!(decoded.reply_to, Some(source));
        assert!(decoded.announcement.is_none());

        let reply = encode_beacon(
            7,
            "/p2p-net/test",
            remote_peer,
            &["/ip4/0.0.0.0/tcp/4001".to_string()],
            false,
        )
        .unwrap();
        let source = SocketAddr::from(([10, 0, 2, 2], 44_777));
        let decoded = decode_beacon(&reply, source, local_peer, 7, "/p2p-net/test").expect("reply");
        assert!(decoded.reply_to.is_none());
        assert_eq!(
            decoded.announcement.unwrap().addresses[0].to_string(),
            format!("/ip4/10.0.2.2/tcp/4001/p2p/{remote_peer}")
        );
    }
}
