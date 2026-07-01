use std::sync::Arc;

use libp2p::{Multiaddr, PeerId};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

use crate::api::{AppMessage, AppSubscription, PeerInfo};
use crate::common::error::NetError;

use super::types::NodeSnapshot;

#[derive(Clone)]
pub struct NodeHandle {
    pub peer_id: PeerId,
    pub snapshot: Arc<Mutex<NodeSnapshot>>,
    pub(crate) command_tx: mpsc::Sender<NodeCommand>,
    pub(crate) messages_tx: broadcast::Sender<AppMessage>,
    pub(crate) shutdown_tx: mpsc::Sender<()>,
    pub(crate) task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl NodeHandle {
    /// Dial a concrete peer multiaddr. The address should include `/p2p/<PeerId>`
    /// when the remote peer identity is known.
    pub async fn connect_peer(&self, addr: Multiaddr) -> Result<(), NetError> {
        self.request(|reply| NodeCommand::ConnectPeer { addr, reply }).await
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

    /// Return peers currently connected to the local swarm.
    pub async fn get_peers(&self) -> Result<Vec<PeerInfo>, NetError> {
        self.request(NodeCommand::GetPeers).await
    }

    /// Request shutdown and wait for the swarm task to exit.
    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(()).await;
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
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
        response
            .await
            .map_err(|_| NetError::ApiCommand("node command response was dropped".to_string()))?
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
}
