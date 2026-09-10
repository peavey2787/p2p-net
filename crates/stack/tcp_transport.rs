//! Readiness-aware TCP port reuse and bounded DCUtR simultaneous-open retries.

use std::{
    collections::{HashMap, HashSet},
    io,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::{Arc, Mutex, Weak},
    task::{Context, Poll},
    time::Duration,
};

use libp2p::{
    core::transport::{DialOpts, ListenerId, PortUse, TransportError, TransportEvent},
    multiaddr::Protocol,
    tcp, Multiaddr, Transport,
};
use tokio::sync::Notify;

type Native = tcp::tokio::Transport;

const PUNCH_CONNECT_WINDOW: Duration = Duration::from_secs(8);
const PUNCH_RETRY_DELAY: Duration = Duration::from_millis(250);

// A remote NAT may reject the first SYN before the other peer opens its
// mapping. Do not turn that early refusal into several overlapping DCUtR
// rounds. Retry only that same endpoint, within one bounded connect window.
async fn connect_punch_window<T, F, Fut>(
    mut attempt: F,
    window: Duration,
    delay: Duration,
) -> io::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = io::Result<T>>,
{
    let deadline = tokio::time::Instant::now() + window;
    loop {
        let error = match tokio::time::timeout_at(deadline, attempt()).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => error,
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "TCP hole-punch connect window elapsed",
                ))
            }
        };
        if !matches!(
            error.kind(),
            io::ErrorKind::ConnectionRefused | io::ErrorKind::AddrInUse
        ) {
            return Err(error);
        }
        let next = tokio::time::Instant::now() + delay;
        if next >= deadline {
            return Err(error);
        }
        tokio::time::sleep_until(next).await;
    }
}

struct Listener {
    ipv4: bool,
    wildcard: bool,
    ready: HashSet<IpAddr>,
}

struct State {
    inner: Native,
    listeners: HashMap<ListenerId, Listener>,
    pending_reuse: HashMap<SocketAddr, Weak<tokio::sync::Mutex<()>>>,
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
/// only Reuse dials awaiting a matching wildcard listener. Listener-role Reuse
/// dials additionally retry early refusal/address-in-use errors for at most
/// eight seconds. Dialer-role refusals are not retried. Concurrent reused-port
/// connects to the same destination wait for each other; unrelated endpoints
/// and new-port dials remain independent.
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
                pending_reuse: HashMap::new(),
            })),
            changed: Arc::new(Notify::new()),
        }
    }

    fn serialize_reuse(
        &self,
        dial: <Native as Transport>::Dial,
        addr: &Multiaddr,
        opts: DialOpts,
    ) -> <Native as Transport>::Dial {
        if opts.port_use != PortUse::Reuse {
            return dial;
        }
        let Some(ip) = address_ip(addr) else {
            return dial;
        };
        let Some(Protocol::Tcp(port)) = addr.iter().nth(1) else {
            return dial;
        };
        let mut state = self.state.lock().expect("TCP transport mutex poisoned");
        if !state.listeners.values().any(|l| l.ipv4 == ip.is_ipv4()) {
            return dial;
        }
        // Retain only active/queued dials, not a lifetime history of endpoints.
        state
            .pending_reuse
            .retain(|_, gate| gate.strong_count() > 0);
        let entry = state
            .pending_reuse
            .entry(SocketAddr::new(ip, port))
            .or_default();
        let gate = entry.upgrade().unwrap_or_else(|| {
            let gate = Arc::new(tokio::sync::Mutex::new(()));
            *entry = Arc::downgrade(&gate);
            gate
        });
        drop(state);
        Box::pin(async move {
            // Reused source ports cannot own two pending connects to the same
            // destination. Cancellation drops this guard and wakes the next
            // dial; successful streams are never shared between upgrades.
            let _pending = gate.lock().await;
            dial.await
        })
    }

    fn with_punch_window(
        &self,
        initial: <Native as Transport>::Dial,
        addr: Multiaddr,
        opts: DialOpts,
    ) -> <Native as Transport>::Dial {
        if opts.role != libp2p::core::Endpoint::Listener || opts.port_use != PortUse::Reuse {
            return initial;
        }
        let state = Arc::clone(&self.state);
        let mut initial = Some(initial);
        Box::pin(connect_punch_window(
            move || {
                if let Some(initial) = initial.take() {
                    return initial;
                }
                let mut state = state.lock().expect("TCP transport mutex poisoned");
                // Never restart punching from an ephemeral port after our
                // listener was removed. No work is spawned outside this dial.
                let ready = address_ip(&addr).is_some_and(|ip| {
                    state.listeners.values().any(|listener| {
                        listener.ipv4 == ip.is_ipv4()
                            && listener
                                .ready
                                .iter()
                                .any(|known| known.is_loopback() == ip.is_loopback())
                    })
                });
                if !ready {
                    return Box::pin(async {
                        Err(io::Error::new(
                            io::ErrorKind::NotConnected,
                            "TCP hole-punch listener is no longer ready",
                        ))
                    });
                }
                match state.inner.dial(addr.clone(), opts) {
                    Ok(dial) => dial,
                    Err(TransportError::Other(error)) => Box::pin(async { Err(error) }),
                    Err(TransportError::MultiaddrNotSupported(addr)) => Box::pin(async move {
                        Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            addr.to_string(),
                        ))
                    }),
                }
            },
            PUNCH_CONNECT_WINDOW,
            PUNCH_RETRY_DELAY,
        ))
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
            drop(state);
            let dial = self.with_punch_window(initial, addr.clone(), opts);
            return Ok(self.serialize_reuse(dial, &addr, opts));
        }
        drop(initial);
        drop(state);
        let state = Arc::clone(&self.state);
        let changed = Arc::clone(&self.changed);
        let retry_addr = addr.clone();
        let ready_dial = Box::pin(async move {
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
        });
        let dial = self.with_punch_window(ready_dial, retry_addr.clone(), opts);
        Ok(self.serialize_reuse(dial, &retry_addr, opts))
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

    #[tokio::test]
    async fn overlapping_reused_dials_wait_and_cancellation_releases_them() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mut transport = ReadyTcpTransport::new(tcp::Config::default());
        transport
            .listen_on(ListenerId::next(), "/ip4/127.0.0.1/tcp/0".parse().unwrap())
            .unwrap();
        let addr = "/ip4/127.0.0.1/tcp/1".parse().unwrap();
        let started = Arc::new(AtomicUsize::new(0));
        let first_started = Arc::clone(&started);
        let first: <Native as Transport>::Dial = Box::pin(async move {
            first_started.fetch_add(1, Ordering::Relaxed);
            std::future::pending().await
        });
        let mut first = transport.serialize_reuse(first, &addr, reuse());
        assert!(futures::poll!(first.as_mut()).is_pending());
        let second_started = Arc::clone(&started);
        let second: <Native as Transport>::Dial = Box::pin(async move {
            second_started.fetch_add(1, Ordering::Relaxed);
            Err(io::ErrorKind::ConnectionRefused.into())
        });
        let mut second = transport.serialize_reuse(second, &addr, reuse());
        assert!(futures::poll!(second.as_mut()).is_pending());
        assert_eq!(started.load(Ordering::Relaxed), 1);
        let independent = transport.serialize_reuse(
            Box::pin(async { Err(io::ErrorKind::PermissionDenied.into()) }),
            &"/ip4/127.0.0.1/tcp/2".parse().unwrap(),
            reuse(),
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), independent)
                .await
                .unwrap()
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        drop(first);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), second)
                .await
                .unwrap()
                .unwrap_err()
                .kind(),
            io::ErrorKind::ConnectionRefused
        );
        assert_eq!(started.load(Ordering::Relaxed), 2);
        let other = "/ip4/127.0.0.1/tcp/2".parse().unwrap();
        let next = transport.serialize_reuse(Box::pin(std::future::pending()), &other, reuse());
        let state = transport.state.lock().unwrap();
        assert_eq!(
            state.pending_reuse.len(),
            1,
            "dead endpoint entries must be reclaimed"
        );
        drop(state);
        drop(next);
    }

    #[tokio::test]
    async fn punch_window_retries_transient_errors_but_not_permanent_errors() {
        let mut calls = 0;
        let result = connect_punch_window(
            || {
                calls += 1;
                std::future::ready(match calls {
                    1 => Err(io::ErrorKind::ConnectionRefused.into()),
                    2 => Err(io::ErrorKind::AddrInUse.into()),
                    _ => Ok(42),
                })
            },
            Duration::from_secs(1),
            Duration::from_millis(1),
        )
        .await
        .unwrap();
        assert_eq!(result, 42);
        assert_eq!(calls, 3);
        let mut calls = 0;
        let result: io::Result<()> = connect_punch_window(
            || {
                calls += 1;
                std::future::ready(Err(io::ErrorKind::PermissionDenied.into()))
            },
            Duration::from_secs(1),
            Duration::from_millis(1),
        )
        .await;
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn punch_window_bounds_a_stalled_connect() {
        let result: io::Result<()> = connect_punch_window(
            std::future::pending,
            Duration::from_millis(25),
            Duration::from_millis(1),
        )
        .await;
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn listener_role_survives_an_initial_refusal() {
        let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let remote_addr = reserved.local_addr().unwrap();
        drop(reserved);
        let mut transport = ReadyTcpTransport::new(tcp::Config::default());
        transport
            .listen_on(ListenerId::next(), "/ip4/127.0.0.1/tcp/0".parse().unwrap())
            .unwrap();
        let TransportEvent::NewAddress { listen_addr, .. } =
            poll_fn(|cx| Pin::new(&mut transport).poll(cx)).await
        else {
            panic!("expected listener address");
        };
        let local_port = listen_addr
            .iter()
            .find_map(|p| match p {
                Protocol::Tcp(port) => Some(port),
                _ => None,
            })
            .unwrap();
        let target: Multiaddr = format!("/ip4/127.0.0.1/tcp/{}", remote_addr.port())
            .parse()
            .unwrap();
        // Observe an actual refusal before starting the delayed peer, so an
        // OS-level SYN retry cannot make this pass without our retry wrapper.
        let refused = tokio::time::timeout(
            Duration::from_secs(5),
            transport.dial(target.clone(), reuse()).unwrap(),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::ConnectionRefused);
        let mut opts = reuse();
        opts.role = Endpoint::Listener;
        let dial = transport.with_punch_window(Box::pin(async { Err(refused) }), target, opts);
        let peer = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let listener = tokio::net::TcpListener::bind(remote_addr).await.unwrap();
            listener.accept().await.unwrap().0
        };
        let (stream, peer) =
            tokio::time::timeout(Duration::from_secs(3), async { tokio::join!(dial, peer) })
                .await
                .unwrap();
        let stream = stream.unwrap();
        assert_eq!(stream.0.local_addr().unwrap().port(), local_port);
        assert_eq!(stream.0.peer_addr().unwrap(), remote_addr);
        assert_eq!(stream.0.local_addr().unwrap(), peer.peer_addr().unwrap());
    }

    #[tokio::test]
    async fn ordinary_dial_does_not_retry_a_refusal() {
        // Test the wrapper decision independently of the OS's SYN retry delay.
        // If it wrongly retries, the absent listener changes the result to
        // NotConnected instead of preserving this initial refusal.
        let transport = ReadyTcpTransport::new(tcp::Config::default());
        let initial: <Native as Transport>::Dial =
            Box::pin(async { Err(io::ErrorKind::ConnectionRefused.into()) });
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            transport.with_punch_window(initial, "/ip4/127.0.0.1/tcp/1".parse().unwrap(), reuse()),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionRefused);
    }

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
