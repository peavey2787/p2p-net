//! Browser-side TURN server set: static `self_hosted` entries, credential
//! endpoints, and servers learned from broker peers (`libp2p`), merged in the
//! configured source order into one bounded list that is refreshed before
//! credentials expire. `browser_webrtc` composes it with STUN and ICE policy.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};
use web_time::Instant;

use super::config::{DiscoveryTrust, TurnClientConfig, TurnSource};
use super::credentials::TurnCredentials;
use crate::connectivity::ice::IceServer;

/// Minimum spacing between credential requests to the same broker.
const REQUEST_COOLDOWN: Duration = Duration::from_secs(60);
/// Hosts remembered per peer, and peers tracked at once.
const MAX_HOSTS_PER_PEER: usize = 8;
const MAX_TRACKED_PEERS: usize = 256;

/// Credentials from one source with their refresh and expiry deadlines.
#[derive(Debug, Clone)]
pub(crate) struct LearnedServers {
    pub(crate) servers: Vec<IceServer>,
    pub(crate) refresh_at: Instant,
    pub(crate) expires_at: Instant,
}

impl LearnedServers {
    /// Refresh at three quarters of the lifetime so a new set arrives first.
    pub(crate) fn new(credentials: TurnCredentials, now: Instant) -> Self {
        let ttl = Duration::from_secs(credentials.ttl_secs());
        Self {
            servers: credentials.ice_servers,
            refresh_at: now + ttl * 3 / 4,
            expires_at: now + ttl,
        }
    }
}

/// Outcome of a broker peer's credential offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrokerOffer {
    pub accepted: usize,
    pub rejected: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnClientStatus {
    pub endpoints_active: usize,
    pub broker_peers: usize,
}

#[derive(Debug)]
pub struct TurnClientState {
    sources: Vec<TurnSource>,
    static_servers: Vec<IceServer>,
    discovery: Option<DiscoveryTrust>,
    accepted_hosts: Vec<String>,
    max_servers: usize,
    ttl_secs: u64,
    endpoints: BTreeMap<TurnSource, LearnedServers>,
    brokers: BTreeMap<PeerId, LearnedServers>,
    peer_hosts: HashMap<PeerId, Vec<String>>,
    requested_at: HashMap<PeerId, Instant>,
    unsupported: HashSet<PeerId>,
}

impl TurnClientState {
    pub fn new(cfg: &TurnClientConfig) -> Self {
        let sources = cfg.sources();
        Self {
            static_servers: if sources.contains(&TurnSource::SelfHosted) {
                cfg.servers.clone()
            } else {
                Vec::new()
            },
            discovery: cfg.discovery(),
            sources,
            accepted_hosts: cfg.accepted_provider_hosts(),
            max_servers: cfg.max_servers,
            ttl_secs: cfg.credential_ttl_secs,
            endpoints: BTreeMap::new(),
            brokers: BTreeMap::new(),
            peer_hosts: HashMap::new(),
            requested_at: HashMap::new(),
            unsupported: HashSet::new(),
        }
    }

    /// Lifetime to request from brokers.
    pub fn requested_ttl_secs(&self) -> u64 {
        self.ttl_secs
    }

    /// Remember the host a direct connection to `peer` uses; relayed
    /// connections say nothing about where the peer's TURN server runs.
    pub fn note_connection(&mut self, peer: PeerId, remote: &Multiaddr) {
        if remote.iter().any(|p| matches!(p, Protocol::P2pCircuit)) {
            return;
        }
        let Some(host) = multiaddr_host(remote) else {
            return;
        };
        if !self.peer_hosts.contains_key(&peer) && self.peer_hosts.len() >= MAX_TRACKED_PEERS {
            return;
        }
        let hosts = self.peer_hosts.entry(peer).or_default();
        if !hosts.contains(&host) && hosts.len() < MAX_HOSTS_PER_PEER {
            hosts.push(host);
        }
    }

    /// The last connection to `peer` closed. Learned credentials stay valid
    /// until they expire; they simply cannot be refreshed from this peer.
    pub fn forget_peer(&mut self, peer: &PeerId) {
        self.peer_hosts.remove(peer);
        self.requested_at.remove(peer);
    }

    /// Decide whether to ask `peer` for credentials now. Records the request.
    pub fn should_request(&mut self, peer: PeerId, now: Instant) -> bool {
        if self.discovery.is_none() || self.unsupported.contains(&peer) {
            return false;
        }
        let recently_asked = self
            .requested_at
            .get(&peer)
            .is_some_and(|at| now.saturating_duration_since(*at) < REQUEST_COOLDOWN);
        let still_fresh = self
            .brokers
            .get(&peer)
            .is_some_and(|learned| learned.refresh_at > now);
        if recently_asked || still_fresh {
            return false;
        }
        self.requested_at.insert(peer, now);
        true
    }

    /// `peer` does not speak the broker protocol; never ask it again.
    pub fn mark_unsupported(&mut self, peer: PeerId) {
        self.unsupported.insert(peer);
        self.requested_at.remove(&peer);
    }

    /// Connected brokers whose credentials are due for a refresh.
    pub fn brokers_due(&self, now: Instant) -> Vec<PeerId> {
        self.brokers
            .iter()
            .filter(|(peer, learned)| {
                learned.refresh_at <= now && self.peer_hosts.contains_key(peer)
            })
            .map(|(peer, _)| *peer)
            .collect()
    }

    /// Keep the offered servers this node's policy accepts.
    pub fn accept_from_broker(
        &mut self,
        peer: PeerId,
        mut credentials: TurnCredentials,
        now: Instant,
    ) -> BrokerOffer {
        let dropped = credentials.sanitize();
        let offered = credentials.ice_servers.len();
        credentials
            .ice_servers
            .retain(|server| self.host_allowed(&peer, server));
        let accepted = credentials.ice_servers.len();
        if accepted > 0 {
            self.brokers
                .insert(peer, LearnedServers::new(credentials, now));
        }
        BrokerOffer {
            accepted,
            rejected: dropped + offered - accepted,
        }
    }

    /// Store fresh credentials from an endpoint source.
    pub fn accept_from_endpoint(
        &mut self,
        source: TurnSource,
        credentials: TurnCredentials,
        now: Instant,
    ) {
        self.endpoints
            .insert(source, LearnedServers::new(credentials, now));
    }

    /// Drop expired credentials. Returns true when the effective list changed.
    pub fn prune(&mut self, now: Instant) -> bool {
        let before = self.brokers.len() + self.endpoints.len();
        self.brokers.retain(|_, learned| learned.expires_at > now);
        self.endpoints.retain(|_, learned| learned.expires_at > now);
        before != self.brokers.len() + self.endpoints.len()
    }

    /// Servers from every active source in source order; duplicates removed
    /// and capped at `max_servers`.
    pub fn effective(&self) -> Vec<IceServer> {
        let mut out: Vec<IceServer> = Vec::new();
        for server in self
            .sources
            .iter()
            .flat_map(|source| self.servers_from(*source))
        {
            if out.len() == self.max_servers {
                break;
            }
            if !out.iter().any(|existing| existing.urls == server.urls) {
                out.push(server.clone());
            }
        }
        out
    }

    pub fn status(&self) -> TurnClientStatus {
        TurnClientStatus {
            endpoints_active: self.endpoints.len(),
            broker_peers: self.brokers.len(),
        }
    }

    fn servers_from(&self, source: TurnSource) -> Box<dyn Iterator<Item = &IceServer> + '_> {
        fn learned(learned: &LearnedServers) -> std::slice::Iter<'_, IceServer> {
            learned.servers.iter()
        }
        match source {
            TurnSource::SelfHosted => Box::new(self.static_servers.iter()),
            TurnSource::Libp2p => Box::new(self.brokers.values().flat_map(learned)),
            TurnSource::Cloudflare | TurnSource::Http => {
                Box::new(self.endpoints.get(&source).into_iter().flat_map(learned))
            }
        }
    }

    fn host_allowed(&self, peer: &PeerId, server: &IceServer) -> bool {
        match self.discovery {
            None => false,
            Some(DiscoveryTrust::Any) => true,
            Some(DiscoveryTrust::SameHost) => {
                let peer_hosts = self.peer_hosts.get(peer);
                server.hosts().all(|host| {
                    self.accepted_hosts.contains(&host)
                        || peer_hosts.is_some_and(|hosts| hosts.contains(&host))
                })
            }
        }
    }
}

fn multiaddr_host(addr: &Multiaddr) -> Option<String> {
    addr.iter().find_map(|protocol| match protocol {
        Protocol::Ip4(ip) => Some(ip.to_string()),
        Protocol::Ip6(ip) => Some(ip.to_string()),
        Protocol::Dns(name) | Protocol::Dns4(name) | Protocol::Dns6(name) => {
            Some(name.to_ascii_lowercase())
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn_at(host: &str) -> IceServer {
        IceServer {
            urls: vec![format!("turn:{host}:3478?transport=udp")],
            username: Some("u".to_string()),
            credential: Some("c".to_string()),
        }
    }

    fn offer(servers: Vec<IceServer>, ttl: u64) -> TurnCredentials {
        TurnCredentials {
            ice_servers: servers,
            ttl: Some(ttl),
        }
    }

    fn state(trust: DiscoveryTrust) -> TurnClientState {
        TurnClientState::new(&TurnClientConfig {
            discovery_trust: trust,
            ..TurnClientConfig::default()
        })
    }

    #[test]
    fn same_host_accepts_only_the_dialed_host_or_trusted_providers() {
        let mut client = state(DiscoveryTrust::SameHost);
        let peer = PeerId::random();
        let now = Instant::now();
        client.note_connection(
            peer,
            &"/dns4/relay.example.org/tcp/443/wss".parse().unwrap(),
        );
        let result = client.accept_from_broker(
            peer,
            offer(
                vec![
                    turn_at("relay.example.org"),
                    turn_at("turn.cloudflare.com"),
                    turn_at("victim.example.net"),
                ],
                3600,
            ),
            now,
        );
        assert_eq!(
            result,
            BrokerOffer {
                accepted: 2,
                rejected: 1
            }
        );
        assert_eq!(client.effective().len(), 2);
    }

    #[test]
    fn relayed_connections_do_not_vouch_for_hosts() {
        let mut client = state(DiscoveryTrust::SameHost);
        let peer = PeerId::random();
        let relayed: Multiaddr = format!(
            "/ip4/203.0.113.9/tcp/4001/p2p/{}/p2p-circuit",
            PeerId::random()
        )
        .parse()
        .unwrap();
        client.note_connection(peer, &relayed);
        let result = client.accept_from_broker(
            peer,
            offer(vec![turn_at("203.0.113.9")], 3600),
            Instant::now(),
        );
        assert_eq!(result.accepted, 0);
    }

    #[test]
    fn request_cooldown_and_unsupported_peers() {
        let mut client = state(DiscoveryTrust::Any);
        let peer = PeerId::random();
        let now = Instant::now();
        assert!(client.should_request(peer, now));
        assert!(!client.should_request(peer, now + Duration::from_secs(1)));
        assert!(client.should_request(peer, now + REQUEST_COOLDOWN));
        client.mark_unsupported(peer);
        assert!(!client.should_request(peer, now + REQUEST_COOLDOWN * 3));
        let mut off = TurnClientState::new(&TurnClientConfig {
            auto_discover: false,
            ..TurnClientConfig::default()
        });
        assert!(!off.should_request(PeerId::random(), now));
    }

    #[test]
    fn refresh_is_due_before_expiry_and_expired_sets_are_pruned() {
        let mut client = state(DiscoveryTrust::Any);
        let peer = PeerId::random();
        let now = Instant::now();
        client.note_connection(peer, &"/ip4/198.18.0.1/udp/4001/quic-v1".parse().unwrap());
        client.accept_from_broker(peer, offer(vec![turn_at("198.18.0.1")], 400), now);
        assert!(client.brokers_due(now).is_empty());
        assert_eq!(
            client.brokers_due(now + Duration::from_secs(300)),
            vec![peer]
        );
        assert!(!client.prune(now + Duration::from_secs(399)));
        assert!(client.prune(now + Duration::from_secs(400)));
        assert!(client.effective().is_empty());
    }

    #[test]
    fn effective_list_follows_source_order_dedupes_and_caps() {
        let mut client = TurnClientState::new(&TurnClientConfig {
            servers: vec![turn_at("own.example.org")],
            http: Some(super::super::HttpProviderConfig::new(
                "https://w.example.dev/ice",
            )),
            providers: Some(vec![
                TurnSource::Http,
                TurnSource::SelfHosted,
                TurnSource::Libp2p,
            ]),
            discovery_trust: DiscoveryTrust::Any,
            max_servers: 3,
            ..TurnClientConfig::default()
        });
        let now = Instant::now();
        client.accept_from_endpoint(
            TurnSource::Http,
            offer(vec![turn_at("a.example.org")], 3600),
            now,
        );
        for host in ["a.example.org", "b.example.org", "c.example.org"] {
            client.accept_from_broker(PeerId::random(), offer(vec![turn_at(host)], 3600), now);
        }
        // Brokers are ordered by peer id, so any non-duplicate broker may
        // fill the last slot; the duplicate of the endpoint's server never does.
        let effective = client.effective();
        assert_eq!(
            effective[..2],
            [turn_at("a.example.org"), turn_at("own.example.org")]
        );
        assert!([turn_at("b.example.org"), turn_at("c.example.org")].contains(&effective[2]));
        assert_eq!(effective.len(), 3);
        assert_eq!(client.status().endpoints_active, 1);
        assert!(client.prune(now + Duration::from_secs(3600)));
        assert_eq!(client.effective(), vec![turn_at("own.example.org")]);
    }

    #[test]
    fn a_disabled_client_has_no_servers_and_asks_no_brokers() {
        let mut client = TurnClientState::new(&TurnClientConfig {
            enabled: false,
            ..TurnClientConfig::default()
        });
        assert!(client.effective().is_empty());
        assert!(!client.should_request(PeerId::random(), Instant::now()));
    }
}
