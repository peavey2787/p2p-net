//! ICE server entries (STUN/TURN) shared between the signaling behaviour and
//! every peer connection it creates.

use std::{
    fmt,
    sync::{Arc, Mutex},
};

/// One `RTCIceServer` entry. TURN entries carry the short-lived username and
/// credential issued by the TURN operator or credential broker.
#[derive(Clone, PartialEq, Eq)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

impl fmt::Debug for IceServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IceServer")
            .field("urls", &self.urls)
            .field("username", &self.username.as_ref().map(|_| "<redacted>"))
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Runtime-updatable ICE server list. Clones share the same list, so short-lived
/// TURN credentials can be refreshed without rebuilding the swarm; each new
/// `RtcPeerConnection` reads the list current at its creation.
#[derive(Debug, Clone, Default)]
pub struct IceServers(Arc<Mutex<Vec<IceServer>>>);

impl IceServers {
    pub fn new(servers: Vec<IceServer>) -> Self {
        Self(Arc::new(Mutex::new(servers)))
    }

    /// Replace the whole list (e.g. after a credential refresh).
    pub fn replace(&self, servers: Vec<IceServer>) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = servers;
    }

    /// Copy of the current list.
    pub fn current(&self) -> Vec<IceServer> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn() -> IceServer {
        IceServer {
            urls: vec!["turn:turn.example.org:3478".to_string()],
            username: Some("user".to_string()),
            credential: Some("secret".to_string()),
        }
    }

    #[test]
    fn clones_share_replacements() {
        let servers = IceServers::default();
        let shared = servers.clone();
        servers.replace(vec![turn()]);
        assert_eq!(shared.current(), vec![turn()]);
    }

    #[test]
    fn debug_redacts_credentials() {
        let rendered = format!("{:?}", turn());
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("\"user\""));
        assert!(rendered.contains("turn.example.org"));
    }
}
