//! Kademlia provider-record discovery for application namespaces.
//!
//! This layer lets a node announce and search for hashed app discovery
//! namespaces through the DHT. Consumer app defaults keep DHT provider
//! discovery running alongside rendezvous so public rendezvous and DHT
//! resurrection can complement each other.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::common::error::config_error;
use libp2p::kad::{self, QueryId};
use libp2p::swarm::Swarm;
use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};

use crate::connectivity::discovery::DiscoveryConfig;
use crate::stack::MeshBehaviour;

const MAX_TRACKED_PROVIDER_PEERS: usize = 2048;
const PROVIDER_ADDR_LOOKUP_COOLDOWN_SECS: u64 = 30;
const AUTO_CONNECT_RETRY_COOLDOWN_SECS: u64 = 5;
const AUTO_CONNECT_RETRY_WINDOW_SECS: u64 = 300;
const MAX_AUTO_CONNECT_ATTEMPTS_PER_WINDOW: u32 = 8;

mod keys;
mod state;
use keys::dht_provider_keys;
pub use keys::dht_record_key;
pub use state::DhtProviderState;

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
    /// Minimum seconds between periodic DHT namespace refreshes.
    pub refresh_interval_secs: u64,
    /// Optional libp2p Kademlia routing-table bootstrap cadence. `None` disables
    /// the built-in periodic bootstrap; explicit startup/event refreshes remain.
    pub periodic_bootstrap_interval_secs: Option<u64>,
    /// Maximum peers an iterative Kademlia query waits on concurrently.
    pub query_parallelism: usize,
    /// Redundant provider keys used per namespace. One retains interoperability
    /// through replica zero while reducing announce/query and key-derivation work.
    pub provider_key_replicas: usize,
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
            refresh_interval_secs: 300,
            periodic_bootstrap_interval_secs: Some(300),
            query_parallelism: 3,
            provider_key_replicas: 3,
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
        if self.query_parallelism == 0 {
            return Err(config_error(
                "discovery.dht.query_parallelism must be at least 1",
            ));
        }
        if !(1..=3).contains(&self.provider_key_replicas) {
            return Err(config_error(
                "discovery.dht.provider_key_replicas must be between 1 and 3",
            ));
        }
        if self.periodic_bootstrap_interval_secs == Some(0) {
            return Err(config_error(
                "discovery.dht.periodic_bootstrap_interval_secs must be null or at least 1",
            ));
        }
        if self.refresh_interval_secs == 0 {
            return Err(config_error(
                "discovery.dht.refresh_interval_secs must be at least 1",
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

pub fn start_dht_namespace_discovery(
    swarm: &mut Swarm<MeshBehaviour>,
    network_id: u32,
    discovery_cfg: &DiscoveryConfig,
    rendezvous_peer_count: usize,
    state: &mut DhtProviderState,
) -> DhtNamespacePlan {
    start_dht_namespace_discovery_with_interval(
        swarm,
        network_id,
        discovery_cfg,
        rendezvous_peer_count,
        state,
        discovery_cfg.dht.refresh_interval_secs,
    )
}

/// Force one namespace refresh after a material reachability change such as
/// learning a new public external address. This intentionally resets only the
/// announce/query refresh timestamps; query bookkeeping and discovered peers
/// remain intact.
pub(crate) fn start_dht_namespace_discovery_immediate(
    swarm: &mut Swarm<MeshBehaviour>,
    network_id: u32,
    discovery_cfg: &DiscoveryConfig,
    rendezvous_peer_count: usize,
    state: &mut DhtProviderState,
) -> DhtNamespacePlan {
    state.provider_announce_started_unix_secs.clear();
    state.provider_query_started_unix_secs.clear();
    start_dht_namespace_discovery_with_interval(
        swarm,
        network_id,
        discovery_cfg,
        rendezvous_peer_count,
        state,
        1,
    )
}

pub fn start_dht_namespace_discovery_with_interval(
    swarm: &mut Swarm<MeshBehaviour>,
    network_id: u32,
    discovery_cfg: &DiscoveryConfig,
    rendezvous_peer_count: usize,
    state: &mut DhtProviderState,
    refresh_interval_secs: u64,
) -> DhtNamespacePlan {
    let dht_cfg = &discovery_cfg.dht;
    let now = now_unix_secs();
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
    let remote_provider_connected = state
        .discovered_provider_peers
        .keys()
        .any(|peer| peer != swarm.local_peer_id() && swarm.is_connected(peer));

    for namespace in namespaces
        .into_iter()
        .take(dht_cfg.max_namespaces_per_refresh)
    {
        let query_refresh_interval = if remote_provider_connected {
            dht_cfg.refresh_interval_secs.max(1)
        } else {
            refresh_interval_secs.max(1)
        };
        // During the bounded startup window, re-publish after relay/public
        // reachability has had time to become confirmed. The first publication
        // can otherwise capture no useful external address and remain stale
        // for the full steady-state refresh interval.
        let announce_refresh_interval = refresh_interval_secs.max(1);

        for (replica, (tracking_key, key)) in state
            .provider_keys(&namespace, discovery_cfg)
            .into_iter()
            .enumerate()
        {
            if dht_cfg.announce
                && state.should_announce_key(
                    &tracking_key,
                    &namespace,
                    now,
                    announce_refresh_interval,
                )
            {
                state.announce_attempts = state.announce_attempts.saturating_add(1);
                match swarm.behaviour_mut().kademlia.start_providing(key.clone()) {
                    Ok(query_id) => {
                        state.track_start_providing_for_key(query_id, &namespace, &tracking_key);
                        state
                            .provider_announce_started_unix_secs
                            .insert(tracking_key.clone(), now);
                        plan.announce_attempts = plan.announce_attempts.saturating_add(1);
                    }
                    Err(err) => {
                        state.announce_failures = state.announce_failures.saturating_add(1);
                        plan.errors.push(format!(
                            "dht provider announce failed namespace={namespace} replica={replica}: {err}"
                        ));
                    }
                }
            }

            if dht_cfg.should_discover(rendezvous_peer_count)
                && state.should_query_key(&tracking_key, now, query_refresh_interval)
            {
                let query_id = swarm.behaviour_mut().kademlia.get_providers(key);
                state.track_get_providers_for_key(query_id, &namespace, &tracking_key);
                state
                    .provider_query_started_unix_secs
                    .insert(tracking_key, now);
                state.provider_queries = state.provider_queries.saturating_add(1);
                plan.provider_queries = plan.provider_queries.saturating_add(1);
            }
        }
    }

    plan
}

pub fn on_kademlia_event(
    swarm: &mut Swarm<MeshBehaviour>,
    event: &kad::Event,
    state: &mut DhtProviderState,
) -> Option<String> {
    match event {
        kad::Event::OutboundQueryProgressed {
            id, result, step, ..
        } => match result {
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
                    if state.record_provider_peer(*provider, namespace.clone()) {
                        learned = learned.saturating_add(1);
                    }
                }
                state.provider_records_found = state.provider_records_found.saturating_add(learned);
                if step.last {
                    state.complete_get_providers(id);
                    state.provider_queries_finished =
                        state.provider_queries_finished.saturating_add(1);
                }
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
            kad::QueryResult::GetClosestPeers(Ok(result)) => {
                let peer = state.complete_provider_addr_lookup(id)?;
                let peer_addr_count = result
                    .peers
                    .iter()
                    .find(|info| info.peer_id == peer)
                    .map(|info| info.addrs.len())
                    .unwrap_or_default();
                Some(format!(
                    "dht provider address lookup finished peer={peer} target_addrs={peer_addr_count} closest_peers={}",
                    result.peers.len()
                ))
            }
            kad::QueryResult::GetClosestPeers(Err(err)) => {
                let peer = state.complete_provider_addr_lookup(id)?;
                Some(format!(
                    "dht provider address lookup failed peer={peer} error={err:?}"
                ))
            }
            _ => None,
        },
        kad::Event::RoutingUpdated { .. } => None,
        kad::Event::RoutablePeer { peer, address }
        | kad::Event::PendingRoutablePeer { peer, address } => {
            add_peer_addr_to_kademlia(swarm, peer, address.clone());
            None
        }
        _ => None,
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
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

#[cfg(test)]
mod tests;
