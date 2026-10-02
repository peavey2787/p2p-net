//! Regression test for messages stranded on a peer's second connection.
//!
//! Gossipsub sends its subscriptions only on the first connection to a peer, so
//! a later connection's handler has no outbound stream. Once the first
//! connection closes (for example a relayed connection replaced by a direct
//! one), every message for that peer is queued for the remaining handler. That
//! handler must be woken by the queue itself: upstream 0.50.0 only checked
//! `is_empty()` without registering a waker, so messages sat until some
//! unrelated event polled the connection and expired before it did.

use std::time::Duration;

use futures::StreamExt;
use libp2p_core::Multiaddr;
use libp2p_gossipsub as gossipsub;
use libp2p_swarm::{
    Swarm, SwarmEvent,
    dial_opts::{DialOpts, PeerCondition},
};
use libp2p_swarm_test::SwarmExt as _;

fn build_swarm() -> Swarm<gossipsub::Behaviour> {
    Swarm::new_ephemeral_tokio(|keypair| {
        let config = gossipsub::ConfigBuilder::default()
            // Keep the behaviour's own timers from waking connections during the
            // test window: only the message queue may wake the handler.
            .heartbeat_initial_delay(Duration::from_secs(3600))
            .heartbeat_interval(Duration::from_secs(3600))
            .publish_queue_duration(Duration::from_secs(5))
            .build()
            .expect("valid gossipsub config");
        gossipsub::Behaviour::new(gossipsub::MessageAuthenticity::Signed(keypair), config)
            .expect("valid gossipsub behaviour")
    })
}

#[tokio::test]
async fn message_reaches_peer_after_first_connection_closes() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let topic = gossipsub::IdentTopic::new("migration");
    let mut sender = build_swarm();
    let mut receiver = build_swarm();
    sender.behaviour_mut().subscribe(&topic).unwrap();
    receiver.behaviour_mut().subscribe(&topic).unwrap();

    let (addr, _) = receiver.listen().with_memory_addr_external().await;

    // First connection: carries the subscription exchange.
    let first_connection = dial(&mut sender, &mut receiver, addr.clone()).await;
    wait_for_subscription(&mut sender, &mut receiver).await;

    // Second connection: its handlers get no outbound gossipsub stream.
    dial(&mut sender, &mut receiver, addr).await;
    // Let both connections go idle so nothing but the queue can wake them.
    settle(&mut sender, &mut receiver).await;

    // Drop the first connection; the second one is now the only path.
    assert!(sender.close_connection(first_connection));
    drive_until_closed(&mut sender, &mut receiver, first_connection).await;
    settle(&mut sender, &mut receiver).await;

    sender
        .behaviour_mut()
        .publish(topic.clone(), b"after migration".to_vec())
        .expect("receiver is still a topic peer");

    let delivered = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            tokio::select! {
                event = receiver.select_next_some() => {
                    if let SwarmEvent::Behaviour(gossipsub::Event::Message { message, .. }) = event {
                        return message.data;
                    }
                }
                _ = sender.select_next_some() => {}
            }
        }
    })
    .await
    .expect("message stranded on the remaining connection");
    assert_eq!(delivered, b"after migration");
}

async fn wait_for_subscription(
    sender: &mut Swarm<gossipsub::Behaviour>,
    receiver: &mut Swarm<gossipsub::Behaviour>,
) {
    let receiver_id = *receiver.local_peer_id();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if sender
                .behaviour()
                .all_peers()
                .any(|(peer, topics)| *peer == receiver_id && !topics.is_empty())
            {
                return;
            }
            tokio::select! {
                _ = sender.select_next_some() => {}
                _ = receiver.select_next_some() => {}
            }
        }
    })
    .await
    .expect("subscriptions exchanged on the first connection");
}

/// Dials the receiver over a new connection and returns its id.
async fn dial(
    sender: &mut Swarm<gossipsub::Behaviour>,
    receiver: &mut Swarm<gossipsub::Behaviour>,
    addr: Multiaddr,
) -> libp2p_swarm::ConnectionId {
    let dial = DialOpts::peer_id(*receiver.local_peer_id())
        .addresses(vec![addr])
        .condition(PeerCondition::Always)
        .build();
    let id = dial.connection_id();
    sender.dial(dial).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            tokio::select! {
                event = sender.select_next_some() => {
                    if let SwarmEvent::ConnectionEstablished { connection_id, .. } = event
                        && connection_id == id
                    {
                        return;
                    }
                }
                _ = receiver.select_next_some() => {}
            }
        }
    })
    .await
    .expect("connection established");
    id
}

async fn settle(
    sender: &mut Swarm<gossipsub::Behaviour>,
    receiver: &mut Swarm<gossipsub::Behaviour>,
) {
    let _ = tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            tokio::select! {
                _ = sender.select_next_some() => {}
                _ = receiver.select_next_some() => {}
            }
        }
    })
    .await;
}

async fn drive_until_closed(
    sender: &mut Swarm<gossipsub::Behaviour>,
    receiver: &mut Swarm<gossipsub::Behaviour>,
    closed: libp2p_swarm::ConnectionId,
) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            tokio::select! {
                event = sender.select_next_some() => {
                    if let SwarmEvent::ConnectionClosed { connection_id, .. } = event
                        && connection_id == closed
                    {
                        return;
                    }
                }
                _ = receiver.select_next_some() => {}
            }
        }
    })
    .await
    .expect("first connection closed");
}
