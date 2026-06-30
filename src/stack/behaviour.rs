use std::convert::Infallible;

use libp2p::allow_block_list::{self, AllowedPeers, BlockedPeers};
use libp2p::autonat;
use libp2p::connection_limits;
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

#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "MeshEvent")]
pub struct MeshBehaviour {
    pub connection_limits: connection_limits::Behaviour,
    pub relay_acl_blocked: Toggle<allow_block_list::Behaviour<BlockedPeers>>,
    pub relay_acl_allowed: Toggle<allow_block_list::Behaviour<AllowedPeers>>,
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub autonat: autonat::Behaviour,
    pub dcutr: dcutr::Behaviour,
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

pub fn build_behaviour(
    local_key: &libp2p::identity::Keypair,
    local_peer: PeerId,
    relay_behaviour: relay::client::Behaviour,
    network_id: u32,
    relay_cfg: &RelayServiceConfig,
    connection_limits_cfg: &ConnectionLimitsConfig,
    discovery_cfg: &DiscoveryConfig,
) -> MeshBehaviour {
    let message_id_fn = |msg: &gossipsub::Message| {
        let h = blake3::hash(&msg.data);
        gossipsub::MessageId::from(h.to_hex().to_string())
    };
    let gossip_cfg = gossipsub::ConfigBuilder::default()
        .validation_mode(gossipsub::ValidationMode::Strict)
        .validate_messages()
        .message_id_fn(message_id_fn)
        .build()
        .expect("gossipsub config");
    let gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(local_key.clone()),
        gossip_cfg,
    )
    .expect("gossipsub behaviour");

    let store = kad::store::MemoryStore::new(local_peer);
    let mut kademlia = kad::Behaviour::new(local_peer, store);
    kademlia.set_mode(Some(kad::Mode::Server));

    let identify = identify::Behaviour::new(identify::Config::new(
        format!("/p2p-net/net-{network_id}/1.0.0"),
        local_key.public(),
    ));

    let relay_server_active = relay_cfg.enabled;
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

    let connection_limits =
        connection_limits::Behaviour::new(connection_limits_cfg.to_libp2p_limits());

    let rendezvous_client = discovery_cfg
        .rendezvous
        .client_enabled
        .then(|| rendezvous::client::Behaviour::new(local_key.clone()))
        .into();
    let rendezvous_server = discovery_cfg
        .rendezvous
        .server_enabled
        .then(|| rendezvous::server::Behaviour::new(discovery_cfg.rendezvous.server_config()))
        .into();

    MeshBehaviour {
        connection_limits,
        relay_acl_blocked,
        relay_acl_allowed,
        gossipsub,
        kademlia,
        autonat: autonat::Behaviour::new(local_peer, Default::default()),
        dcutr: dcutr::Behaviour::new(local_peer),
        relay_client: relay_behaviour,
        relay_server,
        rendezvous_client,
        rendezvous_server,
        identify,
        ping: ping::Behaviour::new(ping::Config::new()),
    }
}
