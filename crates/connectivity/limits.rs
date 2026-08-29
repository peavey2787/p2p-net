use crate::common::error::config_error;
use std::collections::{HashMap, HashSet};

use libp2p::multiaddr::Protocol;
use libp2p::swarm::ConnectionId;
use libp2p::{Multiaddr, PeerId};
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

/// Runtime helper for enforcing per-IP caps and tracking outbound connections
/// that can be released immediately before a high-priority application dial.
#[derive(Debug, Clone, Default)]
pub struct ConnectionCapState {
    max_established_per_ip: Option<u32>,
    max_established_outgoing: Option<u32>,
    by_connection: HashMap<ConnectionId, String>,
    outgoing_by_connection: HashMap<ConnectionId, PeerId>,
    application_outgoing_connections: HashSet<ConnectionId>,
    by_ip: HashMap<String, u32>,
    pub cap_disconnects: usize,
}

impl ConnectionCapState {
    pub fn new(cfg: &ConnectionLimitsConfig) -> Self {
        Self {
            max_established_per_ip: cfg.enabled.then_some(cfg.max_established_per_ip).flatten(),
            max_established_outgoing: cfg
                .enabled
                .then_some(cfg.max_established_outgoing)
                .flatten(),
            by_connection: HashMap::new(),
            outgoing_by_connection: HashMap::new(),
            application_outgoing_connections: HashSet::new(),
            by_ip: HashMap::new(),
            cap_disconnects: 0,
        }
    }

    /// Records an infrastructure connection and reports whether a custom cap
    /// was exceeded.
    pub fn record_established(
        &mut self,
        connection_id: ConnectionId,
        peer_id: PeerId,
        remote_addr: &Multiaddr,
        outgoing: bool,
    ) -> bool {
        self.record_established_with_priority(connection_id, peer_id, remote_addr, outgoing, false)
    }

    /// Record a connection and classify outbound application connections.
    ///
    /// This must not reject an already-established infrastructure connection
    /// merely to keep speculative headroom. Kademlia still owns that connection
    /// and will immediately redial it, creating an unbounded connect/close loop.
    /// Callers instead use [`Self::outgoing_connections_to_release`] directly
    /// before an application-peer dial needs capacity.
    pub fn record_established_with_priority(
        &mut self,
        connection_id: ConnectionId,
        peer_id: PeerId,
        remote_addr: &Multiaddr,
        outgoing: bool,
        application_peer: bool,
    ) -> bool {
        if outgoing {
            self.outgoing_by_connection.insert(connection_id, peer_id);
            if application_peer {
                self.application_outgoing_connections.insert(connection_id);
            }
        }
        let exceeds_ip_cap = multiaddr_ip_key(remote_addr).is_some_and(|ip| {
            self.by_connection.insert(connection_id, ip.clone());
            let count = self.by_ip.entry(ip).or_insert(0);
            *count = count.saturating_add(1);
            self.max_established_per_ip
                .is_some_and(|limit| *count > limit)
        });
        if exceeds_ip_cap {
            self.cap_disconnects = self.cap_disconnects.saturating_add(1);
            return true;
        }

        false
    }

    pub fn record_closed(&mut self, connection_id: ConnectionId) {
        self.outgoing_by_connection.remove(&connection_id);
        self.application_outgoing_connections.remove(&connection_id);
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

    /// Number of outbound connections to release before a high-priority
    /// application-peer dial.
    pub fn outgoing_connections_to_release(&self, desired_headroom: u32) -> usize {
        let Some(limit) = self.max_established_outgoing else {
            return 0;
        };
        let current = u32::try_from(self.outgoing_by_connection.len()).unwrap_or(u32::MAX);
        let available = limit.saturating_sub(current);
        usize::try_from(desired_headroom.saturating_sub(available)).unwrap_or(usize::MAX)
    }

    pub fn outgoing_peers(&self) -> impl Iterator<Item = PeerId> + '_ {
        self.outgoing_by_connection.values().copied()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infrastructure_is_released_only_when_an_application_dial_needs_headroom() {
        let cfg = ConnectionLimitsConfig {
            max_established_outgoing: Some(3),
            max_established_per_ip: None,
            ..ConnectionLimitsConfig::default()
        };
        let mut caps = ConnectionCapState::new(&cfg);
        let addr: Multiaddr = "/ip4/8.8.8.8/tcp/4001".parse().unwrap();

        for id in 0..3 {
            assert!(!caps.record_established_with_priority(
                ConnectionId::new_unchecked(id),
                PeerId::random(),
                &addr,
                true,
                false,
            ));
        }
        assert_eq!(caps.outgoing_connections_to_release(1), 1);
    }

    #[test]
    fn closing_infrastructure_connection_restores_its_capacity() {
        let cfg = ConnectionLimitsConfig {
            max_established_outgoing: Some(2),
            max_established_per_ip: None,
            ..ConnectionLimitsConfig::default()
        };
        let mut caps = ConnectionCapState::new(&cfg);
        let addr: Multiaddr = "/ip4/8.8.8.8/tcp/4001".parse().unwrap();
        let first = ConnectionId::new_unchecked(1);

        assert!(!caps.record_established_with_priority(
            first,
            PeerId::random(),
            &addr,
            true,
            false,
        ));
        assert!(!caps.record_established_with_priority(
            ConnectionId::new_unchecked(2),
            PeerId::random(),
            &addr,
            true,
            false,
        ));
        assert_eq!(caps.outgoing_connections_to_release(1), 1);

        // Closing tracked connections restores the corresponding headroom.
        caps.record_closed(ConnectionId::new_unchecked(2));
        caps.record_closed(first);

        assert_eq!(caps.outgoing_connections_to_release(1), 0);

        assert!(!caps.record_established_with_priority(
            ConnectionId::new_unchecked(3),
            PeerId::random(),
            &addr,
            true,
            false,
        ));
    }
}
