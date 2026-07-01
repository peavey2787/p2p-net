use thiserror::Error;

#[derive(Debug, Error)]
pub enum NetError {
    #[error("libp2p build failed: {0}")]
    Build(String),
    #[error("listen failed for {addr}: {reason}")]
    Listen { addr: String, reason: String },
    #[error("heartbeat generation failed: {0}")]
    Heartbeat(String),
    #[error("gossip message encode/decode failed: {0}")]
    GossipCodec(String),
    #[error("config failed for {path}: {reason}")]
    Config { path: String, reason: String },
    #[error("identity key failed for {path}: {reason}")]
    Identity { path: String, reason: String },
    #[error("api command failed: {0}")]
    ApiCommand(String),
    #[error("dial failed for {target}: {reason}")]
    Dial { target: String, reason: String },
    #[error("app topic `{topic}` is invalid: {reason}")]
    AppTopic { topic: String, reason: String },
    #[error("app message for topic `{topic}` failed: {reason}")]
    AppMessage { topic: String, reason: String },
}
