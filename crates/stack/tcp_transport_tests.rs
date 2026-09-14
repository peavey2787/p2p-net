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
