//! Local `libp2p-dns` patch for p2p-net.
//!
//! rust-libp2p 0.56's WebSocket builder expects the `libp2p-dns` crate when
//! WebSocket support is compiled. The crates.io `libp2p-dns` line pulled the
//! rejected Hickory resolver dependency into `Cargo.lock`, so p2p-net patches
//! that crate to this minimal no-Hickory implementation.
//!
//! `/dns`, `/dns4`, and `/dns6` use Tokio's OS-backed resolver. `/dnsaddr` is
//! intentionally rejected here so there is no hidden hard-coded third-party DoH
//! dependency inside the transport adapter. p2p-net resolves configured and
//! cached `/dnsaddr` entries before dialing via `crates/connectivity/dns.rs`, where
//! the DoH endpoint is operator-configurable.

use std::{
    error, fmt,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    ops::DerefMut,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use futures::{future, FutureExt, TryFutureExt};
use libp2p_core::{
    multiaddr::{Multiaddr, Protocol},
    transport::{DialOpts, ListenerId, TransportError, TransportEvent},
    Transport as CoreTransport,
};

#[derive(Debug, Clone, Default)]
pub struct ResolverConfig;

#[derive(Debug, Clone, Default)]
pub struct ResolverOpts;

pub mod tokio {
    /// A transport wrapper that resolves ordinary DNS multiaddrs before dialing.
    pub type Transport<T> = crate::Transport<T>;

    impl<T> Transport<T> {
        /// Create a DNS transport using the system resolver configuration.
        pub fn system(inner: T) -> Result<Transport<T>, std::io::Error> {
            Ok(Transport::new(inner))
        }

        /// Create a DNS transport using an explicit resolver configuration.
        ///
        /// The local patch keeps the rust-libp2p 0.56 API compatible while
        /// avoiding Hickory. The supplied config values are intentionally unused
        /// because ordinary DNS uses Tokio's OS resolver and `/dnsaddr` is
        /// handled by p2p-net's own configurable pre-resolver.
        pub fn custom(
            inner: T,
            _cfg: crate::ResolverConfig,
            _opts: crate::ResolverOpts,
        ) -> Transport<T> {
            Transport::new(inner)
        }
    }
}

#[derive(Debug)]
pub struct Transport<T> {
    inner: Arc<Mutex<T>>,
}

impl<T> Transport<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

impl<T> Clone for Transport<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> CoreTransport for Transport<T>
where
    T: CoreTransport + Send + Unpin + 'static,
    T::Dial: Send + 'static,
{
    type Output = T::Output;
    type Error = Error;
    type ListenerUpgrade = future::MapErr<T::ListenerUpgrade, fn(T::Error) -> Self::Error>;
    type Dial = future::Either<
        future::MapErr<T::Dial, fn(T::Error) -> Self::Error>,
        future::BoxFuture<'static, Result<Self::Output, Self::Error>>,
    >;

    fn listen_on(
        &mut self,
        id: ListenerId,
        addr: Multiaddr,
    ) -> Result<(), TransportError<Self::Error>> {
        self.inner
            .lock()
            .expect("dns transport mutex poisoned")
            .listen_on(id, addr)
            .map_err(|e| e.map(|_| Error::Transport))
    }

    fn remove_listener(&mut self, id: ListenerId) -> bool {
        self.inner
            .lock()
            .expect("dns transport mutex poisoned")
            .remove_listener(id)
    }

    fn dial(
        &mut self,
        addr: Multiaddr,
        dial_opts: DialOpts,
    ) -> Result<Self::Dial, TransportError<Self::Error>> {
        if !contains_dns(&addr) {
            let dial = self
                .inner
                .lock()
                .expect("dns transport mutex poisoned")
                .dial(addr, dial_opts)
                .map_err(|e| e.map(|_| Error::Transport))?;
            return Ok(future::Either::Left(dial.map_err(|_| Error::Transport)));
        }

        let inner = self.inner.clone();
        Ok(future::Either::Right(
            async move {
                let resolved = resolve_dns_addr(addr).await?;
                let mut last_error = None;

                for candidate in resolved {
                    let dial = inner
                        .lock()
                        .expect("dns transport mutex poisoned")
                        .dial(candidate.clone(), dial_opts.clone())
                        .map_err(|e| match e {
                            TransportError::MultiaddrNotSupported(a) => {
                                Error::TransportAddressUnsupported(a)
                            }
                            TransportError::Other(_) => Error::Transport,
                        });

                    match dial {
                        Ok(dial) => match dial.await.map_err(|_| Error::Transport) {
                            Ok(output) => return Ok(output),
                            Err(err) => last_error = Some(err),
                        },
                        Err(err) => last_error = Some(err),
                    }
                }

                Err(last_error.unwrap_or_else(|| Error::NoRecords("no DNS candidates".to_string())))
            }
            .boxed(),
        ))
    }

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<TransportEvent<Self::ListenerUpgrade, Self::Error>> {
        let mut inner = self.inner.lock().expect("dns transport mutex poisoned");
        CoreTransport::poll(Pin::new(inner.deref_mut()), cx).map(|event| {
            event
                .map_upgrade(|upgr| upgr.map_err::<_, fn(_) -> _>(|_| Error::Transport))
                .map_err(|_| Error::Transport)
        })
    }
}

#[derive(Debug)]
pub enum Error {
    /// The wrapped transport returned an error. The concrete inner error type is
    /// intentionally erased so this shim matches libp2p 0.56 builder bounds.
    Transport,
    Resolve(String),
    NoRecords(String),
    DnsaddrRequiresConfiguredPreresolver(Multiaddr),
    TransportAddressUnsupported(Multiaddr),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport => write!(f, "transport error"),
            Self::Resolve(e) => write!(f, "DNS resolution failed: {e}"),
            Self::NoRecords(host) => write!(f, "DNS resolution returned no usable records for {host}"),
            Self::DnsaddrRequiresConfiguredPreresolver(addr) => write!(
                f,
                "/dnsaddr requires p2p-net's configured pre-resolver before transport dialing: {addr}"
            ),
            Self::TransportAddressUnsupported(addr) => {
                write!(f, "resolved address is not supported by the transport: {addr}")
            }
        }
    }
}

impl error::Error for Error {}

fn contains_dns(addr: &Multiaddr) -> bool {
    addr.iter().any(|p| {
        matches!(
            p,
            Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_) | Protocol::Dnsaddr(_)
        )
    })
}

#[derive(Debug, Clone, Copy)]
enum DnsFamily {
    Any,
    V4,
    V6,
}

async fn resolve_dns_addr(addr: Multiaddr) -> Result<Vec<Multiaddr>, Error> {
    if addr.iter().any(|p| matches!(p, Protocol::Dnsaddr(_))) {
        return Err(Error::DnsaddrRequiresConfiguredPreresolver(addr));
    }

    Ok(vec![resolve_ordinary_dns_addr(addr).await?])
}

async fn resolve_ordinary_dns_addr(addr: Multiaddr) -> Result<Multiaddr, Error> {
    let mut host = None;
    let mut family = DnsFamily::Any;
    let mut tcp_port = 0u16;

    for protocol in addr.iter() {
        match protocol {
            Protocol::Dns(name) => {
                host = Some(name.to_string());
                family = DnsFamily::Any;
                break;
            }
            Protocol::Dns4(name) => {
                host = Some(name.to_string());
                family = DnsFamily::V4;
                break;
            }
            Protocol::Dns6(name) => {
                host = Some(name.to_string());
                family = DnsFamily::V6;
                break;
            }
            Protocol::Tcp(port) => tcp_port = port,
            _ => {}
        }
    }

    let host = host.ok_or_else(|| Error::NoRecords(addr.to_string()))?;
    let query = (host.as_str(), tcp_port);
    let mut records = ::tokio::net::lookup_host(query)
        .await
        .map_err(|err| Error::Resolve(err.to_string()))?;

    let selected = records.find(|record| match (family, record) {
        (DnsFamily::Any, _) => true,
        (DnsFamily::V4, SocketAddr::V4(_)) => true,
        (DnsFamily::V6, SocketAddr::V6(_)) => true,
        _ => false,
    });

    let selected = selected.ok_or_else(|| Error::NoRecords(host.clone()))?;
    let replacement = match selected {
        SocketAddr::V4(v4) => Protocol::Ip4(Ipv4Addr::from(*v4.ip())),
        SocketAddr::V6(v6) => Protocol::Ip6(Ipv6Addr::from(*v6.ip())),
    };

    Ok(replace_first_plain_dns(addr, replacement))
}

fn replace_first_plain_dns(addr: Multiaddr, replacement: Protocol<'static>) -> Multiaddr {
    let mut out = Multiaddr::empty();
    let mut replaced = false;
    for protocol in addr.iter() {
        if !replaced && matches!(protocol, Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_))
        {
            out.push(replacement.clone());
            replaced = true;
        } else {
            out.push(protocol.acquire());
        }
    }
    out
}
