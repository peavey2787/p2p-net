use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use libp2p::{Multiaddr, PeerId};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};

use crate::api::{
    AppMessage, AppSubscription, LocalNodeBinding, NodeEvent, NodeEventSubscription, NodeMetrics,
    P2PNode, PeerInfo,
};
use crate::common::error::NetError;
use crate::connectivity::dns::{resolve_dial_multiaddrs, DnsaddrConfig};
use crate::runtime::{self, TaskHandle};

use super::snapshot::NodeSnapshot;

const NODE_COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const NODE_SHUTDOWN_GRACE: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub struct NodeHandle {
    pub peer_id: PeerId,
    pub snapshot: Arc<Mutex<NodeSnapshot>>,
    pub(crate) snapshot_revision: Arc<AtomicU64>,
    pub(crate) command_tx: mpsc::Sender<NodeCommand>,
    pub(crate) messages_tx: broadcast::Sender<AppMessage>,
    pub(crate) events_tx: broadcast::Sender<NodeEvent>,
    pub(crate) shutdown_tx: mpsc::Sender<()>,
    pub(crate) task: Arc<Mutex<Option<TaskHandle>>>,
    pub(crate) dnsaddr: DnsaddrConfig,
}

impl NodeHandle {
    /// Monotonic dashboard/state revision. Readers can poll this inexpensive
    /// counter before locking/cloning the full snapshot.
    pub fn snapshot_revision(&self) -> u64 {
        self.snapshot_revision.load(Ordering::Relaxed)
    }

    /// Dial a concrete peer multiaddr. The address should include `/p2p/<PeerId>`
    /// when the remote peer identity is known.
    pub async fn connect_peer(&self, addr: Multiaddr) -> Result<(), NetError> {
        let resolved = resolve_dial_multiaddrs(&addr, &self.dnsaddr).await?;
        let mut last_error = None;
        for candidate in resolved {
            match self
                .request(|reply| NodeCommand::ConnectPeer {
                    addr: candidate,
                    reply,
                })
                .await
            {
                Ok(()) => return Ok(()),
                Err(err) => last_error = Some(err),
            }
        }
        Err(last_error.unwrap_or_else(|| NetError::Dial {
            target: addr.to_string(),
            reason: "DNS resolution returned no dialable addresses".to_string(),
        }))
    }

    /// Close active connections to a peer id.
    pub async fn disconnect_peer(&self, peer_id: PeerId) -> Result<(), NetError> {
        self.request(|reply| NodeCommand::DisconnectPeer { peer_id, reply })
            .await
    }

    /// Send an addressed application message on a topic. The receiving app should
    /// call `subscribe` for the same topic and then read from the returned `AppSubscription`.
    pub async fn send_message(
        &self,
        peer_id: PeerId,
        topic: impl Into<String>,
        payload: Vec<u8>,
    ) -> Result<(), NetError> {
        self.request(|reply| NodeCommand::SendMessage {
            peer_id,
            topic: topic.into(),
            payload,
            reply,
        })
        .await
    }

    /// Broadcast an application message to all subscribed peers on a topic.
    pub async fn broadcast(
        &self,
        topic: impl Into<String>,
        payload: Vec<u8>,
    ) -> Result<(), NetError> {
        self.request(|reply| NodeCommand::Broadcast {
            topic: topic.into(),
            payload,
            reply,
        })
        .await
    }

    /// Subscribe the swarm to an application topic and return a topic-filtered
    /// local `AppSubscription` for incoming messages delivered to this process.
    pub async fn subscribe(&self, topic: impl Into<String>) -> Result<AppSubscription, NetError> {
        let topic = topic.into();
        self.request(|reply| NodeCommand::Subscribe {
            topic: topic.clone(),
            reply,
        })
        .await?;
        Ok(AppSubscription::new(topic, self.messages_tx.subscribe()))
    }

    /// Return peers known to the local node, including connected, cached, configured, rendezvous-discovered, DHT-provider-discovered, and relay-discovered peers when available.
    pub async fn get_peers(&self) -> Result<Vec<PeerInfo>, NetError> {
        self.request(NodeCommand::GetPeers).await
    }

    /// Return runtime-owned infrastructure metrics. Passing a peer id filters
    /// per-peer bandwidth details to that peer to avoid large result payloads.
    pub async fn get_metrics(&self, peer_id: Option<PeerId>) -> Result<NodeMetrics, NetError> {
        self.request(|reply| NodeCommand::GetMetrics { peer_id, reply })
            .await
    }

    /// Return the stable peer id plus currently advertised direct/relay dial addresses.
    /// ICE, candidate pairs, and swarm internals never cross this boundary.
    pub async fn local_binding(&self) -> LocalNodeBinding {
        let snapshot = self.snapshot.lock().await;
        let mut dial_addresses = snapshot.public_direct_listen_addresses.clone();
        dial_addresses.extend(snapshot.relayed_listen_addresses.iter().cloned());
        dial_addresses.sort();
        dial_addresses.dedup();
        LocalNodeBinding {
            peer_id: self.peer_id.to_string(),
            dial_addresses,
        }
    }

    /// Subscribe to coarse transport-neutral node events.
    #[must_use]
    pub fn subscribe_events(&self) -> NodeEventSubscription {
        NodeEventSubscription::new(self.events_tx.subscribe())
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) async fn browser_wake(&self) -> Result<(), NetError> {
        self.request(NodeCommand::Wake).await
    }

    /// Request shutdown and wait for the swarm task to exit.
    pub async fn shutdown(&self) {
        // Shutdown must never wait for room in the signal channel. This is
        // especially important for console-close/logoff handlers, where the OS
        // gives the process a short cleanup window before terminating it.
        let _ = self.shutdown_tx.try_send(());
        if let Some(task) = self.task.lock().await.take() {
            task.shutdown(NODE_SHUTDOWN_GRACE).await;
        }
    }

    async fn request<T>(
        &self,
        build: impl FnOnce(oneshot::Sender<Result<T, NetError>>) -> NodeCommand,
    ) -> Result<T, NetError> {
        let (reply, response) = oneshot::channel();
        self.command_tx
            .send(build(reply))
            .await
            .map_err(|_| NetError::ApiCommand("node command channel is closed".to_string()))?;
        runtime::timeout(NODE_COMMAND_TIMEOUT, response)
            .await
            .map_err(|_| {
                NetError::ApiCommand(format!(
                    "node command response timed out after {}s",
                    NODE_COMMAND_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|_| NetError::ApiCommand("node command response was dropped".to_string()))?
    }
}

impl P2PNode for NodeHandle {
    async fn connect_peer(&self, addr: Multiaddr) -> Result<(), NetError> {
        NodeHandle::connect_peer(self, addr).await
    }
    async fn disconnect_peer(&self, peer_id: PeerId) -> Result<(), NetError> {
        NodeHandle::disconnect_peer(self, peer_id).await
    }
    async fn send_message<'a>(
        &'a self,
        peer_id: PeerId,
        topic: &'a str,
        payload: Vec<u8>,
    ) -> Result<(), NetError> {
        NodeHandle::send_message(self, peer_id, topic.to_string(), payload).await
    }
    async fn broadcast<'a>(&'a self, topic: &'a str, payload: Vec<u8>) -> Result<(), NetError> {
        NodeHandle::broadcast(self, topic.to_string(), payload).await
    }
    async fn subscribe<'a>(&'a self, topic: &'a str) -> Result<AppSubscription, NetError> {
        NodeHandle::subscribe(self, topic.to_string()).await
    }
    async fn get_peers(&self) -> Result<Vec<PeerInfo>, NetError> {
        NodeHandle::get_peers(self).await
    }
    async fn get_metrics(&self, peer_id: Option<PeerId>) -> Result<NodeMetrics, NetError> {
        NodeHandle::get_metrics(self, peer_id).await
    }
}

pub(crate) enum NodeCommand {
    ConnectPeer {
        addr: Multiaddr,
        reply: oneshot::Sender<Result<(), NetError>>,
    },
    DisconnectPeer {
        peer_id: PeerId,
        reply: oneshot::Sender<Result<(), NetError>>,
    },
    SendMessage {
        peer_id: PeerId,
        topic: String,
        payload: Vec<u8>,
        reply: oneshot::Sender<Result<(), NetError>>,
    },
    Broadcast {
        topic: String,
        payload: Vec<u8>,
        reply: oneshot::Sender<Result<(), NetError>>,
    },
    Subscribe {
        topic: String,
        reply: oneshot::Sender<Result<(), NetError>>,
    },
    GetPeers(oneshot::Sender<Result<Vec<PeerInfo>, NetError>>),
    GetMetrics {
        peer_id: Option<PeerId>,
        reply: oneshot::Sender<Result<NodeMetrics, NetError>>,
    },
    /// Browser page-lifecycle wake (visibility/online events).
    #[cfg(target_arch = "wasm32")]
    Wake(oneshot::Sender<Result<(), NetError>>),
}
