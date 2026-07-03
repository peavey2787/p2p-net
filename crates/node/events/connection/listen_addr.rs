use libp2p::Multiaddr;

use crate::connectivity::addr::{is_local_direct_addr, is_public_direct_addr};
use crate::connectivity::relay::is_p2p_circuit_addr;
use crate::node::NodeSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListenAddrClass {
    PublicDirect,
    Relayed,
    LocalOnly,
}

impl ListenAddrClass {
    pub(super) fn advertise_as_external(self) -> bool {
        matches!(self, Self::PublicDirect | Self::Relayed)
    }

    pub(super) fn is_relayed(self) -> bool {
        matches!(self, Self::Relayed)
    }
}

pub(super) fn classify_listen_addr(addr: &Multiaddr) -> ListenAddrClass {
    if is_p2p_circuit_addr(addr) {
        ListenAddrClass::Relayed
    } else if is_public_direct_addr(addr) {
        ListenAddrClass::PublicDirect
    } else {
        ListenAddrClass::LocalOnly
    }
}

pub(super) fn record_listen_addr_snapshot(
    snapshot: &mut NodeSnapshot,
    addr: &Multiaddr,
    classification: ListenAddrClass,
) {
    let addr_string = addr.to_string();
    match classification {
        ListenAddrClass::PublicDirect => {
            push_unique(
                &mut snapshot.public_direct_listen_addresses,
                addr_string.clone(),
            );
            snapshot.public_addr = Some(addr_string);
        }
        ListenAddrClass::Relayed => {
            push_unique(&mut snapshot.relayed_listen_addresses, addr_string.clone());
            snapshot.public_addr = Some(addr_string);
        }
        ListenAddrClass::LocalOnly if is_local_direct_addr(addr) => {
            push_unique(&mut snapshot.local_listen_addresses, addr_string);
        }
        ListenAddrClass::LocalOnly => {}
    }
}

pub(super) fn remove_listen_addr_snapshot(
    snapshot: &mut NodeSnapshot,
    addr: &Multiaddr,
    classification: ListenAddrClass,
) {
    let addr = addr.to_string();
    match classification {
        ListenAddrClass::PublicDirect => snapshot
            .public_direct_listen_addresses
            .retain(|v| v != &addr),
        ListenAddrClass::Relayed => snapshot.relayed_listen_addresses.retain(|v| v != &addr),
        ListenAddrClass::LocalOnly => snapshot.local_listen_addresses.retain(|v| v != &addr),
    }
    snapshot.public_addr = snapshot
        .relayed_listen_addresses
        .first()
        .cloned()
        .or_else(|| snapshot.public_direct_listen_addresses.first().cloned());
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

pub(super) fn autonat_status_label(debug: &str) -> String {
    if debug.contains("NoAddresses") {
        "unknown_no_public_direct_addr_yet".to_string()
    } else if debug.contains("NoServer") {
        "unknown_waiting_for_autonat_server".to_string()
    } else {
        debug.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_listen_addresses_are_not_public_addresses() {
        let docker: Multiaddr = "/ip4/172.17.0.1/udp/4001/quic-v1".parse().unwrap();
        let public: Multiaddr = "/ip4/8.8.8.8/udp/4001/quic-v1".parse().unwrap();

        assert_eq!(classify_listen_addr(&docker), ListenAddrClass::LocalOnly);
        assert_eq!(classify_listen_addr(&public), ListenAddrClass::PublicDirect);
    }

    #[test]
    fn autonat_no_addresses_is_labeled_as_pending_public_direct_addr() {
        assert_eq!(
            autonat_status_label("OutboundProbe(Error { error: NoAddresses })"),
            "unknown_no_public_direct_addr_yet"
        );
    }
}
