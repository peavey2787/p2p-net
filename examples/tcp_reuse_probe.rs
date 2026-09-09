//! Bounded diagnostic for native TCP listener-port reuse, without a swarm.
//!
//! `cargo run --example tcp_reuse_probe -- LOCAL_PORT MULTIADDR [MULTIADDR ...]`
//! Every supplied target is dialed concurrently from the listening port with
//! a five-second timeout. This diagnoses socket behavior, not DCUtR success.

use std::pin::Pin;
use std::time::Duration;

use futures::future::{join_all, poll_fn};
use libp2p::core::transport::{DialOpts, ListenerId, PortUse, TransportEvent};
use libp2p::core::Endpoint;
use libp2p::{tcp, Multiaddr, Transport};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let port = args
        .next()
        .ok_or("missing local TCP port")?
        .parse::<u16>()?;
    let targets = args
        .map(|a| a.parse::<Multiaddr>())
        .collect::<Result<Vec<_>, _>>()?;
    if targets.is_empty() || targets.len() > 8 {
        return Err("supply between one and eight TCP target addresses".into());
    }
    let mut transport = tcp::tokio::Transport::new(tcp::Config::default().nodelay(true));
    transport.listen_on(
        ListenerId::next(),
        format!("/ip4/0.0.0.0/tcp/{port}").parse()?,
    )?;
    // Poll interface notifications so the real transport registers its
    // non-loopback listener for reuse before any dial is constructed.
    let ready = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let TransportEvent::NewAddress { listen_addr, .. } =
                poll_fn(|cx| Pin::new(&mut transport).poll(cx)).await
            {
                println!("listen={listen_addr}");
                if listen_addr
                    .iter()
                    .any(|p| matches!(p, libp2p::multiaddr::Protocol::Ip4(ip) if !ip.is_loopback()))
                {
                    return;
                }
            }
        }
    })
    .await;
    if ready.is_err() {
        return Err("no non-loopback TCP listener appeared within five seconds".into());
    }
    let dials = targets.into_iter().map(|addr| {
        let dial = transport.dial(addr.clone(), DialOpts { role: Endpoint::Dialer, port_use: PortUse::Reuse });
        async move {
            match dial {
                Err(error) => { println!("target={addr} setup_error={error:?}"); false }
                Ok(dial) => match tokio::time::timeout(Duration::from_secs(5), dial).await {
                    Err(_) => { println!("target={addr} timeout"); false }
                    Ok(Err(error)) => { println!("target={addr} connect_error={error:?}"); false }
                    Ok(Ok(stream)) => {
                        let local = stream.0.local_addr().ok();
                        let remote = stream.0.peer_addr().ok();
                        let reused = local.is_some_and(|addr| addr.port() == port);
                        println!("target={addr} local={local:?} remote={remote:?} reused={reused}");
                        reused
                    }
                },
            }
        }
    }).collect::<Vec<_>>();
    if join_all(dials).await.into_iter().all(|success| success) {
        Ok(())
    } else {
        Err("at least one TCP connection failed or did not reuse the listener port".into())
    }
}
