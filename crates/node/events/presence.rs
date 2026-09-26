//! Peer presence is per peer, not per connection.
//!
//! A peer can hold several simultaneous connections (a browser that reaches a
//! relay through two local interfaces, or a relayed circuit next to a direct
//! upgrade). Closing a redundant connection must not report the peer as gone,
//! so `PeerConnected` fires for the first connection and `PeerDisconnected`
//! only when the last one closes.

/// `num_established` after a connection opened (always >= 1).
pub(super) fn is_first_connection(num_established: u32) -> bool {
    num_established == 1
}

/// `num_established` remaining after a connection closed.
pub(super) fn is_last_connection(remaining: u32) -> bool {
    remaining == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_connection_announces_the_peer() {
        assert!(is_first_connection(1));
        assert!(!is_first_connection(2));
    }

    #[test]
    fn a_redundant_connection_closing_keeps_the_peer_present() {
        assert!(!is_last_connection(1));
        assert!(is_last_connection(0));
    }
}
