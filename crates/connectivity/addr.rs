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

/// Return true when an address is safe to advertise as a public direct address.
///
/// Private, loopback, link-local, unspecified, documentation, multicast, and
/// CGNAT IP ranges are intentionally rejected. Relayed `/p2p-circuit`
/// addresses are handled by the relay module and are not direct addresses.
pub(crate) fn is_public_direct_addr(addr: &Multiaddr) -> bool {
    let mut has_routable_endpoint = false;

    for protocol in addr.iter() {
        match protocol {
            Protocol::Ip4(ip) => {
                if !is_public_ipv4(ip) {
                    return false;
                }
                has_routable_endpoint = true;
            }
            Protocol::Ip6(ip) => {
                if !is_public_ipv6(ip) {
                    return false;
                }
                has_routable_endpoint = true;
            }
            Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_) | Protocol::Dnsaddr(_) => {
                has_routable_endpoint = true;
            }
            _ => {}
        }
    }

    has_routable_endpoint
}

/// Return true when an address is a concrete local/private direct listen address.
/// These addresses may be useful on LAN/dev setups but must not be displayed as
/// public reachability or manually added as external addresses.
pub(crate) fn is_local_direct_addr(addr: &Multiaddr) -> bool {
    addr.iter().any(|protocol| match protocol {
        Protocol::Ip4(ip) => !is_public_ipv4(ip),
        Protocol::Ip6(ip) => !is_public_ipv6(ip),
        _ => false,
    })
}

fn is_public_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let octets = ip.octets();
    if ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
    {
        return false;
    }

    // RFC 6598 carrier-grade NAT: 100.64.0.0/10.
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return false;
    }

    // Documentation/test networks: 192.0.2.0/24, 198.51.100.0/24,
    // 203.0.113.0/24. They are valid examples but never public routes.
    if (octets[0], octets[1], octets[2]) == (192, 0, 2)
        || (octets[0], octets[1], octets[2]) == (198, 51, 100)
        || (octets[0], octets[1], octets[2]) == (203, 0, 113)
    {
        return false;
    }

    true
}

fn is_public_ipv6(ip: std::net::Ipv6Addr) -> bool {
    let segments = ip.segments();
    let first = segments[0];

    if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() {
        return false;
    }

    // Unique-local fc00::/7.
    if (first & 0xfe00) == 0xfc00 {
        return false;
    }

    // Link-local fe80::/10.
    if (first & 0xffc0) == 0xfe80 {
        return false;
    }

    // Documentation 2001:db8::/32.
    if first == 0x2001 && segments[1] == 0x0db8 {
        return false;
    }

    true
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

    #[test]
    fn public_direct_addr_rejects_local_private_and_cgnat_ranges() {
        let loopback: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        let docker: Multiaddr = "/ip4/172.17.0.1/udp/4001/quic-v1".parse().unwrap();
        let cgnat: Multiaddr = "/ip4/100.64.1.1/tcp/4001".parse().unwrap();
        let public: Multiaddr = "/ip4/8.8.8.8/tcp/4001".parse().unwrap();
        let dns: Multiaddr = "/dns/bootstrap.example.com/tcp/4001".parse().unwrap();

        assert!(!is_public_direct_addr(&loopback));
        assert!(!is_public_direct_addr(&docker));
        assert!(!is_public_direct_addr(&cgnat));
        assert!(is_public_direct_addr(&public));
        assert!(is_public_direct_addr(&dns));
    }

    #[test]
    fn local_direct_addr_labels_private_listen_addresses() {
        let docker: Multiaddr = "/ip4/172.17.0.1/udp/4001/quic-v1".parse().unwrap();
        let public: Multiaddr = "/ip4/8.8.8.8/tcp/4001".parse().unwrap();
        let dns: Multiaddr = "/dns/bootstrap.example.com/tcp/4001".parse().unwrap();

        assert!(is_local_direct_addr(&docker));
        assert!(!is_local_direct_addr(&public));
        assert!(!is_local_direct_addr(&dns));
    }
}
