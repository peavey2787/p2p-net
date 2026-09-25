//! Local subscriptions handed out by `NodeHandle`: node lifecycle events and
//! topic-filtered application messages.

use tokio::sync::broadcast;

use super::{AppMessage, NodeEvent};
use crate::common::error::NetError;

pub struct NodeEventSubscription {
    receiver: broadcast::Receiver<NodeEvent>,
}

impl NodeEventSubscription {
    #[must_use]
    pub fn new(receiver: broadcast::Receiver<NodeEvent>) -> Self {
        Self { receiver }
    }

    pub async fn recv(&mut self) -> Result<NodeEvent, NetError> {
        self.receiver
            .recv()
            .await
            .map_err(|err| NetError::ApiCommand(format!("node event subscription failed: {err}")))
    }
}

/// Topic-filtered local subscription returned by `NodeHandle::subscribe`.
pub struct AppSubscription {
    topic: String,
    receiver: broadcast::Receiver<AppMessage>,
}

impl AppSubscription {
    #[must_use]
    pub fn new(topic: String, receiver: broadcast::Receiver<AppMessage>) -> Self {
        Self { topic, receiver }
    }

    #[must_use]
    pub fn topic(&self) -> &str {
        &self.topic
    }

    pub async fn recv(&mut self) -> Result<AppMessage, broadcast::error::RecvError> {
        loop {
            let message = self.receiver.recv().await?;
            if message.topic == self.topic {
                return Ok(message);
            }
        }
    }
}
