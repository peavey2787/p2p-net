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
#[path = "tcp_transport_tests.rs"]
mod tests;
