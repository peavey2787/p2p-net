use std::{
    convert::Infallible,
    net::{SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

use futures::future::Either;
use rand::{distributions, Rng};

use crate::{provider::Provider, Error};

// The paired dialer fans out 128 legitimate QUIC endpoints. Sampling 4,096
// remote ports gives the two sets a >99.96% chance of intersection even when
// a symmetric NAT assigns unpredictable public ports. This is bounded to one
// authenticated application peer and avoids an exhaustive 65,535-port sweep.
const PORT_GUESSES: u32 = 4_096;
const PORT_GUESS_STEP: u32 = 251;
const PORT_SCAN_WARMUP: Duration = Duration::from_millis(50);
const PORT_SCAN_REFRESH: Duration = Duration::from_millis(100);
const PORT_SCAN_PASSES: u32 = 3;

pub(crate) async fn hole_puncher<P: Provider>(
    socket: UdpSocket,
    remote_addr: SocketAddr,
    timeout_duration: Duration,
) -> Error {
    let local_addr = socket.local_addr().ok();
    tracing::debug!(
        target: "p2p_net::event",
        event = format_args!(
            "quic dcutr listener punch started local={local_addr:?} remote={remote_addr}"
        )
    );
    let socket = match P::wrap_udp_socket(socket) {
        Ok(socket) => socket,
        Err(error) => return Error::from(error),
    };
    let punch_holes_future = punch_holes::<P>(socket, local_addr, remote_addr, timeout_duration);
    futures::pin_mut!(punch_holes_future);
    match futures::future::select(P::sleep(timeout_duration), punch_holes_future).await {
        Either::Left(_) => Error::HandshakeTimedOut,
        Either::Right((Err(hole_punch_err), _)) => hole_punch_err,
        Either::Right((Ok(never), _)) => match never {},
    }
}

async fn punch_holes<P: Provider>(
    socket: P::AsyncUdpSocket,
    local_addr: Option<SocketAddr>,
    remote_addr: SocketAddr,
    timeout_duration: Duration,
) -> Result<Infallible, Error> {
    let first_port = rand::thread_rng().gen_range(1..=u16::MAX);
    let contents: Vec<u8> = rand::thread_rng()
        .sample_iter(distributions::Standard)
        .take(64)
        .collect();

    // Give the remote dialer a brief head start to create its fan-out mappings,
    // then repeat the same bounded permutation so a packet lost during mapping
    // creation cannot make an otherwise punchable attempt fail. The step is
    // coprime with 65,535, so no port repeats within a pass.
    let started = Instant::now();
    let deadline = started + timeout_duration.saturating_sub(Duration::from_secs(1));
    let mut ports_sent = 0_u32;
    let mut send_errors = 0_u32;
    P::sleep(PORT_SCAN_WARMUP).await;
    'passes: for _ in 0..PORT_SCAN_PASSES {
        for offset in 0..PORT_GUESSES {
            if Instant::now() >= deadline {
                break 'passes;
            }
            let port = ((u32::from(first_port) - 1 + offset.saturating_mul(PORT_GUESS_STEP))
                % u32::from(u16::MAX)
                + 1) as u16;
            let target = SocketAddr::new(remote_addr.ip(), port);
            match P::send_to(&socket, &contents, target).await {
                Ok(_) => ports_sent = ports_sent.saturating_add(1),
                Err(_) => send_errors = send_errors.saturating_add(1),
            }
        }
        // Preserve upstream DCUtR's advertised-port fast path and leave time
        // for the remote QUIC Initial retransmission before refreshing filters.
        P::send_to(&socket, &contents, remote_addr).await?;
        P::sleep(PORT_SCAN_REFRESH).await;
    }
    let scan_elapsed = started.elapsed();
    tracing::debug!(
        target: "p2p_net::event",
        event = format_args!(
            "quic dcutr listener bounded port sampling completed local={:?} remote_ip={} ports_sent={ports_sent} elapsed_ms={} send_errors={send_errors}",
            local_addr,
            remote_addr.ip(),
            scan_elapsed.as_millis()
        )
    );

    loop {
        tracing::trace!("Sending random UDP packet to {remote_addr}");

        P::send_to(&socket, &contents, remote_addr).await?;

        let sleep_duration = Duration::from_millis(rand::thread_rng().gen_range(10..=200));
        P::sleep(sleep_duration).await;
    }
}
