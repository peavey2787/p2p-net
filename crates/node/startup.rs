//! Startup discovery preparation for node orchestration.
//!
//! This module keeps DNS/bootstrap resolution, public fallback decisions,
//! peer-book seeding, and startup relay selection out of the main node
//! orchestration loop.

use libp2p::Multiaddr;

use crate::api::PeerSource;
use crate::common::error::NetError;
use crate::connectivity::peer_book::PeerBook;
use crate::connectivity::public_fallback::PublicFallbackDecision;
use crate::connectivity::relay_discovery::{self, RelaySelectionPlan};
use crate::connectivity::{dns, peer_cache};
use crate::platform::NodeStorage;
use crate::stack::{
    extract_p2p_peer_id, startup_discovery_plan, startup_discovery_plan_with_public,
    StartupDiscoveryPlan,
};

use super::profile::ResolvedNodeConfig;
use super::config::NodeConfig;

pub(crate) struct StartupDiscoverySetup {
    pub(crate) startup_plan: StartupDiscoveryPlan,
    pub(crate) rendezvous_peers: Vec<Multiaddr>,
    pub(crate) peer_book: PeerBook,
    pub(crate) relay_selection_plan: RelaySelectionPlan,
    pub(crate) public_bootstrap_decision: PublicFallbackDecision,
    pub(crate) public_relay_decision: PublicFallbackDecision,
}

pub(crate) async fn prepare_startup_discovery(
    cfg: &NodeConfig,
    resolved_config: &ResolvedNodeConfig,
    storage: &dyn NodeStorage,
) -> Result<StartupDiscoverySetup, NetError> {
    let startup_addrs = resolve_startup_addrs(cfg, storage).await?;
    let public_bootstrap_decision = public_decision_for_resolved_addrs(
        cfg.discovery
            .public_bootstrap
            .bootstrap_decision(startup_addrs.owned_startup_candidate_count()),
        startup_addrs.public_bootstrap_seed_peers.len(),
        "no_resolved_public_bootstrap_candidates",
    );
    let startup_plan = startup_addrs.startup_plan(public_bootstrap_decision.used);
    let relay_selection_plan = startup_addrs.relay_selection_plan(cfg, resolved_config);
    let public_relay_decision = public_decision_for_resolved_addrs(
        cfg.discovery
            .public_bootstrap
            .relay_decision(relay_selection_plan.selected_addrs.len()),
        startup_addrs.public_relay_peers.len(),
        "no_resolved_public_relay_candidates",
    );
    let relay_selection_plan = if public_relay_decision.used {
        startup_addrs.relay_selection_plan_with_public(cfg, resolved_config)
    } else {
        relay_selection_plan
    };
    let peer_book = startup_addrs.peer_book(
        public_bootstrap_decision.used,
        &relay_selection_plan.selected_addrs,
    );

    Ok(StartupDiscoverySetup {
        startup_plan,
        rendezvous_peers: startup_addrs.rendezvous_peers,
        peer_book,
        relay_selection_plan,
        public_bootstrap_decision,
        public_relay_decision,
    })
}


fn public_decision_for_resolved_addrs(
    decision: PublicFallbackDecision,
    resolved_public_candidates: usize,
    empty_reason: &'static str,
) -> PublicFallbackDecision {
    if decision.used && resolved_public_candidates == 0 {
        return PublicFallbackDecision::new(decision.mode, false, empty_reason);
    }
    decision
}

struct StartupAddrs {
    bootstrap_peers: Vec<Multiaddr>,
    bootstrap_seed_peers: Vec<Multiaddr>,
    public_bootstrap_seed_peers: Vec<Multiaddr>,
    public_relay_peers: Vec<Multiaddr>,
    rendezvous_peers: Vec<Multiaddr>,
    relay_peers: Vec<Multiaddr>,
    cached_peers: Vec<Multiaddr>,
    cached_relay_peers: Vec<Multiaddr>,
}

impl StartupAddrs {
    fn owned_startup_candidate_count(&self) -> usize {
        startup_discovery_plan(
            self.bootstrap_peers.clone(),
            self.bootstrap_seed_peers.clone(),
            self.rendezvous_peers.clone(),
            self.cached_peers.clone(),
        )
        .dial_addrs
        .len()
    }

    fn startup_plan(&self, use_public_bootstrap: bool) -> StartupDiscoveryPlan {
        startup_discovery_plan_with_public(
            self.bootstrap_peers.clone(),
            self.bootstrap_seed_peers.clone(),
            self.rendezvous_peers.clone(),
            self.cached_peers.clone(),
            if use_public_bootstrap {
                self.public_bootstrap_seed_peers.clone()
            } else {
                Vec::new()
            },
            use_public_bootstrap,
        )
    }

    fn peer_book(
        &self,
        use_public_bootstrap: bool,
        selected_relay_peers: &[Multiaddr],
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
                PeerSource::BootstrapSeed,
            );
        }
        record_peer_book_addrs(
            &mut peer_book,
            selected_relay_peers,
            PeerSource::RelayDiscovery,
        );
        peer_book
    }

    fn relay_selection_plan(
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

    fn relay_selection_plan_with_public(
        &self,
        cfg: &NodeConfig,
        resolved_config: &ResolvedNodeConfig,
    ) -> RelaySelectionPlan {
        relay_discovery::select_startup_relays(
            &relay_discovery_policy(cfg, resolved_config),
            self.relay_peers.clone(),
            self.cached_relay_peers.clone(),
            self.rendezvous_peers.clone(),
            self.public_relay_peers.clone(),
        )
    }
}

async fn resolve_startup_addrs(
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
    // Built-in public fallback is best-effort: DNS outages must not stop startup.
    let public_bootstrap_seed_peers = dns::resolve_cached_multiaddrs(
        cfg.parsed_public_bootstrap_seed_peers()?,
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
