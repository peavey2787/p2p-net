use crate::common::error::config_error;
use std::collections::HashMap;

use libp2p::multiaddr::Protocol;
use libp2p::swarm::ConnectionId;
use libp2p::Multiaddr;
use serde::{Deserialize, Serialize};

/// Global connection caps for the node.
///
/// These protect the whole node, including volunteer relay nodes. The libp2p
/// connection-limits behaviour enforces all limits except `max_established_per_ip`,
/// which is enforced by this crate after connection establishment because rust-libp2p's
/// stock connection-limits behaviour is peer-oriented, not IP-oriented.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectionLimitsConfig {
    pub enabled: bool,
    pub max_pending_incoming: Option<u32>,
    pub max_pending_outgoing: Option<u32>,
    pub max_established_incoming: Option<u32>,
    pub max_established_outgoing: Option<u32>,
    pub max_established: Option<u32>,
    pub max_established_per_peer: Option<u32>,
    pub max_established_per_ip: Option<u32>,
}

impl Default for ConnectionLimitsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_pending_incoming: Some(32),
            max_pending_outgoing: Some(32),
            max_established_incoming: Some(64),
            max_established_outgoing: Some(64),
            max_established: Some(128),
            max_established_per_peer: Some(3),
            max_established_per_ip: Some(8),
        }
    }
}

impl ConnectionLimitsConfig {
    pub fn validate(&self) -> Result<(), crate::common::error::NetError> {
        if !self.enabled {
            return Ok(());
        }

        ensure_nonzero(
            "connection_limits.max_pending_incoming",
            self.max_pending_incoming,
        )?;
        ensure_nonzero(
            "connection_limits.max_pending_outgoing",
            self.max_pending_outgoing,
        )?;
        ensure_nonzero(
            "connection_limits.max_established_incoming",
            self.max_established_incoming,
        )?;
        ensure_nonzero(
            "connection_limits.max_established_outgoing",
            self.max_established_outgoing,
        )?;
        ensure_nonzero("connection_limits.max_established", self.max_established)?;
        ensure_nonzero(
            "connection_limits.max_established_per_peer",
            self.max_established_per_peer,
        )?;
        ensure_nonzero(
            "connection_limits.max_established_per_ip",
            self.max_established_per_ip,
        )?;

        if let (Some(per_peer), Some(total)) = (self.max_established_per_peer, self.max_established)
        {
            if per_peer > total {
                return Err(config_error(
                    "connection_limits.max_established_per_peer must be <= connection_limits.max_established",
                ));
            }
        }

        if let (Some(per_ip), Some(total)) = (self.max_established_per_ip, self.max_established) {
            if per_ip > total {
                return Err(config_error(
                    "connection_limits.max_established_per_ip must be <= connection_limits.max_established",
                ));
            }
        }

        Ok(())
    }

    pub fn to_libp2p_limits(&self) -> libp2p::connection_limits::ConnectionLimits {
        if !self.enabled {
            return libp2p::connection_limits::ConnectionLimits::default();
        }

        libp2p::connection_limits::ConnectionLimits::default()
            .with_max_pending_incoming(self.max_pending_incoming)
            .with_max_pending_outgoing(self.max_pending_outgoing)
            .with_max_established_incoming(self.max_established_incoming)
            .with_max_established_outgoing(self.max_established_outgoing)
            .with_max_established(self.max_established)
            .with_max_established_per_peer(self.max_established_per_peer)
    }
}

/// Runtime helper for enforcing per-IP connection caps that libp2p's built-in
/// connection limits do not currently expose as a stock setting.
#[derive(Debug, Clone, Default)]
pub struct ConnectionCapState {
    max_established_per_ip: Option<u32>,
    by_connection: HashMap<ConnectionId, String>,
    by_ip: HashMap<String, u32>,
    pub cap_disconnects: usize,
}

impl ConnectionCapState {
    pub fn new(cfg: &ConnectionLimitsConfig) -> Self {
        Self {
            max_established_per_ip: cfg.enabled.then_some(cfg.max_established_per_ip).flatten(),
            by_connection: HashMap::new(),
            by_ip: HashMap::new(),
            cap_disconnects: 0,
        }
    }

    /// Records a connection and returns true when the configured per-IP cap is exceeded.
    pub fn record_established(
        &mut self,
        connection_id: ConnectionId,
        remote_addr: &Multiaddr,
    ) -> bool {
        let Some(ip) = multiaddr_ip_key(remote_addr) else {
            return false;
        };

        self.by_connection.insert(connection_id, ip.clone());
        let count = self.by_ip.entry(ip).or_insert(0);
        *count = count.saturating_add(1);

        if self
            .max_established_per_ip
            .is_some_and(|limit| *count > limit)
        {
            self.cap_disconnects = self.cap_disconnects.saturating_add(1);
            return true;
        }

        false
    }

    pub fn record_closed(&mut self, connection_id: ConnectionId) {
        let Some(ip) = self.by_connection.remove(&connection_id) else {
            return;
        };
        if let Some(count) = self.by_ip.get_mut(&ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.by_ip.remove(&ip);
            }
        }
    }

    pub fn count_for_ip(&self, ip: &str) -> u32 {
        self.by_ip.get(ip).copied().unwrap_or(0)
    }
}

pub fn multiaddr_ip_key(addr: &Multiaddr) -> Option<String> {
    addr.iter().find_map(|protocol| match protocol {
        Protocol::Ip4(ip) => Some(ip.to_string()),
        Protocol::Ip6(ip) => Some(ip.to_string()),
        _ => None,
    })
}

fn ensure_nonzero(field: &str, value: Option<u32>) -> Result<(), crate::common::error::NetError> {
    if value == Some(0) {
        return Err(config_error(format!("{field} must be null or at least 1")));
    }
    Ok(())
}
