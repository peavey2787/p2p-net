//! Kademlia provider-record discovery for application namespaces.
//!
//! This layer lets a node announce and search for hashed app discovery
//! namespaces through the DHT. Consumer app defaults keep DHT provider
//! discovery running alongside rendezvous so public rendezvous and DHT
//! resurrection can complement each other.

use crate::common::error::config_error;
use std::collections::{BTreeSet, HashMap, HashSet};

use libp2p::kad::{self, QueryId};
use libp2p::swarm::Swarm;
use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};

use crate::connectivity::discovery::DiscoveryConfig;
use crate::connectivity::peer_cache;
use crate::platform::NodeStorage;
use crate::stack::MeshBehaviour;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DhtDiscoveryConfig {
    /// Enable namespace announcement/discovery through Kademlia provider records.
    pub enabled: bool,
    /// Announce the local node as a provider for each derived namespace key.
    pub announce: bool,
    /// Query providers for each derived namespace key.
    pub discover: bool,
    /// Run provider lookups even when rendezvous peers are configured.
    /// Consumer defaults keep this on so DHT resurrection complements rendezvous.
    pub discover_with_rendezvous_peers: bool,
    /// Bound startup work when many app tags are configured.
    pub max_namespaces_per_refresh: usize,
}

impl Default for DhtDiscoveryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            announce: true,
            discover: true,
            discover_with_rendezvous_peers: true,
            max_namespaces_per_refresh: 16,
        }
    }
}

impl DhtDiscoveryConfig {
    pub fn validate(&self) -> Result<(), crate::common::error::NetError> {
        if self.enabled && !self.announce && !self.discover {
            return Err(config_error(
                "discovery.dht must enable announce, discover, or be disabled",
            ));
        }
        if self.max_namespaces_per_refresh == 0 {
            return Err(config_error(
                "discovery.dht.max_namespaces_per_refresh must be at least 1",
            ));
        }
        Ok(())
    }

    #[must_use]
    pub const fn should_discover(&self, rendezvous_peer_count: usize) -> bool {
        self.enabled
            && self.discover
            && (self.discover_with_rendezvous_peers || rendezvous_peer_count == 0)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DhtNamespacePlan {
    pub enabled: bool,
    pub namespace_count: usize,
    pub announce_attempts: usize,
    pub provider_queries: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DhtProviderState {
    pub announce_attempts: usize,
    pub announce_failures: usize,
    pub namespaces_announced: BTreeSet<String>,
    pub provider_queries: usize,
    pub provider_query_failures: usize,
    pub provider_records_found: usize,
    pub provider_queries_finished: usize,
    pub discovered_provider_peers: HashMap<PeerId, BTreeSet<String>>,
    pub auto_connect_attempted_peers: HashSet<PeerId>,
    pub auto_connect_waiting_for_addrs: HashSet<PeerId>,
    start_providing_queries: HashMap<QueryId, String>,
    get_provider_queries: HashMap<QueryId, String>,
}

impl DhtProviderState {
    pub fn track_start_providing(&mut self, id: QueryId, namespace: &str) {
        self.start_providing_queries
            .insert(id, namespace.to_string());
    }

    pub fn track_get_providers(&mut self, id: QueryId, namespace: &str) {
        self.get_provider_queries.insert(id, namespace.to_string());
    }

    #[must_use]
    pub fn provider_peer_count(&self) -> usize {
        self.discovered_provider_peers.len()
    }

    pub fn mark_auto_connect_attempted(&mut self, peer: PeerId) -> bool {
        self.auto_connect_waiting_for_addrs.remove(&peer);
        self.auto_connect_attempted_peers.insert(peer)
    }

    pub fn mark_auto_connect_waiting_for_addrs(&mut self, peer: PeerId) -> bool {
        if self.auto_connect_attempted_peers.contains(&peer) {
            return false;
        }
        self.auto_connect_waiting_for_addrs.insert(peer)
    }

    pub fn mark_auto_connect_failed(&mut self, peer: &PeerId) -> bool {
        self.auto_connect_waiting_for_addrs.remove(peer);
        self.auto_connect_attempted_peers.remove(peer)
    }

    #[must_use]
    pub fn should_auto_connect_provider_result(&self, peer: &PeerId) -> bool {
        !self.auto_connect_attempted_peers.contains(peer)
            && !self.auto_connect_waiting_for_addrs.contains(peer)
    }

    #[must_use]
    pub fn should_auto_connect_after_addr_update(&self, peer: &PeerId) -> bool {
        !self.auto_connect_attempted_peers.contains(peer)
    }

    fn complete_start_providing(&mut self, id: &QueryId) -> Option<String> {
        self.start_providing_queries.remove(id)
    }

    pub fn provider_namespace(&self, id: &QueryId) -> Option<String> {
        self.get_provider_queries.get(id).cloned()
    }

    fn provider_query_inflight(&self, namespace: &str) -> bool {
        self.get_provider_queries
            .values()
            .any(|active| active == namespace)
    }

    fn complete_get_providers(&mut self, id: &QueryId) -> Option<String> {
        self.get_provider_queries.remove(id)
    }
}

pub fn dht_record_key(namespace: &str) -> kad::RecordKey {
    kad::RecordKey::new(&namespace.as_bytes())
}

pub fn start_dht_namespace_discovery(
    swarm: &mut Swarm<MeshBehaviour>,
    network_id: u32,
    discovery_cfg: &DiscoveryConfig,
    rendezvous_peer_count: usize,
    state: &mut DhtProviderState,
) -> DhtNamespacePlan {
    let dht_cfg = &discovery_cfg.dht;
    let mut plan = DhtNamespacePlan {
        enabled: dht_cfg.enabled,
        ..DhtNamespacePlan::default()
    };

    if !dht_cfg.enabled {
        return plan;
    }

    let namespaces = match discovery_cfg.rendezvous_namespaces(network_id) {
        Ok(namespaces) => namespaces,
        Err(err) => {
            plan.errors.push(err.to_string());
            return plan;
        }
    };
    plan.namespace_count = namespaces.len();

    for namespace in namespaces
        .into_iter()
        .take(dht_cfg.max_namespaces_per_refresh)
    {
        let key = dht_record_key(&namespace);
        if dht_cfg.announce {
            state.announce_attempts = state.announce_attempts.saturating_add(1);
            match swarm.behaviour_mut().kademlia.start_providing(key.clone()) {
                Ok(query_id) => {
                    state.track_start_providing(query_id, &namespace);
                    plan.announce_attempts = plan.announce_attempts.saturating_add(1);
                }
                Err(err) => {
                    state.announce_failures = state.announce_failures.saturating_add(1);
                    plan.errors.push(format!(
                        "dht provider announce failed namespace={namespace}: {err}"
                    ));
                }
            }
        }

        if dht_cfg.should_discover(rendezvous_peer_count) {
            if state.provider_query_inflight(&namespace) {
                continue;
            }
            let query_id = swarm.behaviour_mut().kademlia.get_providers(key);
            state.track_get_providers(query_id, &namespace);
            state.provider_queries = state.provider_queries.saturating_add(1);
            plan.provider_queries = plan.provider_queries.saturating_add(1);
        }
    }

    plan
}

pub fn on_kademlia_event(
    swarm: &mut Swarm<MeshBehaviour>,
    event: &kad::Event,
    discovery_cfg: &DiscoveryConfig,
    storage: &dyn NodeStorage,
    state: &mut DhtProviderState,
) -> Option<String> {
    match event {
        kad::Event::OutboundQueryProgressed { id, result, .. } => match result {
            kad::QueryResult::StartProviding(Ok(_)) => {
                let namespace = state
                    .complete_start_providing(id)
                    .unwrap_or_else(|| "<unknown>".to_string());
                state.namespaces_announced.insert(namespace.clone());
                Some(format!(
                    "dht provider announce confirmed namespace={namespace}"
                ))
            }
            kad::QueryResult::StartProviding(Err(err)) => {
                let namespace = state
                    .complete_start_providing(id)
                    .unwrap_or_else(|| "<unknown>".to_string());
                state.announce_failures = state.announce_failures.saturating_add(1);
                Some(format!(
                    "dht provider announce failed namespace={namespace} error={err:?}"
                ))
            }
            kad::QueryResult::GetProviders(Ok(kad::GetProvidersOk::FoundProviders {
                providers,
                ..
            })) => {
                let namespace = state
                    .provider_namespace(id)
                    .unwrap_or_else(|| "<unknown>".to_string());
                let mut learned = 0usize;
                for provider in providers {
                    let inserted = state
                        .discovered_provider_peers
                        .entry(*provider)
                        .or_default()
                        .insert(namespace.clone());
                    if inserted {
                        learned = learned.saturating_add(1);
                    }
                }
                state.provider_records_found = state.provider_records_found.saturating_add(learned);
                Some(format!(
                    "dht provider lookup found namespace={namespace} providers={learned}"
                ))
            }
            kad::QueryResult::GetProviders(Ok(
                kad::GetProvidersOk::FinishedWithNoAdditionalRecord { .. },
            )) => {
                let namespace = state
                    .complete_get_providers(id)
                    .unwrap_or_else(|| "<unknown>".to_string());
                state.provider_queries_finished = state.provider_queries_finished.saturating_add(1);
                Some(format!(
                    "dht provider lookup finished namespace={namespace} discovered_peers={}",
                    state.provider_peer_count()
                ))
            }
            kad::QueryResult::GetProviders(Err(err)) => {
                let namespace = state
                    .complete_get_providers(id)
                    .unwrap_or_else(|| "<unknown>".to_string());
                state.provider_query_failures = state.provider_query_failures.saturating_add(1);
                Some(format!(
                    "dht provider lookup failed namespace={namespace} error={err:?}"
                ))
            }
            _ => None,
        },
        kad::Event::RoutingUpdated {
            peer, addresses, ..
        } => {
            for addr in addresses.iter() {
                peer_cache::record_seen_peer_addr_with_storage(discovery_cfg, peer, addr, storage);
            }
            None
        }
        kad::Event::RoutablePeer { peer, address }
        | kad::Event::PendingRoutablePeer { peer, address } => {
            add_peer_addr_to_kademlia(swarm, peer, address.clone());
            peer_cache::record_seen_peer_addr_with_storage(discovery_cfg, peer, address, storage);
            None
        }
        _ => None,
    }
}

fn add_peer_addr_to_kademlia(swarm: &mut Swarm<MeshBehaviour>, peer: &PeerId, addr: Multiaddr) {
    if addr
        .iter()
        .any(|protocol| matches!(protocol, libp2p::multiaddr::Protocol::P2p(_)))
    {
        swarm.behaviour_mut().kademlia.add_address(peer, addr);
    } else {
        swarm.behaviour_mut().kademlia.add_address(
            peer,
            addr.with(libp2p::multiaddr::Protocol::P2p(peer.to_owned())),
        );
    }
}
