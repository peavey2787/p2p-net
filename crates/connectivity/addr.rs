//! Shared multiaddr classification helpers used by discovery components.
//!
//! These helpers intentionally classify only transport/reachability details.
//! Persistence policy, relay selection policy, and peer identity validation stay
//! in their owning modules.

use libp2p::multiaddr::Protocol;
use libp2p::Multiaddr;

/// Return true when an address contains a directly reachable transport base.
///
/// `/dnsaddr` is intentionally excluded because it requires separate DNSADDR
/// expansion before the concrete transport is known.
pub(crate) fn has_reachable_transport(addr: &Multiaddr) -> bool {
    addr.iter().any(|protocol| {
        matches!(
            protocol,
            Protocol::Ip4(_)
                | Protocol::Ip6(_)
                | Protocol::Dns(_)
                | Protocol::Dns4(_)
                | Protocol::Dns6(_)
        )
    })
}

/// Return true when an address contains an unspecified IP endpoint.
pub(crate) fn has_unspecified_ip(addr: &Multiaddr) -> bool {
    addr.iter().any(|protocol| match protocol {
        Protocol::Ip4(ip) => ip.is_unspecified(),
        Protocol::Ip6(ip) => ip.is_unspecified(),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reachable_transport_accepts_ip_and_dns_transports() {
        let ip4: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        let ip6: Multiaddr = "/ip6/::1/tcp/4001".parse().unwrap();
        let dns: Multiaddr = "/dns/example.com/tcp/4001".parse().unwrap();
        let dns4: Multiaddr = "/dns4/example.com/tcp/4001".parse().unwrap();
        let dns6: Multiaddr = "/dns6/example.com/tcp/4001".parse().unwrap();

        assert!(has_reachable_transport(&ip4));
        assert!(has_reachable_transport(&ip6));
        assert!(has_reachable_transport(&dns));
        assert!(has_reachable_transport(&dns4));
        assert!(has_reachable_transport(&dns6));
    }

    #[test]
    fn reachable_transport_rejects_dnsaddr_until_resolved() {
        let dnsaddr: Multiaddr = "/dnsaddr/bootstrap.example.com".parse().unwrap();

        assert!(!has_reachable_transport(&dnsaddr));
    }

    #[test]
    fn unspecified_ip_detects_unspecified_ipv4_and_ipv6() {
        let ip4: Multiaddr = "/ip4/0.0.0.0/tcp/4001".parse().unwrap();
        let ip6: Multiaddr = "/ip6/::/tcp/4001".parse().unwrap();
        let loopback: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        let dns: Multiaddr = "/dns/example.com/tcp/4001".parse().unwrap();

        assert!(has_unspecified_ip(&ip4));
        assert!(has_unspecified_ip(&ip6));
        assert!(!has_unspecified_ip(&loopback));
        assert!(!has_unspecified_ip(&dns));
    }
}
