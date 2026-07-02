use libp2p::Multiaddr;

use crate::api::PeerSource;
use crate::common::error::NetError;
use crate::connectivity::peer_book::PeerBook;
use crate::connectivity::relay_discovery::{self, RelaySelectionPlan};
use crate::connectivity::{dns, peer_cache};
use crate::platform::NodeStorage;
use crate::stack::{
    extract_p2p_peer_id, startup_discovery_plan, startup_discovery_plan_with_public,
    StartupDiscoveryPlan,
};

use super::super::config::NodeConfig;
use super::super::profile::ResolvedNodeConfig;

pub(super) struct StartupAddrs {
    bootstrap_peers: Vec<Multiaddr>,
    bootstrap_seed_peers: Vec<Multiaddr>,
    pub(super) public_bootstrap_seed_peers: Vec<Multiaddr>,
    pub(super) public_rendezvous_peers: Vec<Multiaddr>,
    pub(super) public_relay_peers: Vec<Multiaddr>,
    pub(super) rendezvous_peers: Vec<Multiaddr>,
    relay_peers: Vec<Multiaddr>,
    cached_peers: Vec<Multiaddr>,
    cached_relay_peers: Vec<Multiaddr>,
}

impl StartupAddrs {
    pub(super) fn owned_startup_candidate_count(&self) -> usize {
        startup_discovery_plan(
            self.bootstrap_peers.clone(),
            self.bootstrap_seed_peers.clone(),
            self.rendezvous_peers.clone(),
            self.cached_peers.clone(),
        )
        .dial_addrs
        .len()
    }

    pub(super) fn startup_plan(
        &self,
        use_public_bootstrap: bool,
        use_public_rendezvous: bool,
    ) -> StartupDiscoveryPlan {
        startup_discovery_plan_with_public(
            self.bootstrap_peers.clone(),
            self.bootstrap_seed_peers.clone(),
            self.rendezvous_peers(use_public_rendezvous),
            self.cached_peers.clone(),
            if use_public_bootstrap {
                self.public_bootstrap_seed_peers.clone()
            } else {
                Vec::new()
            },
            use_public_bootstrap || use_public_rendezvous,
        )
    }

    pub(super) fn rendezvous_peers(&self, use_public_rendezvous: bool) -> Vec<Multiaddr> {
        let mut peers = self.rendezvous_peers.clone();
        if use_public_rendezvous {
            for addr in &self.public_rendezvous_peers {
                if !peers.contains(addr) {
                    peers.push(addr.clone());
                }
            }
        }
        peers
    }

    pub(super) fn peer_book(
        &self,
        use_public_bootstrap: bool,
        use_public_rendezvous: bool,
        selected_relay_peers: &[Multiaddr],
        use_public_relay: bool,
    ) -> PeerBook {
        let mut peer_book = PeerBook::default();
        record_peer_book_addrs(&mut peer_book, &self.bootstrap_peers, PeerSource::Bootstrap);
        record_peer_book_addrs(
            &mut peer_book,
            &self.bootstrap_seed_peers,
            PeerSource::BootstrapSeed,
        );
        record_peer_book_addrs(&mut peer_book, &self.rendezvous_peers, PeerSource::Rendezvous);
        record_peer_book_addrs(&mut peer_book, &self.cached_peers, PeerSource::PeerCache);
        if use_public_bootstrap {
            record_peer_book_addrs(
                &mut peer_book,
                &self.public_bootstrap_seed_peers,
                PeerSource::PublicBootstrapSeed,
            );
        }
        if use_public_rendezvous {
            record_peer_book_addrs(
                &mut peer_book,
                &self.public_rendezvous_peers,
                PeerSource::PublicRendezvous,
            );
        }
        record_peer_book_addrs(
            &mut peer_book,
            selected_relay_peers,
            PeerSource::RelayDiscovery,
        );
        if use_public_relay {
            record_peer_book_addrs(
                &mut peer_book,
                &self.public_relay_candidates(),
                PeerSource::PublicRelayDiscovery,
            );
        }
        peer_book
    }

    pub(super) fn relay_selection_plan(
        &self,
        cfg: &NodeConfig,
        resolved_config: &ResolvedNodeConfig,
    ) -> RelaySelectionPlan {
        relay_discovery::select_startup_relays(
            &relay_discovery_policy(cfg, resolved_config),
            self.relay_peers.clone(),
            self.cached_relay_peers.clone(),
            self.rendezvous_peers.clone(),
            Vec::new(),
        )
    }

    pub(super) fn public_relay_candidate_count(&self) -> usize {
        self.public_relay_candidates().len()
    }

    pub(super) fn relay_selection_plan_with_public(
        &self,
        cfg: &NodeConfig,
        resolved_config: &ResolvedNodeConfig,
    ) -> RelaySelectionPlan {
        relay_discovery::select_startup_relays(
            &relay_discovery_policy(cfg, resolved_config),
            self.relay_peers.clone(),
            self.cached_relay_peers.clone(),
            self.rendezvous_peers.clone(),
            self.public_relay_candidates(),
        )
    }

    fn public_relay_candidates(&self) -> Vec<Multiaddr> {
        if self.public_relay_peers.is_empty() {
            // Consumer public fallback has no separate bundled relay fleet.
            // Treat resolved public libp2p bootstrap peers as best-effort relay
            // candidates too: nodes that support Circuit Relay v2 will accept a
            // reservation, while non-relays fail visibly and do not block DHT
            // discovery or direct dials.
            return self.public_bootstrap_seed_peers.clone();
        }
        self.public_relay_peers.clone()
    }
}

pub(super) async fn resolve_startup_addrs(
    cfg: &NodeConfig,
    storage: &dyn NodeStorage,
) -> Result<StartupAddrs, NetError> {
    let bootstrap_peers = dns::resolve_configured_multiaddrs(
        "bootstrap_peers",
        cfg.parsed_bootstrap_peers()?,
        &cfg.dnsaddr,
    )
    .await?;
    let bootstrap_seed_peers = dns::resolve_configured_multiaddrs(
        "discovery.bootstrap_seed_peers",
        cfg.parsed_bootstrap_seed_peers()?,
        &cfg.dnsaddr,
    )
    .await?;
    let public_bootstrap_seed_peers = dns::resolve_cached_multiaddrs(
        cfg.parsed_public_bootstrap_seed_peers()?,
        &cfg.dnsaddr,
    )
    .await;
    let public_rendezvous_peers = dns::resolve_cached_multiaddrs(
        cfg.parsed_public_rendezvous_peers()?,
        &cfg.dnsaddr,
    )
    .await;
    let public_relay_peers = dns::resolve_cached_multiaddrs(
        cfg.parsed_public_relay_peers()?,
        &cfg.dnsaddr,
    )
    .await;
    let rendezvous_peers = dns::resolve_configured_multiaddrs(
        "discovery.rendezvous_peers",
        cfg.parsed_rendezvous_peers()?,
        &cfg.dnsaddr,
    )
    .await?;
    let relay_peers = dns::resolve_configured_multiaddrs(
        "relay_peers",
        cfg.parsed_relay_peers()?,
        &cfg.dnsaddr,
    )
    .await?;
    let cached_startup_addrs = peer_cache::load_last_addrs_with_storage(
        &cfg.discovery,
        cfg.startup_peer_cache_probe
            .max(cfg.discovery.relay_discovery.max_reservations),
        storage,
    );
    let cached_peers =
        dns::resolve_cached_multiaddrs(cached_startup_addrs.clone(), &cfg.dnsaddr).await;
    let cached_relay_peers = dns::resolve_cached_multiaddrs(cached_startup_addrs, &cfg.dnsaddr).await;

    Ok(StartupAddrs {
        bootstrap_peers,
        bootstrap_seed_peers,
        public_bootstrap_seed_peers,
        public_rendezvous_peers,
        public_relay_peers,
        rendezvous_peers,
        relay_peers,
        cached_peers,
        cached_relay_peers,
    })
}

fn relay_discovery_policy(
    cfg: &NodeConfig,
    resolved_config: &ResolvedNodeConfig,
) -> relay_discovery::RelayDiscoveryPolicy {
    if resolved_config.relay_discovery_enabled {
        cfg.discovery.relay_discovery.clone()
    } else {
        relay_discovery::RelayDiscoveryPolicy {
            enabled: false,
            ..cfg.discovery.relay_discovery.clone()
        }
    }
}

fn record_peer_book_addrs(peer_book: &mut PeerBook, addrs: &[Multiaddr], source: PeerSource) {
    for addr in addrs {
        if let Some(peer) = extract_p2p_peer_id(addr) {
            peer_book.record_addr(peer, addr.clone(), source);
        }
    }
}
