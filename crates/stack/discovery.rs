use libp2p::swarm::Swarm;
use libp2p::{identify, Multiaddr, PeerId};
use libp2p_rendezvous as rendezvous;

use crate::api::PeerSource;
use crate::connectivity::discovery::DiscoveryConfig;
use crate::connectivity::peer_book::PeerBook;
use crate::connectivity::peer_cache::PeerCacheWriteBatch;
use crate::connectivity::relay::{relay_reservation_addr, RelayReservationPlan};
use crate::connectivity::rendezvous::{
    peer_record_addrs, RendezvousActionPlan, RendezvousPeerNamespace, RendezvousState,
};

use super::behaviour::{MeshBehaviour, MeshEvent};

const MAX_IDENTIFY_ADDRS_PER_PEER: usize = 8;

mod identify_state;
pub use identify_state::IdentifyAddressState;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartupDiscoveryPlan {
    pub dial_addrs: Vec<Multiaddr>,
    pub bootstrap_peer_count: usize,
    pub bootstrap_seed_count: usize,
    pub rendezvous_seed_count: usize,
    pub cached_peer_count: usize,
    pub public_bootstrap_seed_count: usize,
    pub public_fallback_used: bool,
}

pub fn startup_discovery_plan(
    bootstrap_peers: Vec<Multiaddr>,
    bootstrap_seed_peers: Vec<Multiaddr>,
    rendezvous_peers: Vec<Multiaddr>,
    cached_peers: Vec<Multiaddr>,
) -> StartupDiscoveryPlan {
    startup_discovery_plan_with_public(
        bootstrap_peers,
        bootstrap_seed_peers,
        rendezvous_peers,
        cached_peers,
        Vec::new(),
        false,
    )
}

pub fn startup_discovery_plan_with_public(
    bootstrap_peers: Vec<Multiaddr>,
    bootstrap_seed_peers: Vec<Multiaddr>,
    rendezvous_peers: Vec<Multiaddr>,
    cached_peers: Vec<Multiaddr>,
    public_bootstrap_seed_peers: Vec<Multiaddr>,
    public_fallback_used: bool,
) -> StartupDiscoveryPlan {
    let mut plan = StartupDiscoveryPlan {
        bootstrap_peer_count: bootstrap_peers.len(),
        bootstrap_seed_count: bootstrap_seed_peers.len(),
        rendezvous_seed_count: rendezvous_peers.len(),
        cached_peer_count: cached_peers.len(),
        public_bootstrap_seed_count: public_bootstrap_seed_peers.len(),
        public_fallback_used,
        dial_addrs: Vec::new(),
    };

    for addr in bootstrap_peers
        .into_iter()
        .chain(bootstrap_seed_peers)
        .chain(rendezvous_peers)
        .chain(cached_peers)
        .chain(public_bootstrap_seed_peers)
    {
        if extract_p2p_peer_id(&addr).is_some() && !plan.dial_addrs.contains(&addr) {
            plan.dial_addrs.push(addr);
        }
    }

    plan
}

pub fn seed_bootstrap(swarm: &mut Swarm<MeshBehaviour>, addrs: &[Multiaddr]) {
    let local_peer = *swarm.local_peer_id();
    for addr in addrs {
        if let Some(peer) = extract_p2p_peer_id(addr) {
            if peer == local_peer {
                continue;
            }
            add_peer_address_to_discovery(swarm, peer, addr.clone());
            let _ = swarm.dial(addr.clone());
        }
    }
    let _ = swarm.behaviour_mut().kademlia.bootstrap();
}

/// Dial selected relays and request Circuit Relay v2 reservations.
pub fn reserve_selected_relays(
    swarm: &mut Swarm<MeshBehaviour>,
    relay_addrs: &[Multiaddr],
) -> RelayReservationPlan {
    let mut plan = RelayReservationPlan::default();

    let local_peer = *swarm.local_peer_id();
    for relay_addr in relay_addrs {
        if let Some(peer) = extract_p2p_peer_id(relay_addr) {
            if peer == local_peer {
                plan.errors.push(format!(
                    "relay reservation skipped local peer address: {relay_addr}"
                ));
                continue;
            }
            add_peer_address_to_discovery(swarm, peer, relay_addr.clone());
        }

        let _ = swarm.dial(relay_addr.clone());

        let Some(listen_addr) = relay_reservation_addr(relay_addr) else {
            plan.errors.push(format!(
                "relay peer address cannot be converted to reservation address: {relay_addr}"
            ));
            continue;
        };

        match swarm.listen_on(listen_addr.clone()) {
            Ok(_) => {
                plan.attempted = plan.attempted.saturating_add(1);
                plan.listen_addrs.push(listen_addr);
            }
            Err(err) => plan.errors.push(format!(
                "relay reservation listen_on failed for {listen_addr}: {err}"
            )),
        }
    }

    plan
}

/// Dial configured rendezvous peers and issue deduplicated register/discover requests.
pub fn refresh_rendezvous(
    swarm: &mut Swarm<MeshBehaviour>,
    network_id: u32,
    discovery_cfg: &DiscoveryConfig,
    rendezvous_addrs: &[Multiaddr],
    state: &mut RendezvousState,
) -> RendezvousActionPlan {
    let mut plan = RendezvousActionPlan::default();
    let rv_cfg = &discovery_cfg.rendezvous;

    if !rv_cfg.client_enabled || rendezvous_addrs.is_empty() {
        return plan;
    }

    let namespace_strings = match discovery_cfg.rendezvous_namespaces(network_id) {
        Ok(v) => v,
        Err(err) => {
            plan.errors.push(err.to_string());
            return plan;
        }
    };
    let mut namespaces = Vec::new();
    for namespace in namespace_strings {
        match rendezvous::Namespace::new(namespace.clone()) {
            Ok(value) => namespaces.push((namespace, value)),
            Err(err) => {
                plan.errors.push(format!(
                    "rendezvous namespace `{namespace}` is invalid: {err}"
                ));
            }
        }
    }
    if namespaces.is_empty() {
        return plan;
    }

    for addr in rendezvous_addrs {
        let Some(peer) = extract_p2p_peer_id(addr) else {
            plan.errors
                .push(format!("rendezvous address lacks /p2p/<PeerId>: {addr}"));
            continue;
        };

        add_peer_address_to_discovery(swarm, peer, addr.clone());
        let _ = swarm.dial(addr.clone());

        let Some(client) = swarm.behaviour_mut().rendezvous_client.as_mut() else {
            plan.errors
                .push("rendezvous client behaviour is disabled".to_string());
            continue;
        };

        for (namespace_key, namespace) in &namespaces {
            if rv_cfg.register
                && !state.is_registered(peer, namespace_key)
                && !state.is_register_inflight(peer, namespace_key)
            {
                state.register_attempts = state.register_attempts.saturating_add(1);
                match client.register(namespace.clone(), peer, Some(rv_cfg.register_ttl_secs)) {
                    Ok(()) => {
                        state.mark_register_inflight(peer, namespace_key);
                        plan.register_attempts = plan.register_attempts.saturating_add(1);
                    }
                    Err(err) => {
                        state.register_failures = state.register_failures.saturating_add(1);
                        plan.errors.push(format!(
                            "rendezvous register request failed peer={peer} namespace={namespace}: {err}"
                        ));
                    }
                }
            }

            if rv_cfg.discover
                && !state.discover_inflight.contains(&peer)
                && !state
                    .discover_inflight_namespaces
                    .contains(&RendezvousPeerNamespace::new(peer, namespace_key.as_str()))
            {
                state.discover_attempts = state.discover_attempts.saturating_add(1);
                let cookie = state.discover_cookie(peer, namespace_key);
                client.discover(
                    Some(namespace.clone()),
                    cookie,
                    rv_cfg.discover_limit(),
                    peer,
                );
                state.mark_discover_inflight(peer, namespace_key);
                plan.discover_attempts = plan.discover_attempts.saturating_add(1);
            }
        }
    }

    plan
}

pub fn on_mesh_event(
    swarm: &mut Swarm<MeshBehaviour>,
    event: &MeshEvent,
    _discovery_cfg: &DiscoveryConfig,
    peer_cache_writes: &mut PeerCacheWriteBatch,
    peer_book: &mut PeerBook,
    identify_addresses: &mut IdentifyAddressState,
) {
    if let MeshEvent::Identify(ev) = event {
        on_identify_event(
            swarm,
            ev,
            peer_cache_writes,
            peer_book,
            identify_addresses,
        );
    }
}

pub fn on_rendezvous_client_event(
    swarm: &mut Swarm<MeshBehaviour>,
    event: &rendezvous::client::Event,
    _discovery_cfg: &DiscoveryConfig,
    peer_cache_writes: &mut PeerCacheWriteBatch,
    state: &mut RendezvousState,
) -> String {
    match event {
        rendezvous::client::Event::Registered {
            rendezvous_node,
            ttl,
            namespace,
        } => {
            let namespace_key = namespace.to_string();
            state.mark_registered(rendezvous_node.to_owned(), &namespace_key);
            format!(
                "rendezvous_client registered node={rendezvous_node} namespace={namespace} ttl={ttl}"
            )
        }
        rendezvous::client::Event::RegisterFailed {
            rendezvous_node,
            namespace,
            error,
        } => {
            let namespace_key = namespace.to_string();
            state.mark_register_failed(rendezvous_node.to_owned(), &namespace_key);
            state.register_failures = state.register_failures.saturating_add(1);
            format!(
                "rendezvous_client register failed node={rendezvous_node} namespace={namespace} error={error:?}"
            )
        }
        rendezvous::client::Event::Discovered {
            rendezvous_node,
            registrations,
            cookie,
        } => {
            let completed_namespace =
                state.complete_discover_for_peer(rendezvous_node.to_owned(), cookie.clone());
            let mut learned = 0usize;
            for registration in registrations {
                let peer = registration.record.peer_id();
                for addr in peer_record_addrs(registration) {
                    add_peer_address_to_discovery(swarm, peer, addr.clone());
                    peer_cache_writes.record_seen(peer, addr.clone());
                    learned = learned.saturating_add(1);
                }
                state.record_discovered_peer(peer);
            }
            match completed_namespace {
                Some(namespace) => format!(
                    "rendezvous_client discovered {} registrations from {rendezvous_node} namespace={namespace}; learned {learned} addrs",
                    registrations.len()
                ),
                None => format!(
                    "rendezvous_client discovered {} registrations from {rendezvous_node}; learned {learned} addrs",
                    registrations.len()
                ),
            }
        }
        rendezvous::client::Event::DiscoverFailed {
            rendezvous_node,
            namespace,
            error,
        } => {
            let namespace_key = namespace.as_ref().map(ToString::to_string);
            state.fail_discover(rendezvous_node.to_owned(), namespace_key.as_deref());
            state.discover_failures = state.discover_failures.saturating_add(1);
            format!(
                "rendezvous_client discover failed node={rendezvous_node} namespace={namespace:?} error={error:?}"
            )
        }
        rendezvous::client::Event::Expired { peer } => {
            state.discovered_peers.remove(peer);
            format!("rendezvous_client discovered registration expired peer={peer}")
        }
    }
}

pub fn on_rendezvous_server_event(
    event: &rendezvous::server::Event,
    state: &mut RendezvousState,
) -> String {
    match event {
        rendezvous::server::Event::DiscoverServed {
            enquirer,
            registrations,
        } => {
            state.server_discoveries_served = state.server_discoveries_served.saturating_add(1);
            format!(
                "rendezvous_server discover served enquirer={enquirer} registrations={}",
                registrations.len()
            )
        }
        rendezvous::server::Event::DiscoverNotServed { enquirer, error } => {
            state.server_errors = state.server_errors.saturating_add(1);
            format!("rendezvous_server discover not served enquirer={enquirer} error={error:?}")
        }
        rendezvous::server::Event::PeerRegistered { peer, registration } => {
            state.server_registrations = state.server_registrations.saturating_add(1);
            format!(
                "rendezvous_server peer registered peer={peer} namespace={} ttl={}",
                registration.namespace, registration.ttl
            )
        }
        rendezvous::server::Event::PeerNotRegistered {
            peer,
            namespace,
            error,
        } => {
            state.server_errors = state.server_errors.saturating_add(1);
            format!(
                "rendezvous_server peer not registered peer={peer} namespace={namespace} error={error:?}"
            )
        }
        rendezvous::server::Event::PeerUnregistered { peer, namespace } => {
            state.server_registrations = state.server_registrations.saturating_sub(1);
            format!("rendezvous_server peer unregistered peer={peer} namespace={namespace}")
        }
        rendezvous::server::Event::RegistrationExpired(registration) => {
            state.server_registrations = state.server_registrations.saturating_sub(1);
            format!(
                "rendezvous_server registration expired peer={} namespace={}",
                registration.record.peer_id(),
                registration.namespace
            )
        }
    }
}

pub fn add_peer_address_to_discovery(
    swarm: &mut Swarm<MeshBehaviour>,
    peer: PeerId,
    addr: Multiaddr,
) {
    swarm
        .behaviour_mut()
        .kademlia
        .add_address(&peer, addr.clone());
    swarm.add_peer_address(peer, addr);
}

fn on_identify_event(
    swarm: &mut Swarm<MeshBehaviour>,
    event: &identify::Event,
    peer_cache_writes: &mut PeerCacheWriteBatch,
    peer_book: &mut PeerBook,
    identify_addresses: &mut IdentifyAddressState,
) {
    if let identify::Event::Received { peer_id, info, .. } = event {
        let known_record = peer_book.record(peer_id);
        let distribute_to_swarm = known_record.is_some();
        let persist_as_application_peer = known_record.is_some_and(|record| {
            !record.namespaces.is_empty()
                || record.sources.iter().any(|source| {
                    matches!(
                        source,
                        PeerSource::Manual
                            | PeerSource::PeerCache
                            | PeerSource::DhtProvider
                            | PeerSource::Rendezvous
                            | PeerSource::PublicRendezvous
                    )
                })
        });
        for addr in info.listen_addrs.iter().take(MAX_IDENTIFY_ADDRS_PER_PEER) {
            identify_addresses.record(swarm, *peer_id, addr.clone());
            if distribute_to_swarm {
                swarm.add_peer_address(*peer_id, addr.clone());
                peer_book.record_addr(*peer_id, addr.clone(), PeerSource::Connected);
            }
            if persist_as_application_peer {
                peer_cache_writes.record_seen(*peer_id, addr.clone());
            }
        }
    }
}

pub fn extract_p2p_peer_id(addr: &Multiaddr) -> Option<PeerId> {
    let mut peer = None;
    for protocol in addr.iter() {
        if let libp2p::multiaddr::Protocol::P2p(id) = protocol {
            peer = Some(id);
        }
    }
    peer
}
