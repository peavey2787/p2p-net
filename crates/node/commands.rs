use std::sync::Arc;

use libp2p::gossipsub::TopicHash;
use libp2p::{PeerId, Swarm};
use tokio::sync::Mutex;

use crate::api::{app_ident_topic, encode_app_message, AppMessage, PeerInfo};
use crate::common::error::NetError;
use crate::stack::MeshBehaviour;

use super::handle::NodeCommand;
use super::types::NodeSnapshot;

pub(crate) async fn handle_node_command(
    command: NodeCommand,
    swarm: &mut Swarm<MeshBehaviour>,
    local_peer: PeerId,
    network_id: u32,
    app_topic_hashes: &mut Vec<TopicHash>,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
) {
    let (success, sent_app_message) = match command {
        NodeCommand::ConnectPeer { addr, reply } => {
            let result = swarm.dial(addr.clone()).map_err(|err| NetError::Dial {
                target: addr.to_string(),
                reason: err.to_string(),
            });
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
            let result = publish_app_message(
                swarm,
                network_id,
                local_peer,
                Some(peer_id),
                topic,
                payload,
            );
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
            let result = subscribe_app_topic(swarm, network_id, topic, app_topic_hashes, snapshot)
                .await;
            let success = result.is_ok();
            let _ = reply.send(result);
            (success, false)
        }
        NodeCommand::GetPeers(reply) => {
            let peers = swarm
                .connected_peers()
                .copied()
                .map(PeerInfo::connected)
                .collect::<Vec<_>>();
            let _ = reply.send(Ok(peers));
            (true, false)
        }
    };

    let mut guard = snapshot.lock().await;
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
