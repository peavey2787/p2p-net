//! No-Hickory DNS transport adapter used by the WebSocket transport.
//!
//! `/dns`, `/dns4`, and `/dns6` use Tokio's OS-backed resolver. `/dnsaddr`
//! stays in p2p-net's configurable pre-resolver so the transport never embeds
//! an implicit third-party DoH resolver.

use std::{
    error, fmt,
    net::SocketAddr,
    ops::DerefMut,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use futures::{future, FutureExt, TryFutureExt};
use libp2p::core::{
    multiaddr::{Multiaddr, Protocol},
    transport::{DialOpts, ListenerId, TransportError, TransportEvent},
    Transport as CoreTransport,
};

/// Transport wrapper that resolves ordinary DNS multiaddrs with the OS resolver.
#[derive(Debug)]
pub(super) struct OsDnsTransport<T> {
    inner: Arc<Mutex<T>>,
}

impl<T> OsDnsTransport<T> {
    pub(super) fn new(inner: T) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

impl<T> Clone for OsDnsTransport<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> CoreTransport for OsDnsTransport<T>
where
    T: CoreTransport + Send + Unpin + 'static,
    T::Dial: Send + 'static,
    T::Error: error::Error + Send + 'static,
{
    type Output = T::Output;
    type Error = DnsTransportError<T::Error>;
    type ListenerUpgrade =
        future::MapErr<T::ListenerUpgrade, fn(T::Error) -> DnsTransportError<T::Error>>;
    type Dial = future::Either<
        future::MapErr<T::Dial, fn(T::Error) -> DnsTransportError<T::Error>>,
        future::BoxFuture<'static, Result<Self::Output, Self::Error>>,
    >;

    fn listen_on(
        &mut self,
        id: ListenerId,
        addr: Multiaddr,
    ) -> Result<(), TransportError<Self::Error>> {
        self.inner
            .lock()
            .expect("DNS transport mutex poisoned")
            .listen_on(id, addr)
            .map_err(|err| err.map(DnsTransportError::Transport))
    }

    fn remove_listener(&mut self, id: ListenerId) -> bool {
        self.inner
            .lock()
            .expect("DNS transport mutex poisoned")
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
                .expect("DNS transport mutex poisoned")
                .dial(addr, dial_opts)
                .map_err(|err| err.map(DnsTransportError::Transport))?;
            return Ok(future::Either::Left(
                dial.map_err(DnsTransportError::Transport),
            ));
        }

        let inner = self.inner.clone();
        Ok(future::Either::Right(
            async move {
                let resolved = resolve_dns_addr(addr).await?;
                let mut last_error = None;

                for candidate in resolved {
                    let dial = inner
                        .lock()
                        .expect("DNS transport mutex poisoned")
                        .dial(candidate, dial_opts)
                        .map_err(|err| match err {
                            TransportError::MultiaddrNotSupported(addr) => {
                                DnsTransportError::TransportAddressUnsupported(addr)
                            }
                            TransportError::Other(err) => DnsTransportError::Transport(err),
                        });

                    match dial {
                        Ok(dial) => match dial.await.map_err(DnsTransportError::Transport) {
                            Ok(output) => return Ok(output),
                            Err(err) => last_error = Some(err),
                        },
                        Err(err) => last_error = Some(err),
                    }
                }

                Err(last_error.unwrap_or_else(|| {
                    DnsTransportError::NoRecords("no DNS candidates".to_string())
                }))
            }
            .boxed(),
        ))
    }

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<TransportEvent<Self::ListenerUpgrade, Self::Error>> {
        let mut inner = self.inner.lock().expect("DNS transport mutex poisoned");
        CoreTransport::poll(Pin::new(inner.deref_mut()), cx).map(|event| {
            event
                .map_upgrade(|upgrade| {
                    upgrade.map_err::<_, fn(T::Error) -> DnsTransportError<T::Error>>(
                        DnsTransportError::Transport,
                    )
                })
                .map_err(DnsTransportError::Transport)
        })
    }
}

#[derive(Debug)]
pub(super) enum DnsTransportError<E> {
    Transport(E),
    Resolve(String),
    NoRecords(String),
    DnsaddrRequiresConfiguredPreresolver(Multiaddr),
    TransportAddressUnsupported(Multiaddr),
}

impl<E: fmt::Display> fmt::Display for DnsTransportError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(err) => write!(f, "{err}"),
            Self::Resolve(err) => write!(f, "DNS resolution failed: {err}"),
            Self::NoRecords(host) => {
                write!(f, "DNS resolution returned no usable records for {host}")
            }
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

impl<E: error::Error + 'static> error::Error for DnsTransportError<E> {}

fn contains_dns(addr: &Multiaddr) -> bool {
    addr.iter().any(|protocol| {
        matches!(
            protocol,
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

async fn resolve_dns_addr<E>(addr: Multiaddr) -> Result<Vec<Multiaddr>, DnsTransportError<E>> {
    if addr
        .iter()
        .any(|protocol| matches!(protocol, Protocol::Dnsaddr(_)))
    {
        return Err(DnsTransportError::DnsaddrRequiresConfiguredPreresolver(
            addr,
        ));
    }

    Ok(vec![resolve_ordinary_dns_addr(addr).await?])
}

async fn resolve_ordinary_dns_addr<E>(addr: Multiaddr) -> Result<Multiaddr, DnsTransportError<E>> {
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

    let host = host.ok_or_else(|| DnsTransportError::NoRecords(addr.to_string()))?;
    let selected = {
        let mut records = tokio::net::lookup_host((host.as_str(), tcp_port))
            .await
            .map_err(|err| DnsTransportError::Resolve(err.to_string()))?;

        records.find(|record| {
            matches!(
                (family, record),
                (DnsFamily::Any, _)
                    | (DnsFamily::V4, SocketAddr::V4(_))
                    | (DnsFamily::V6, SocketAddr::V6(_))
            )
        })
    };

    let selected = selected.ok_or_else(|| DnsTransportError::NoRecords(host))?;
    let replacement = match selected {
        SocketAddr::V4(v4) => Protocol::Ip4(*v4.ip()),
        SocketAddr::V6(v6) => Protocol::Ip6(*v6.ip()),
    };

    Ok(replace_first_plain_dns(addr, replacement))
}

fn replace_first_plain_dns(addr: Multiaddr, replacement: Protocol<'static>) -> Multiaddr {
    let mut out = Multiaddr::empty();
    let mut replaced = false;
    for protocol in addr.iter() {
        if !replaced
            && matches!(
                protocol,
                Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_)
            )
        {
            out.push(replacement.clone());
            replaced = true;
        } else {
            out.push(protocol.acquire());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    #[test]
    fn plain_dns_detection_is_bounded_to_dns_protocols() {
        let dns: Multiaddr = "/dns4/example.com/tcp/443/wss".parse().unwrap();
        let ip: Multiaddr = "/ip4/127.0.0.1/tcp/443/ws".parse().unwrap();
        assert!(contains_dns(&dns));
        assert!(!contains_dns(&ip));
    }

    #[test]
    fn replacement_preserves_the_remaining_transport_stack() {
        let addr: Multiaddr = "/dns4/example.com/tcp/443/wss".parse().unwrap();
        let replaced = replace_first_plain_dns(addr, Protocol::Ip4(Ipv4Addr::new(192, 0, 2, 42)));
        assert_eq!(replaced.to_string(), "/ip4/192.0.2.42/tcp/443/wss");
    }
}
