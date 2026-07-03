use std::net::{Ipv4Addr, Ipv6Addr};

use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};

use super::model::{CachedDialAddrKind, CachedPeerAddr};
use crate::connectivity::addr::{has_reachable_transport, has_unspecified_ip};
use crate::connectivity::discovery::DiscoveryConfig;

pub fn is_cacheable_peer_addr(addr: &Multiaddr, expected_peer: Option<&PeerId>) -> bool {
    let Some(peer) = extract_last_p2p_peer_id(addr) else {
        return false;
    };
    if expected_peer.is_some_and(|expected| expected != &peer) {
        return false;
    }

    if has_unspecified_ip(addr) || contains_dnsaddr(addr) {
        return false;
    }

    has_reachable_transport(addr)
}

pub fn is_persistable_dialable_peer_addr(cfg: &DiscoveryConfig, addr: &Multiaddr) -> bool {
    if !is_cacheable_peer_addr(addr, None) {
        return false;
    }
    classify_dialable_addr(addr)
        .map(|kind| is_persistable_dialable_addr_kind(cfg, kind))
        .unwrap_or(false)
}

pub fn classify_dialable_addr(addr: &Multiaddr) -> Option<CachedDialAddrKind> {
    if !has_reachable_transport(addr) || has_unspecified_ip(addr) || contains_dnsaddr(addr) {
        return None;
    }
    if addr
        .iter()
        .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
    {
        return Some(CachedDialAddrKind::RelayReservation);
    }
    if contains_local_ip(addr) {
        return Some(CachedDialAddrKind::LocalSession);
    }
    Some(CachedDialAddrKind::PublicDirect)
}

pub fn normalize_peer_addr(peer: &PeerId, addr: &Multiaddr) -> Option<Multiaddr> {
    if is_cacheable_peer_addr(addr, Some(peer)) {
        return Some(addr.clone());
    }

    if contains_any_p2p(addr) || !has_reachable_transport(addr) || has_unspecified_ip(addr) {
        return None;
    }

    Some(addr.clone().with(Protocol::P2p(peer.to_owned())))
}

pub(super) fn normalize_entry_kind(entry: &mut CachedPeerAddr) -> Option<()> {
    let addr = entry.addr.parse::<Multiaddr>().ok()?;
    entry.addr_kind = classify_dialable_addr(&addr)?;
    Some(())
}

pub(super) fn is_valid_cache_entry(
    cfg: &DiscoveryConfig,
    entry: &CachedPeerAddr,
    now: u64,
) -> bool {
    let Ok(peer) = entry.peer_id.parse::<PeerId>() else {
        return false;
    };
    let Ok(addr) = entry.addr.parse::<Multiaddr>() else {
        return false;
    };
    if !is_cacheable_peer_addr(&addr, Some(&peer)) {
        return false;
    }
    let Some(kind) = classify_dialable_addr(&addr) else {
        return false;
    };
    if !is_persistable_dialable_addr_kind(cfg, kind) {
        return false;
    }
    if let Some(expires) = entry.expires_unix_secs {
        if expires <= now {
            return false;
        }
    }
    if let Some(max_age_secs) = effective_dialable_max_age_secs(cfg, kind) {
        if entry.last_seen_unix_secs > 0
            && now.saturating_sub(entry.last_seen_unix_secs) > max_age_secs
        {
            return false;
        }
    }
    if cfg.peer_cache_max_failures > 0 && entry.failures >= cfg.peer_cache_max_failures {
        return false;
    }
    true
}

pub(super) fn inferred_expiry_secs(
    cfg: &DiscoveryConfig,
    addr_kind: CachedDialAddrKind,
    now: u64,
) -> Option<u64> {
    effective_dialable_max_age_secs(cfg, addr_kind).map(|ttl| now.saturating_add(ttl))
}

pub(super) fn is_persistable_dialable_addr_kind(
    cfg: &DiscoveryConfig,
    addr_kind: CachedDialAddrKind,
) -> bool {
    match addr_kind {
        CachedDialAddrKind::PublicDirect | CachedDialAddrKind::RelayReservation => true,
        CachedDialAddrKind::LocalSession => cfg.peer_cache_persist_local_addrs,
    }
}

pub(super) fn extract_last_p2p_peer_id(addr: &Multiaddr) -> Option<PeerId> {
    let mut out = None;
    for protocol in addr.iter() {
        if let Protocol::P2p(peer) = protocol {
            out = Some(peer);
        }
    }
    out
}

fn effective_dialable_max_age_secs(
    cfg: &DiscoveryConfig,
    addr_kind: CachedDialAddrKind,
) -> Option<u64> {
    let kind_max = match addr_kind {
        CachedDialAddrKind::PublicDirect => Some(cfg.peer_cache_public_addr_max_age_secs),
        CachedDialAddrKind::RelayReservation => Some(cfg.peer_cache_relay_addr_max_age_secs),
        CachedDialAddrKind::LocalSession => {
            if cfg.peer_cache_local_addr_max_age_secs == 0 {
                None
            } else {
                Some(cfg.peer_cache_local_addr_max_age_secs)
            }
        }
    };
    min_nonzero(kind_max, cfg.peer_cache_max_age_secs)
}

fn min_nonzero(kind_max: Option<u64>, global_max: u64) -> Option<u64> {
    match (kind_max, global_max) {
        (None, 0) => None,
        (None, max) => Some(max),
        (Some(max), 0) => Some(max),
        (Some(kind), global) => Some(kind.min(global)),
    }
}

fn contains_any_p2p(addr: &Multiaddr) -> bool {
    addr.iter()
        .any(|protocol| matches!(protocol, Protocol::P2p(_)))
}

fn contains_dnsaddr(addr: &Multiaddr) -> bool {
    addr.iter()
        .any(|protocol| matches!(protocol, Protocol::Dnsaddr(_)))
}

fn contains_local_ip(addr: &Multiaddr) -> bool {
    addr.iter().any(|protocol| match protocol {
        Protocol::Ip4(ip) => is_local_ipv4(ip),
        Protocol::Ip6(ip) => is_local_ipv6(ip),
        _ => false,
    })
}

fn is_local_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

fn is_local_ipv6(ip: Ipv6Addr) -> bool {
    ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
}
