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
use sha2::{Digest, Sha256};

use crate::connectivity::discovery::DiscoveryConfig;
use crate::stack::MeshBehaviour;

const MAX_TRACKED_PROVIDER_PEERS: usize = 2048;
const PROVIDER_ADDR_LOOKUP_COOLDOWN_SECS: u64 = 30;
const AUTO_CONNECT_RETRY_COOLDOWN_SECS: u64 = 5;
const AUTO_CONNECT_RETRY_WINDOW_SECS: u64 = 300;
const MAX_AUTO_CONNECT_ATTEMPTS_PER_WINDOW: u32 = 8;
const DHT_PROVIDER_KEY_REPLICAS: u8 = 3;
const DHT_PROVIDER_ANCHOR_PREFIX_BYTES: usize = 2;
const DHT_PROVIDER_ANCHOR_MAX_ATTEMPTS: u32 = 1 << 20;
const DHT_PROVIDER_ANCHOR_CONTEXT: &str = "p2p-net.dht.provider.anchor.v1";
const MULTIHASH_SHA2_256_CODE: u8 = 0x12;
const SHA2_256_DIGEST_BYTES: u8 = 32;

mod state;
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

pub fn dht_record_key(namespace: &str) -> kad::RecordKey {
    provider_multihash_key(namespace.as_bytes())
}

fn dht_record_replica_key(namespace: &str, replica: u8) -> kad::RecordKey {
    if replica == 0 {
        dht_record_key(namespace)
    } else {
        provider_multihash_key(format!("{namespace}/provider-replica/{replica}").as_bytes())
    }
}

fn dht_record_replica_tracking_key(namespace: &str, replica: u8) -> String {
    if replica == 0 {
        namespace.to_string()
    } else {
        format!("{namespace}:provider-replica:{replica}")
    }
}

fn dht_provider_keys(
    namespace: &str,
    discovery_cfg: &DiscoveryConfig,
) -> Vec<(String, kad::RecordKey)> {
    let public_anchors = discovery_cfg
        .public_bootstrap
        .mode
        .is_enabled()
        .then(|| {
            discovery_cfg
                .public_bootstrap
                .bootstrap_seed_peers
                .iter()
                .filter_map(|addr| {
                    addr.rsplit_once("/p2p/")
                        .and_then(|(_, peer)| peer.parse::<PeerId>().ok())
                })
                .take(usize::from(DHT_PROVIDER_KEY_REPLICAS))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if !public_anchors.is_empty() {
        return public_anchors
            .into_iter()
            .enumerate()
            .map(|(replica, anchor)| {
                (
                    format!("{namespace}:provider-anchor:{replica}"),
                    anchored_provider_key(namespace, &anchor, replica as u8),
                )
            })
            .collect();
    }

    (0..DHT_PROVIDER_KEY_REPLICAS)
        .map(|replica| {
            (
                dht_record_replica_tracking_key(namespace, replica),
                dht_record_replica_key(namespace, replica),
            )
        })
        .collect()
}

fn anchored_provider_key(namespace: &str, anchor: &PeerId, replica: u8) -> kad::RecordKey {
    let anchor_bytes = anchor.to_bytes();
    let target = Sha256::digest(&anchor_bytes);
    let mut material = Vec::with_capacity(
        DHT_PROVIDER_ANCHOR_CONTEXT.len()
            + namespace.len()
            + anchor_bytes.len()
            + std::mem::size_of::<u32>()
            + 2,
    );
    material.extend_from_slice(DHT_PROVIDER_ANCHOR_CONTEXT.as_bytes());
    material.push(0);
    material.extend_from_slice(namespace.as_bytes());
    material.push(replica);
    material.extend_from_slice(&anchor_bytes);
    let counter_offset = material.len();
    material.extend_from_slice(&0_u32.to_be_bytes());

    for counter in 0..DHT_PROVIDER_ANCHOR_MAX_ATTEMPTS {
        material[counter_offset..].copy_from_slice(&counter.to_be_bytes());
        let digest = Sha256::digest(&material);
        let mut candidate = [0_u8; 34];
        candidate[0] = MULTIHASH_SHA2_256_CODE;
        candidate[1] = SHA2_256_DIGEST_BYTES;
        candidate[2..].copy_from_slice(&digest);
        let location = Sha256::digest(candidate);
        if location[..DHT_PROVIDER_ANCHOR_PREFIX_BYTES]
            == target[..DHT_PROVIDER_ANCHOR_PREFIX_BYTES]
        {
            return kad::RecordKey::new(&candidate);
        }
    }
    dht_record_replica_key(namespace, replica)
}

fn provider_multihash_key(material: impl AsRef<[u8]>) -> kad::RecordKey {
    let digest = Sha256::digest(material.as_ref());
    let mut key = Vec::with_capacity(2 + digest.len());
    // Public IPFS/libp2p DHT implementations commonly validate provider keys
    // as content multihashes.
    key.push(MULTIHASH_SHA2_256_CODE);
    key.push(SHA2_256_DIGEST_BYTES);
    key.extend_from_slice(&digest);
    kad::RecordKey::new(&key)
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

    for namespace in namespaces
        .into_iter()
        .take(dht_cfg.max_namespaces_per_refresh)
    {
        let remote_provider_connected = state
            .discovered_provider_peers
            .keys()
            .any(|peer| peer != swarm.local_peer_id() && swarm.is_connected(peer));
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
        kad::Event::RoutingUpdated {
            peer: _,
            addresses: _,
            ..
        } => None,
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
