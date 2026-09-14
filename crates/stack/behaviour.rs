use std::convert::Infallible;
use std::num::NonZeroUsize;
use std::time::Duration;

use libp2p::allow_block_list::{self, AllowedPeers, BlockedPeers};
use libp2p::autonat;
use libp2p::dcutr;
use libp2p::gossipsub;
use libp2p::identify;
use libp2p::kad;
use libp2p::ping;
use libp2p::relay;
use libp2p::swarm::behaviour::toggle::Toggle;
use libp2p::swarm::NetworkBehaviour;
use libp2p::PeerId;
use libp2p_rendezvous as rendezvous;

use crate::connectivity::discovery::DiscoveryConfig;
use crate::connectivity::limits::ConnectionLimitsConfig;
use crate::connectivity::relay::{RelayAccess, RelayServiceConfig};
use crate::ResolvedNodeConfig;

use super::{
    ApplicationKeepAlive, DcutrBehaviour, ExternalAddressCandidates, PrioritizedConnectionLimits,
};

const KADEMLIA_QUERY_TIMEOUT: Duration = Duration::from_secs(20);
// The IPFS DHT default (20) makes a fresh consumer wait for twenty remote
// answers per lookup while it is also publishing provider and signed-address
// records.  Three independent application provider keys at five replicas each
// retain fifteen public copies without letting startup queries occupy every
// connection slot until the 60-second acceptance deadline.
const KADEMLIA_REPLICATION_FACTOR: usize = 5;

fn autonat_config(serve_public_probes: bool) -> autonat::Config {
    let mut config = autonat::Config::default();
    if !serve_public_probes {
        // AutoNAT v1 combines its client and server in one behaviour. A normal
        // application node needs the client, but must not accept arbitrary
        // dial-back requests from the public swarm: every accepted request
        // creates a new outbound transport dial and can starve the intended
        // application peer and DCUtR of connection slots. Dedicated public
        // infrastructure roles retain the server side.
        config.throttle_clients_global_max = 0;
        config.throttle_clients_peer_max = 0;
    }
    config
}

#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "MeshEvent")]
pub struct MeshBehaviour {
    pub connection_limits: PrioritizedConnectionLimits,
    pub application_keep_alive: ApplicationKeepAlive,
    pub relay_acl_blocked: Toggle<allow_block_list::Behaviour<BlockedPeers>>,
    pub relay_acl_allowed: Toggle<allow_block_list::Behaviour<AllowedPeers>>,
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: Toggle<kad::Behaviour<kad::store::MemoryStore>>,
    pub autonat: autonat::Behaviour,
    pub dcutr: Toggle<DcutrBehaviour>,
    pub external_address_candidates: ExternalAddressCandidates,
    pub relay_client: relay::client::Behaviour,
    pub relay_server: Toggle<relay::Behaviour>,
    pub rendezvous_client: Toggle<rendezvous::client::Behaviour>,
    pub rendezvous_server: Toggle<rendezvous::server::Behaviour>,
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
}

#[derive(Debug)]
pub enum MeshEvent {
    Gossipsub(gossipsub::Event),
    Kademlia(Box<kad::Event>),
    AutoNat(autonat::Event),
    Dcutr(dcutr::Event),
    RelayClient(relay::client::Event),
    RelayServer(relay::Event),
    RendezvousClient(Box<rendezvous::client::Event>),
    RendezvousServer(Box<rendezvous::server::Event>),
    Identify(Box<identify::Event>),
    Ping(ping::Event),
}

impl From<Infallible> for MeshEvent {
    fn from(v: Infallible) -> Self {
        match v {}
    }
}

impl From<gossipsub::Event> for MeshEvent {
    fn from(v: gossipsub::Event) -> Self {
        Self::Gossipsub(v)
    }
}
impl From<kad::Event> for MeshEvent {
    fn from(v: kad::Event) -> Self {
        Self::Kademlia(Box::new(v))
    }
}
impl From<autonat::Event> for MeshEvent {
    fn from(v: autonat::Event) -> Self {
        Self::AutoNat(v)
    }
}
impl From<dcutr::Event> for MeshEvent {
    fn from(v: dcutr::Event) -> Self {
        Self::Dcutr(v)
    }
}
impl From<relay::client::Event> for MeshEvent {
    fn from(v: relay::client::Event) -> Self {
        Self::RelayClient(v)
    }
}
impl From<relay::Event> for MeshEvent {
    fn from(v: relay::Event) -> Self {
        Self::RelayServer(v)
    }
}
impl From<rendezvous::client::Event> for MeshEvent {
    fn from(v: rendezvous::client::Event) -> Self {
        Self::RendezvousClient(Box::new(v))
    }
}
impl From<rendezvous::server::Event> for MeshEvent {
    fn from(v: rendezvous::server::Event) -> Self {
        Self::RendezvousServer(Box::new(v))
    }
}
impl From<identify::Event> for MeshEvent {
    fn from(v: identify::Event) -> Self {
        Self::Identify(Box::new(v))
    }
}
impl From<ping::Event> for MeshEvent {
    fn from(v: ping::Event) -> Self {
        Self::Ping(v)
    }
}

pub struct BehaviourBuildContext<'a> {
    pub local_key: &'a libp2p::identity::Keypair,
    pub local_peer: PeerId,
    pub relay_behaviour: relay::client::Behaviour,
    pub network_id: u32,
    pub gossipsub_heartbeat_interval_secs: u64,
    pub ping_interval_secs: u64,
    pub relay_cfg: &'a RelayServiceConfig,
    pub connection_limits_cfg: &'a ConnectionLimitsConfig,
    pub discovery_cfg: &'a DiscoveryConfig,
    pub resolved_cfg: &'a ResolvedNodeConfig,
}

pub fn build_behaviour(ctx: BehaviourBuildContext<'_>) -> MeshBehaviour {
    let BehaviourBuildContext {
        local_key,
        local_peer,
        relay_behaviour,
        network_id,
        gossipsub_heartbeat_interval_secs,
        ping_interval_secs,
        relay_cfg,
        connection_limits_cfg,
        discovery_cfg,
        resolved_cfg,
    } = ctx;
    let message_id_fn = |msg: &gossipsub::Message| {
        // Bind duplicate suppression to the signed author and outer topic as
        // well as payload bytes. Hashing only `data` lets a different signer
        // replay/copy bytes first and poison the legitimate author's message ID.
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"p2p-net/gossipsub-message-id/v2\0");
        match &msg.source {
            Some(source) => {
                hasher.update(&[1]);
                let source_bytes = source.to_bytes();
                hasher.update(&(source_bytes.len() as u64).to_be_bytes());
                hasher.update(&source_bytes);
            }
            None => {
                hasher.update(&[0]);
            }
        }
        let topic_bytes = msg.topic.as_str().as_bytes();
        hasher.update(&(topic_bytes.len() as u64).to_be_bytes());
        hasher.update(topic_bytes);
        hasher.update(&(msg.data.len() as u64).to_be_bytes());
        hasher.update(&msg.data);
        let hash = hasher.finalize();
        gossipsub::MessageId::from(hash.as_bytes().to_vec())
    };
    let gossip_cfg = gossipsub::ConfigBuilder::default()
        .validation_mode(gossipsub::ValidationMode::Strict)
        .validate_messages()
        .heartbeat_interval(Duration::from_secs(gossipsub_heartbeat_interval_secs))
        .message_id_fn(message_id_fn)
        .build()
        .expect("gossipsub config");
    let gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(local_key.clone()),
        gossip_cfg,
    )
    .expect("gossipsub behaviour");

    let behaviour_policy = &resolved_cfg.enabled_behaviours;

    let store = kad::store::MemoryStore::new(local_peer);
    let mut kad_config = kad::Config::default();
    kad_config.set_query_timeout(KADEMLIA_QUERY_TIMEOUT);
    kad_config.set_replication_factor(
        NonZeroUsize::new(KADEMLIA_REPLICATION_FACTOR)
            .expect("Kademlia replication factor is non-zero"),
    );
    // Seed insertion otherwise starts a full bucket crawl immediately. The
    // scheduled provider queries can route from the seeds directly, while the
    // separately configured periodic bootstrap maintains the server's routing
    // table after startup has settled.
    kad_config.set_automatic_bootstrap_throttle(None);
    kad_config.set_periodic_bootstrap_interval(
        discovery_cfg
            .dht
            .enabled
            .then_some(discovery_cfg.dht.periodic_bootstrap_interval_secs)
            .flatten()
            .map(Duration::from_secs),
    );
    kad_config.set_parallelism(
        NonZeroUsize::new(discovery_cfg.dht.query_parallelism)
            .expect("validated DHT query parallelism"),
    );
    let mut kademlia = kad::Behaviour::with_config(local_peer, store, kad_config);
    let kad_mode = if behaviour_policy.kademlia_server {
        Some(kad::Mode::Server)
    } else if behaviour_policy.kademlia_client {
        Some(kad::Mode::Client)
    } else {
        None
    };
    kademlia.set_mode(kad_mode);

    let identify_protocol = discovery_cfg
        .application_protocol_version(network_id)
        .expect("validated discovery namespace configuration");
    let identify =
        identify::Behaviour::new(identify::Config::new(identify_protocol, local_key.public()));

    let relay_server_active = behaviour_policy.relay_server && relay_cfg.enabled;
    let serve_public_autonat_probes = matches!(
        resolved_cfg.role.as_str(),
        "relay" | "mediator" | "rendezvous" | "bootstrap"
    );
    let relay_server = relay_server_active
        .then(|| relay::Behaviour::new(local_peer, relay_cfg.to_libp2p_config()))
        .into();

    let relay_acl_blocked = relay_server_active
        .then(|| {
            let mut blocked = allow_block_list::Behaviour::<BlockedPeers>::default();
            for peer in relay_cfg.denied_peer_ids() {
                blocked.block_peer(peer);
            }
            blocked
        })
        .into();

    let relay_acl_allowed =
        if relay_server_active && matches!(relay_cfg.access, RelayAccess::AllowList) {
            let mut allowed = allow_block_list::Behaviour::<AllowedPeers>::default();
            for peer in relay_cfg.allowed_peer_ids() {
                if relay_cfg.allows_peer(&peer) {
                    allowed.allow_peer(peer);
                }
            }
            Some(allowed)
        } else {
            None
        }
        .into();

    let connection_limits = PrioritizedConnectionLimits::new(connection_limits_cfg);

    let rendezvous_client = (behaviour_policy.rendezvous_client
        && discovery_cfg.rendezvous.client_enabled)
        .then(|| rendezvous::client::Behaviour::new(local_key.clone()))
        .into();
    let rendezvous_server = (behaviour_policy.rendezvous_server
        && discovery_cfg.rendezvous.server_enabled)
        .then(|| rendezvous::server::Behaviour::new(discovery_cfg.rendezvous.server_config()))
        .into();

    MeshBehaviour {
        connection_limits,
        application_keep_alive: ApplicationKeepAlive::default(),
        relay_acl_blocked,
        relay_acl_allowed,
        gossipsub,
        kademlia: discovery_cfg.dht.enabled.then_some(kademlia).into(),
        autonat: autonat::Behaviour::new(local_peer, autonat_config(serve_public_autonat_probes)),
        dcutr: behaviour_policy
            .dcutr
            .then(|| {
                DcutrBehaviour::new(
                    local_peer,
                    resolved_cfg.dcutr_retry_interval_secs,
                    resolved_cfg.dcutr_max_attempts_per_peer,
                )
                .with_lan_candidates(discovery_cfg.lan.enabled)
            })
            .into(),
        external_address_candidates: ExternalAddressCandidates::new(),
        relay_client: relay_behaviour,
        relay_server,
        rendezvous_client,
        rendezvous_server,
        identify,
        ping: ping::Behaviour::new(
            ping::Config::new().with_interval(Duration::from_secs(ping_interval_secs)),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::autonat_config;

    #[test]
    fn application_nodes_do_not_serve_public_autonat_dial_backs() {
        let config = autonat_config(false);
        assert_eq!(config.throttle_clients_global_max, 0);
        assert_eq!(config.throttle_clients_peer_max, 0);
        assert!(config.use_connected);
    }

    #[test]
    fn relay_nodes_retain_the_autonat_server_defaults() {
        let config = autonat_config(true);
        assert!(config.throttle_clients_global_max > 0);
        assert!(config.throttle_clients_peer_max > 0);
    }
}
