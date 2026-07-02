//! Startup discovery preparation for node orchestration.
//!
//! This module keeps DNS/bootstrap resolution, public fallback decisions,
//! peer-book seeding, and startup relay selection out of the main node
//! orchestration loop.

mod addrs;

use libp2p::Multiaddr;

use crate::common::error::NetError;
use crate::connectivity::peer_book::PeerBook;
use crate::connectivity::public_fallback::PublicFallbackDecision;
use crate::connectivity::relay_discovery::RelaySelectionPlan;
use crate::platform::NodeStorage;
use crate::stack::StartupDiscoveryPlan;

use super::config::NodeConfig;
use super::profile::ResolvedNodeConfig;
use self::addrs::resolve_startup_addrs;

pub(crate) struct StartupDiscoverySetup {
    pub(crate) startup_plan: StartupDiscoveryPlan,
    pub(crate) rendezvous_peers: Vec<Multiaddr>,
    pub(crate) peer_book: PeerBook,
    pub(crate) relay_selection_plan: RelaySelectionPlan,
    pub(crate) public_bootstrap_decision: PublicFallbackDecision,
    pub(crate) public_rendezvous_decision: PublicFallbackDecision,
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
    let public_rendezvous_decision = public_decision_for_resolved_addrs(
        cfg.discovery
            .public_bootstrap
            .rendezvous_decision(startup_addrs.rendezvous_peers.len()),
        startup_addrs.public_rendezvous_peers.len(),
        "no_resolved_public_rendezvous_candidates",
    );
    let rendezvous_peers = startup_addrs.rendezvous_peers(public_rendezvous_decision.used);
    let startup_plan = startup_addrs.startup_plan(
        public_bootstrap_decision.used,
        public_rendezvous_decision.used,
    );
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
        public_rendezvous_decision.used,
        &relay_selection_plan.selected_addrs,
    );

    Ok(StartupDiscoverySetup {
        startup_plan,
        rendezvous_peers,
        peer_book,
        relay_selection_plan,
        public_bootstrap_decision,
        public_rendezvous_decision,
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
