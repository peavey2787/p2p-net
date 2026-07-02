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

impl NetError {
    /// Build a configuration error for the default config surface.
    #[must_use]
    pub fn config(reason: impl Into<String>) -> Self {
        Self::config_at("<config>", reason)
    }

    /// Build a configuration error for a specific config or resolver surface.
    #[must_use]
    pub fn config_at(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Config {
            path: path.into(),
            reason: reason.into(),
        }
    }
}

/// Build a configuration error for the default config surface.
#[must_use]
pub fn config_error(reason: impl Into<String>) -> NetError {
    NetError::config(reason)
}

/// Build a configuration error for a specific config or resolver surface.
#[must_use]
pub fn config_error_at(path: impl Into<String>, reason: impl Into<String>) -> NetError {
    NetError::config_at(path, reason)
}

#[cfg(test)]
mod tests {
    use super::{config_error, config_error_at, NetError};

    #[test]
    fn default_config_error_uses_default_config_path() {
        let err = config_error("bad value");
        match err {
            NetError::Config { path, reason } => {
                assert_eq!(path, "<config>");
                assert_eq!(reason, "bad value");
            }
            other => panic!("expected config error, got {other:?}"),
        }
    }

    #[test]
    fn custom_config_error_preserves_custom_path() {
        let err = config_error_at("<capability-resolver>", "invalid capability");
        match err {
            NetError::Config { path, reason } => {
                assert_eq!(path, "<capability-resolver>");
                assert_eq!(reason, "invalid capability");
            }
            other => panic!("expected config error, got {other:?}"),
        }
    }
}
