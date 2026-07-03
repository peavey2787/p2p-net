use std::sync::Arc;

use libp2p::gossipsub::TopicHash;
use libp2p::{PeerId, Swarm};
use tokio::sync::Mutex;

use crate::api::{app_ident_topic, encode_app_message, AppMessage, PeerSource};
use crate::common::error::NetError;
use crate::connectivity::connection_strategy::{build_connection_plan, PendingConnectionPlans};
use crate::connectivity::dcutr::DcutrPolicy;
use crate::connectivity::peer_book::PeerBook;
use crate::stack::{extract_p2p_peer_id, MeshBehaviour};

use super::dial::dial_connection_plan;
use super::handle::NodeCommand;
use super::snapshot::NodeSnapshot;

pub(crate) struct NodeCommandContext<'a> {
    pub(crate) swarm: &'a mut Swarm<MeshBehaviour>,
    pub(crate) local_peer: PeerId,
    pub(crate) network_id: u32,
    pub(crate) app_topic_hashes: &'a mut Vec<TopicHash>,
    pub(crate) snapshot: &'a Arc<Mutex<NodeSnapshot>>,
    pub(crate) peer_book: &'a mut PeerBook,
    pub(crate) pending_connections: &'a mut PendingConnectionPlans,
    pub(crate) dcutr_policy: &'a DcutrPolicy,
}

pub(crate) async fn handle_node_command(command: NodeCommand, ctx: NodeCommandContext<'_>) {
    let NodeCommandContext {
        swarm,
        local_peer,
        network_id,
        app_topic_hashes,
        snapshot,
        peer_book,
        pending_connections,
        dcutr_policy,
    } = ctx;

    let (success, sent_app_message) = match command {
        NodeCommand::ConnectPeer { addr, reply } => {
            let result = if let Some(peer) = extract_p2p_peer_id(&addr) {
                if peer == local_peer {
                    Err(NetError::Dial {
                        target: peer.to_string(),
                        reason: "refusing to dial local peer id".to_string(),
                    })
                } else {
                    peer_book.record_addr(peer, addr.clone(), PeerSource::Manual);
                    let plan = build_connection_plan(addr, peer_book, dcutr_policy);
                    dial_connection_plan(swarm, pending_connections, &plan)
                }
            } else {
                let plan = build_connection_plan(addr, peer_book, dcutr_policy);
                dial_connection_plan(swarm, pending_connections, &plan)
            };
            let success = result.is_ok();
            let _ = reply.send(result);
            (success, false)
        }
        NodeCommand::DisconnectPeer { peer_id, reply } => {
            let result = swarm.disconnect_peer_id(peer_id).map_err(|err| {
                NetError::ApiCommand(format!("disconnect_peer failed for {peer_id}: {err:?}"))
            });
            let success = result.is_ok();
            let _ = reply.send(result);
            (success, false)
        }
        NodeCommand::SendMessage {
            peer_id,
            topic,
            payload,
            reply,
        } => {
            let result =
                publish_app_message(swarm, network_id, local_peer, Some(peer_id), topic, payload);
            let success = result.is_ok();
            let _ = reply.send(result);
            (success, success)
        }
        NodeCommand::Broadcast {
            topic,
            payload,
            reply,
        } => {
            let result = publish_app_message(swarm, network_id, local_peer, None, topic, payload);
            let success = result.is_ok();
            let _ = reply.send(result);
            (success, success)
        }
        NodeCommand::Subscribe { topic, reply } => {
            let result =
                subscribe_app_topic(swarm, network_id, topic, app_topic_hashes, snapshot).await;
            let success = result.is_ok();
            let _ = reply.send(result);
            (success, false)
        }
        NodeCommand::GetPeers(reply) => {
            for peer in swarm.connected_peers().copied() {
                peer_book.record_connected(peer, None);
            }
            let peers = peer_book.peers();
            let _ = reply.send(Ok(peers));
            (true, false)
        }
    };

    let mut guard = snapshot.lock().await;
    guard.peer_book_known_peers = peer_book.len();
    guard.peer_book_discovered_peers = peer_book.discovered_count();
    guard.connection_plan_pending_peers = pending_connections.pending_count();
    guard.api_commands_processed = guard.api_commands_processed.saturating_add(1);
    if !success {
        guard.api_command_failures = guard.api_command_failures.saturating_add(1);
    }
    if sent_app_message {
        guard.app_messages_sent = guard.app_messages_sent.saturating_add(1);
    }
}

fn publish_app_message(
    swarm: &mut Swarm<MeshBehaviour>,
    network_id: u32,
    local_peer: PeerId,
    target_peer: Option<PeerId>,
    topic: String,
    payload: Vec<u8>,
) -> Result<(), NetError> {
    let topic_handle = app_ident_topic(network_id, &topic)?;
    let message = match target_peer {
        Some(peer_id) => AppMessage::addressed(network_id, topic, local_peer, peer_id, payload)?,
        None => AppMessage::broadcast(network_id, topic, local_peer, payload)?,
    };
    let wire = encode_app_message(&message)?;
    swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic_handle, wire)
        .map(|_| ())
        .map_err(|err| NetError::AppMessage {
            topic: message.topic,
            reason: err.to_string(),
        })
}

async fn subscribe_app_topic(
    swarm: &mut Swarm<MeshBehaviour>,
    network_id: u32,
    topic: String,
    app_topic_hashes: &mut Vec<TopicHash>,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
) -> Result<(), NetError> {
    let topic_handle = app_ident_topic(network_id, &topic)?;
    let topic_hash = topic_handle.hash().clone();
    swarm
        .behaviour_mut()
        .gossipsub
        .subscribe(&topic_handle)
        .map_err(|err| NetError::AppTopic {
            topic: topic.clone(),
            reason: err.to_string(),
        })?;
    if !app_topic_hashes.iter().any(|known| known == &topic_hash) {
        app_topic_hashes.push(topic_hash);
    }
    let mut guard = snapshot.lock().await;
    if !guard.app_subscriptions.iter().any(|known| known == &topic) {
        guard.app_subscriptions.push(topic);
    }
    Ok(())
}
