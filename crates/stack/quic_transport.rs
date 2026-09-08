//! Preserve DCUtR's listener role at the QUIC transport boundary.

use std::pin::Pin;
use std::task::{Context, Poll};

use libp2p::core::{
    transport::{DialOpts, ListenerId, PortUse, TransportError, TransportEvent},
    Endpoint,
};
use libp2p::{Multiaddr, Transport};

/// libp2p-quic 0.13.1 treats Listener + Reuse as an ordinary QUIC client dial.
/// Its actual hole-punch listener path instead requires Listener + New, despite
/// cloning the existing listener socket in that path. Normalize only the DCUtR
/// listener-role request; ordinary outbound connections keep port reuse.
/// This preserves the client/server roles required by
/// <https://github.com/libp2p/specs/blob/master/relay/DCUtR.md>.
pub(super) struct DcutrQuicTransport<T>(pub(super) T);

impl<T: Transport + Unpin> Transport for DcutrQuicTransport<T> {
    type Output = T::Output;
    type Error = T::Error;
    type ListenerUpgrade = T::ListenerUpgrade;
    type Dial = T::Dial;

    fn listen_on(
        &mut self,
        id: ListenerId,
        addr: Multiaddr,
    ) -> Result<(), TransportError<Self::Error>> {
        self.0.listen_on(id, addr)
    }
    fn remove_listener(&mut self, id: ListenerId) -> bool {
        self.0.remove_listener(id)
    }
    fn dial(
        &mut self,
        addr: Multiaddr,
        mut opts: DialOpts,
    ) -> Result<Self::Dial, TransportError<Self::Error>> {
        if opts.role == Endpoint::Listener {
            opts.port_use = PortUse::New;
        }
        self.0.dial(addr, opts)
    }
    fn poll(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<TransportEvent<Self::ListenerUpgrade, Self::Error>> {
        Pin::new(&mut self.get_mut().0).poll(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn listener_role_requires_a_hole_punch_listener() {
        let key = libp2p::identity::Keypair::generate_ed25519();
        let mut transport = DcutrQuicTransport(libp2p::quic::tokio::Transport::new(
            libp2p::quic::Config::new(&key),
        ));
        let peer = libp2p::PeerId::random();
        let addr = format!("/ip4/8.8.8.8/udp/4001/quic-v1/p2p/{peer}")
            .parse()
            .unwrap();
        let result = transport.dial(
            addr,
            DialOpts {
                role: Endpoint::Listener,
                port_use: PortUse::Reuse,
            },
        );
        // The unadapted transport creates a client endpoint here. A genuine
        // hole-punch listener must instead require an existing listener socket.
        assert!(matches!(
            result,
            Err(TransportError::Other(
                libp2p::quic::Error::NoActiveListenerForDialAsListener
            ))
        ));
    }
}
