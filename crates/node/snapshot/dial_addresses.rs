use super::NodeSnapshot;
use crate::connectivity::relay::RelayState;

impl NodeSnapshot {
    pub(super) fn apply_private_relayed(&mut self, relay_state: &RelayState) {
        self.private_relayed_listen_addresses = relay_state
            .private_relayed_listen_addrs
            .iter()
            .cloned()
            .collect();
    }

    /// Addresses a peer may dial to reach this node: public direct and
    /// public-relay circuits first, then circuits through a LAN relay.
    pub fn local_dial_addresses(&self) -> Vec<String> {
        let mut public = self.public_direct_listen_addresses.clone();
        public.extend(self.relayed_listen_addresses.iter().cloned());
        public.sort();
        public.dedup();
        let mut private = self.private_relayed_listen_addresses.clone();
        private.sort();
        private.dedup();
        private.retain(|addr| !public.contains(addr));
        public.extend(private);
        public
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_relay_circuits_follow_public_routes() {
        let snapshot = NodeSnapshot {
            relayed_listen_addresses: vec!["/ip4/8.8.8.8/tcp/1/p2p-circuit".into()],
            private_relayed_listen_addresses: vec![
                "/ip4/192.168.0.5/udp/2/webrtc-direct/p2p-circuit".into(),
                "/ip4/8.8.8.8/tcp/1/p2p-circuit".into(),
            ],
            ..NodeSnapshot::default()
        };
        assert_eq!(
            snapshot.local_dial_addresses(),
            vec![
                "/ip4/8.8.8.8/tcp/1/p2p-circuit".to_string(),
                "/ip4/192.168.0.5/udp/2/webrtc-direct/p2p-circuit".to_string(),
            ]
        );
    }
}
