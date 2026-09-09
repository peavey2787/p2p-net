//! Keep early TCP dials from advertising an ephemeral, non-listening NAT port.

use std::{
    collections::{HashMap, HashSet},
    io,
    net::IpAddr,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use libp2p::{
    core::transport::{DialOpts, ListenerId, PortUse, TransportError, TransportEvent},
    multiaddr::Protocol,
    tcp, Multiaddr, Transport,
};
use tokio::sync::Notify;

type Native = tcp::tokio::Transport;

struct Listener {
    ipv4: bool,
    wildcard: bool,
    ready: HashSet<IpAddr>,
}

struct State {
    inner: Native,
    listeners: HashMap<ListenerId, Listener>,
}

impl State {
    fn awaiting_reuse(&self, ip: IpAddr) -> bool {
        let mut pending = false;
        for listener in self.listeners.values().filter(|l| l.ipv4 == ip.is_ipv4()) {
            if listener
                .ready
                .iter()
                .any(|known| known.is_loopback() == ip.is_loopback())
            {
                return false;
            }
            pending |= listener.wildcard;
        }
        pending
    }
}

fn address_ip(addr: &Multiaddr) -> Option<IpAddr> {
    match addr.iter().next()? {
        Protocol::Ip4(ip) => Some(ip.into()),
        Protocol::Ip6(ip) => Some(ip.into()),
        _ => None,
    }
}

/// Native TCP with readiness-aware listener-port reuse.
///
/// Upstream wildcard listeners register reusable ports only when interface
/// events are polled. `dial` captures its binding choice immediately, so an
/// early bootstrap dial can otherwise silently use an ephemeral port. Defer
/// only Reuse dials awaiting a matching wildcard listener; outbound-only nodes,
/// explicit New-port requests and already-ready listeners keep native behavior.
pub(super) struct ReadyTcpTransport {
    state: Arc<Mutex<State>>,
    changed: Arc<Notify>,
}

impl ReadyTcpTransport {
    pub(super) fn new(config: tcp::Config) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                inner: Native::new(config),
                listeners: HashMap::new(),
            })),
            changed: Arc::new(Notify::new()),
        }
    }
}

impl Transport for ReadyTcpTransport {
    type Output = <Native as Transport>::Output;
    type Error = io::Error;
    type ListenerUpgrade = <Native as Transport>::ListenerUpgrade;
    type Dial = <Native as Transport>::Dial;

    fn listen_on(
        &mut self,
        id: ListenerId,
        addr: Multiaddr,
    ) -> Result<(), TransportError<io::Error>> {
        let mut state = self.state.lock().expect("TCP transport mutex poisoned");
        state.inner.listen_on(id, addr.clone())?;
        if let Some(ip) = address_ip(&addr) {
            let mut ready = HashSet::new();
            if !ip.is_unspecified() {
                ready.insert(ip);
            }
            state.listeners.insert(
                id,
                Listener {
                    ipv4: ip.is_ipv4(),
                    wildcard: ip.is_unspecified(),
                    ready,
                },
            );
        }
        self.changed.notify_waiters();
        Ok(())
    }

    fn remove_listener(&mut self, id: ListenerId) -> bool {
        let mut state = self.state.lock().expect("TCP transport mutex poisoned");
        let removed = state.inner.remove_listener(id);
        if removed {
            state.listeners.remove(&id);
            self.changed.notify_waiters();
        }
        removed
    }

    fn dial(
        &mut self,
        addr: Multiaddr,
        opts: DialOpts,
    ) -> Result<Self::Dial, TransportError<io::Error>> {
        let mut state = self.state.lock().expect("TCP transport mutex poisoned");
        // Let native TCP validate the full Multiaddr, including unsupported
        // transport suffixes. An unpolled dial has not initiated a connection.
        let initial = state.inner.dial(addr.clone(), opts)?;
        let ip = address_ip(&addr);
        if opts.port_use != PortUse::Reuse || !ip.is_some_and(|ip| state.awaiting_reuse(ip)) {
            return Ok(initial);
        }
        drop(initial);
        drop(state);
        let state = Arc::clone(&self.state);
        let changed = Arc::clone(&self.changed);
        Ok(Box::pin(async move {
            loop {
                // Register before checking state to avoid losing a readiness
                // notification. notify_waiters wakes every pending startup dial.
                let notified = changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                let dial = {
                    let mut state = state.lock().expect("TCP transport mutex poisoned");
                    if ip.is_some_and(|ip| state.awaiting_reuse(ip)) {
                        None
                    } else {
                        Some(state.inner.dial(addr.clone(), opts))
                    }
                };
                if let Some(dial) = dial {
                    return match dial {
                        Ok(dial) => dial.await,
                        Err(TransportError::Other(error)) => Err(error),
                        Err(TransportError::MultiaddrNotSupported(addr)) => Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            addr.to_string(),
                        )),
                    };
                }
                notified.await;
            }
        }))
    }

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<TransportEvent<Self::ListenerUpgrade, io::Error>> {
        let mut state = self.state.lock().expect("TCP transport mutex poisoned");
        let event = Pin::new(&mut state.inner).poll(cx);
        match &event {
            Poll::Ready(TransportEvent::NewAddress {
                listener_id,
                listen_addr,
            }) => {
                if let (Some(listener), Some(ip)) = (
                    state.listeners.get_mut(listener_id),
                    address_ip(listen_addr),
                ) {
                    listener.ready.insert(ip);
                    self.changed.notify_waiters();
                }
            }
            Poll::Ready(TransportEvent::AddressExpired {
                listener_id,
                listen_addr,
            }) => {
                if let (Some(listener), Some(ip)) = (
                    state.listeners.get_mut(listener_id),
                    address_ip(listen_addr),
                ) {
                    listener.ready.remove(&ip);
                    self.changed.notify_waiters();
                }
            }
            Poll::Ready(TransportEvent::ListenerClosed { listener_id, .. }) => {
                state.listeners.remove(listener_id);
                self.changed.notify_waiters();
            }
            _ => {}
        }
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::future::poll_fn;
    use libp2p::core::Endpoint;
    use std::time::Duration;

    fn reuse() -> DialOpts {
        DialOpts {
            role: Endpoint::Dialer,
            port_use: PortUse::Reuse,
        }
    }

    #[tokio::test]
    async fn early_wildcard_dial_waits_and_reuses_listener_port() {
        let remote = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = format!("/ip4/127.0.0.1/tcp/{}", remote.local_addr().unwrap().port())
            .parse()
            .unwrap();
        let mut transport = ReadyTcpTransport::new(tcp::Config::default());
        transport
            .listen_on(ListenerId::next(), "/ip4/0.0.0.0/tcp/0".parse().unwrap())
            .unwrap();
        let mut dial = transport.dial(target, reuse()).unwrap();
        assert!(futures::poll!(dial.as_mut()).is_pending());
        let port = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let TransportEvent::NewAddress { listen_addr, .. } =
                    poll_fn(|cx| Pin::new(&mut transport).poll(cx)).await
                {
                    if address_ip(&listen_addr).is_some_and(|ip| ip.is_loopback()) {
                        break listen_addr
                            .iter()
                            .find_map(|p| {
                                if let Protocol::Tcp(port) = p {
                                    Some(port)
                                } else {
                                    None
                                }
                            })
                            .unwrap();
                    }
                }
            }
        })
        .await
        .unwrap();
        let stream = tokio::time::timeout(Duration::from_secs(5), dial)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stream.0.local_addr().unwrap().port(), port);
    }

    #[tokio::test]
    async fn removing_pending_listener_releases_waiting_dial() {
        let remote = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = format!("/ip4/127.0.0.1/tcp/{}", remote.local_addr().unwrap().port())
            .parse()
            .unwrap();
        let mut transport = ReadyTcpTransport::new(tcp::Config::default());
        let id = ListenerId::next();
        transport
            .listen_on(id, "/ip4/0.0.0.0/tcp/0".parse().unwrap())
            .unwrap();
        let mut dial = transport.dial(target, reuse()).unwrap();
        assert!(futures::poll!(dial.as_mut()).is_pending());
        assert!(transport.remove_listener(id));
        tokio::time::timeout(Duration::from_secs(5), dial)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn outbound_only_transport_does_not_wait_for_a_listener() {
        let remote = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = format!("/ip4/127.0.0.1/tcp/{}", remote.local_addr().unwrap().port())
            .parse()
            .unwrap();
        let mut transport = ReadyTcpTransport::new(tcp::Config::default());
        tokio::time::timeout(
            Duration::from_secs(5),
            transport.dial(target, reuse()).unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
    }
}
