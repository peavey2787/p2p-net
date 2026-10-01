//! End-to-end proof of p2p-net relay metering over the real Circuit Relay
//! data path. Nothing here injects byte counts: every assertion compares the
//! relay's per-circuit counters with what the endpoints actually read from and
//! wrote to their raw relayed circuit streams (measured below the endpoints'
//! own encryption/multiplexing, i.e. exactly the opaque bytes the relay
//! forwards).

use std::{
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use futures::{
    AsyncReadExt, AsyncWriteExt, StreamExt,
    channel::mpsc,
    io::{AsyncRead, AsyncWrite},
};
use libp2p_core::{
    multiaddr::{Multiaddr, Protocol},
    muxing::StreamMuxerBox,
    transport::{Boxed, MemoryTransport, Transport, choice::OrTransport},
    upgrade,
};
use libp2p_identity::{Keypair, PeerId};
use libp2p_plaintext as plaintext;
use libp2p_relay as relay;
use libp2p_swarm::{
    Config, NetworkBehaviour, StreamProtocol, Swarm, SwarmEvent,
    dial_opts::{DialOpts, PeerCondition},
};

const BLOB: StreamProtocol = StreamProtocol::new("/p2p-net/metering-test/1.0.0");
const STEP: Duration = Duration::from_secs(30);

// ---- raw circuit byte counters -------------------------------------------

/// Bytes an endpoint read from / wrote to one raw relayed circuit stream.
#[derive(Default, Debug)]
struct Wire {
    read: AtomicU64,
    written: AtomicU64,
}

impl Wire {
    fn read(&self) -> u64 {
        self.read.load(Ordering::SeqCst)
    }
    fn written(&self) -> u64 {
        self.written.load(Ordering::SeqCst)
    }
}

struct Counted<C> {
    inner: C,
    wire: Arc<Wire>,
}

impl<C: AsyncRead + Unpin> AsyncRead for Counted<C> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        let read = futures::ready!(Pin::new(&mut self.inner).poll_read(cx, buf))?;
        self.wire.read.fetch_add(read as u64, Ordering::SeqCst);
        Poll::Ready(Ok(read))
    }
}

impl<C: AsyncWrite + Unpin> AsyncWrite for Counted<C> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let written = futures::ready!(Pin::new(&mut self.inner).poll_write(cx, buf))?;
        self.wire
            .written
            .fetch_add(written as u64, Ordering::SeqCst);
        Poll::Ready(Ok(written))
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_close(cx)
    }
}

/// Every relayed connection a client opened or accepted, in creation order.
type Wires = Arc<Mutex<Vec<Arc<Wire>>>>;

// ---- swarms ----------------------------------------------------------------

#[derive(NetworkBehaviour)]
#[behaviour(prelude = "libp2p_swarm::derive_prelude")]
struct Relay {
    relay: relay::Behaviour,
}

#[derive(NetworkBehaviour)]
#[behaviour(prelude = "libp2p_swarm::derive_prelude")]
struct Client {
    relay: relay::client::Behaviour,
    stream: libp2p_stream::Behaviour,
}

fn upgraded<S>(transport: Boxed<S>, key: &Keypair) -> Boxed<(PeerId, StreamMuxerBox)>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    transport
        .upgrade(upgrade::Version::V1)
        .authenticate(plaintext::Config::new(key))
        .multiplex(libp2p_yamux::Config::default())
        .boxed()
}

fn swarm_config() -> Config {
    Config::with_tokio_executor().with_idle_connection_timeout(Duration::from_secs(60))
}

/// A relay node driven on its own task; its relay events are forwarded.
struct RelayNode {
    peer: PeerId,
    addr: Multiaddr,
    events: mpsc::UnboundedReceiver<relay::Event>,
}

fn spawn_relay(config: relay::Config) -> RelayNode {
    let key = Keypair::generate_ed25519();
    let peer = key.public().to_peer_id();
    let mut swarm = Swarm::new(
        upgraded(MemoryTransport::default().boxed(), &key),
        Relay {
            relay: relay::Behaviour::new(peer, config),
        },
        peer,
        swarm_config(),
    );
    let addr = Multiaddr::empty().with(Protocol::Memory(rand::random::<u64>()));
    swarm.listen_on(addr.clone()).unwrap();
    swarm.add_external_address(addr.clone());
    let (tx, events) = mpsc::unbounded();
    tokio::spawn(async move {
        loop {
            if let SwarmEvent::Behaviour(RelayEvent::Relay(event)) = swarm.select_next_some().await
            {
                let _ = tx.unbounded_send(event);
            }
        }
    });
    RelayNode { peer, addr, events }
}

enum Cmd {
    Dial(DialOpts),
    Disconnect(PeerId),
    Shutdown,
}

#[derive(Debug)]
enum ClientNote {
    Reserved,
    Connected(PeerId),
    Closed(PeerId),
}

/// A client driven on its own task.
struct ClientNode {
    peer: PeerId,
    control: libp2p_stream::Control,
    wires: Wires,
    cmd: mpsc::UnboundedSender<Cmd>,
    notes: mpsc::UnboundedReceiver<ClientNote>,
}

impl ClientNode {
    async fn next_note(&mut self) -> ClientNote {
        tokio::time::timeout(STEP, self.notes.next())
            .await
            .expect("client event within step timeout")
            .expect("client task alive")
    }

    async fn wait_connected(&mut self, peer: PeerId) {
        loop {
            if let ClientNote::Connected(p) = self.next_note().await
                && p == peer
            {
                return;
            }
        }
    }

    async fn wait_closed(&mut self, peer: PeerId) {
        loop {
            if let ClientNote::Closed(p) = self.next_note().await
                && p == peer
            {
                return;
            }
        }
    }

    fn wire(&self, index: usize) -> Arc<Wire> {
        self.wires.lock().unwrap()[index].clone()
    }
}

fn spawn_client(listen: Option<Multiaddr>) -> ClientNode {
    let key = Keypair::generate_ed25519();
    let peer = key.public().to_peer_id();
    let wires: Wires = Arc::default();
    let (relay_transport, relay_behaviour) = relay::client::new(peer);
    let tap = wires.clone();
    let metered = relay_transport.map(move |connection, _| {
        let wire = Arc::new(Wire::default());
        tap.lock().unwrap().push(wire.clone());
        Counted {
            inner: connection,
            wire,
        }
    });
    let transport = upgraded(
        OrTransport::new(metered, MemoryTransport::default()).boxed(),
        &key,
    );
    let stream = libp2p_stream::Behaviour::new();
    let control = stream.new_control();
    let mut swarm = Swarm::new(
        transport,
        Client {
            relay: relay_behaviour,
            stream,
        },
        peer,
        swarm_config(),
    );
    let (cmd, mut commands) = mpsc::unbounded::<Cmd>();
    let (note_tx, notes) = mpsc::unbounded();
    tokio::spawn(async move {
        if let Some(addr) = listen {
            swarm.listen_on(addr).unwrap();
        }
        loop {
            tokio::select! {
                command = commands.next() => match command {
                    Some(Cmd::Dial(opts)) => swarm.dial(opts).unwrap(),
                    Some(Cmd::Disconnect(peer)) => { let _ = swarm.disconnect_peer_id(peer); }
                    Some(Cmd::Shutdown) | None => return,
                },
                event = swarm.select_next_some() => {
                    let note = match event {
                        SwarmEvent::Behaviour(ClientEvent::Relay(
                            relay::client::Event::ReservationReqAccepted { .. },
                        )) => Some(ClientNote::Reserved),
                        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                            Some(ClientNote::Connected(peer_id))
                        }
                        SwarmEvent::ConnectionClosed { peer_id, .. } => {
                            Some(ClientNote::Closed(peer_id))
                        }
                        _ => None,
                    };
                    if let Some(note) = note {
                        let _ = note_tx.unbounded_send(note);
                    }
                }
            }
        }
    });
    ClientNode {
        peer,
        control,
        wires,
        cmd,
        notes,
    }
}

fn circuit_addr(relay: &RelayNode, dst: PeerId) -> Multiaddr {
    relay
        .addr
        .clone()
        .with(Protocol::P2p(relay.peer))
        .with(Protocol::P2pCircuit)
        .with(Protocol::P2p(dst))
}

/// A destination client holding a reservation on `relay`.
async fn reserved_destination(relay: &RelayNode) -> ClientNode {
    let mut dst = spawn_client_listening(relay);
    loop {
        if let ClientNote::Reserved = dst.next_note().await {
            return dst;
        }
    }
}

fn spawn_client_listening(relay: &RelayNode) -> ClientNode {
    // Listening on `<relay>/p2p-circuit` requests a reservation.
    let listen = relay
        .addr
        .clone()
        .with(Protocol::P2p(relay.peer))
        .with(Protocol::P2pCircuit);
    spawn_client(Some(listen))
}

// ---- traffic ---------------------------------------------------------------

/// `src` sends `forward` bytes to `dst` and `dst` answers with `back` bytes on
/// the same stream; both sides read to EOF.
async fn exchange(src: &ClientNode, dst: &ClientNode, forward: usize, back: usize) {
    let mut incoming = dst.control.clone().accept(BLOB).unwrap();
    let responder = tokio::spawn(async move {
        let (_, mut stream) = incoming.next().await.expect("inbound blob stream");
        let mut received = Vec::new();
        stream.read_to_end(&mut received).await.unwrap();
        stream.write_all(&vec![0xB0; back]).await.unwrap();
        stream.close().await.unwrap();
        received.len()
    });
    let mut stream = src
        .control
        .clone()
        .open_stream(dst.peer, BLOB)
        .await
        .expect("open blob stream over the relayed connection");
    stream.write_all(&vec![0xA0; forward]).await.unwrap();
    stream.close().await.unwrap();
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.unwrap();
    assert_eq!(responder.await.unwrap(), forward);
    assert_eq!(answer.len(), back);
}

struct Opened {
    circuit_id: relay::CircuitId,
    usage: Arc<relay::CircuitUsage>,
}

struct Closed {
    circuit_id: relay::CircuitId,
    src: PeerId,
    dst: PeerId,
    bytes: relay::CircuitBytes,
    error: Option<std::io::Error>,
}

async fn next_opened(relay: &mut RelayNode) -> Opened {
    loop {
        let event = tokio::time::timeout(STEP, relay.events.next())
            .await
            .expect("relay event within step timeout")
            .expect("relay task alive");
        if let relay::Event::CircuitReqAccepted {
            circuit_id, usage, ..
        } = event
        {
            return Opened { circuit_id, usage };
        }
    }
}

async fn next_closed(relay: &mut RelayNode) -> Closed {
    loop {
        let event = tokio::time::timeout(STEP, relay.events.next())
            .await
            .expect("relay event within step timeout")
            .expect("relay task alive");
        if let relay::Event::CircuitClosed {
            circuit_id,
            src_peer_id,
            dst_peer_id,
            bytes,
            error,
        } = event
        {
            return Closed {
                circuit_id,
                src: src_peer_id,
                dst: dst_peer_id,
                bytes,
                error,
            };
        }
    }
}

fn large_quota() -> relay::Config {
    relay::Config {
        max_circuit_bytes: 0, // unlimited
        max_circuit_duration: Duration::from_secs(600),
        ..Default::default()
    }
}

/// Dial `dst` over the relay and wait until both ends see the connection.
async fn relayed_connection(relay: &RelayNode, src: &mut ClientNode, dst: &mut ClientNode) {
    src.cmd
        .unbounded_send(Cmd::Dial(
            DialOpts::unknown_peer_id()
                .address(circuit_addr(relay, dst.peer))
                .build(),
        ))
        .unwrap();
    let (src_peer, dst_peer) = (src.peer, dst.peer);
    futures::join!(src.wait_connected(dst_peer), dst.wait_connected(src_peer));
}

/// Gracefully close the relayed connection from the source side.
async fn close(src: &mut ClientNode, dst: &mut ClientNode) {
    src.cmd.unbounded_send(Cmd::Disconnect(dst.peer)).unwrap();
    let (src_peer, dst_peer) = (src.peer, dst.peer);
    futures::join!(src.wait_closed(dst_peer), dst.wait_closed(src_peer));
}

fn assert_exact(closed: &Closed, src_wire: &Wire, dst_wire: &Wire) {
    // What the relay billed toward each side is exactly what that side read.
    assert_eq!(
        closed.bytes.src_to_dst,
        dst_wire.read(),
        "src->dst vs destination read"
    );
    assert_eq!(
        closed.bytes.dst_to_src,
        src_wire.read(),
        "dst->src vs source read"
    );
    // And on a clean close it is also exactly what the other side wrote.
    assert_eq!(
        closed.bytes.src_to_dst,
        src_wire.written(),
        "src->dst vs source wrote"
    );
    assert_eq!(
        closed.bytes.dst_to_src,
        dst_wire.written(),
        "dst->src vs destination wrote"
    );
    assert_eq!(
        closed.bytes.total(),
        dst_wire.read() + src_wire.read(),
        "transit counts each byte once"
    );
}

// ---- tests -----------------------------------------------------------------

#[tokio::test]
async fn bidirectional_traffic_is_metered_exactly_per_direction() {
    let mut relay = spawn_relay(large_quota());
    let mut dst = reserved_destination(&relay).await;
    let mut src = spawn_client(None);
    relayed_connection(&relay, &mut src, &mut dst).await;
    let opened = next_opened(&mut relay).await;

    const FORWARD: usize = 900 * 1024;
    const BACK: usize = 300 * 1024;
    exchange(&src, &dst, FORWARD, BACK).await;
    let live = opened.usage.snapshot();
    assert!(
        live.src_to_dst >= FORWARD as u64,
        "live usage covers the payload"
    );
    assert!(live.dst_to_src >= BACK as u64);

    close(&mut src, &mut dst).await;
    let closed = next_closed(&mut relay).await;
    assert_eq!(closed.circuit_id, opened.circuit_id);
    assert_eq!((closed.src, closed.dst), (src.peer, dst.peer));
    assert!(closed.error.is_none(), "clean close: {:?}", closed.error);
    assert_exact(&closed, &src.wire(0), &dst.wire(0));
    // The live handle converges on the same final totals.
    assert_eq!(opened.usage.snapshot(), closed.bytes);
    // Payload is inside the metered bytes, never double counted.
    assert!(closed.bytes.src_to_dst >= FORWARD as u64);
    assert!(closed.bytes.dst_to_src >= BACK as u64);
    assert!(closed.bytes.total() < 2 * (FORWARD + BACK) as u64);
}

#[tokio::test]
async fn sequential_circuits_sum_to_the_aggregate() {
    let mut relay = spawn_relay(large_quota());
    let mut dst = reserved_destination(&relay).await;
    let mut aggregate = 0u64;
    let mut endpoint_total = 0u64;
    for (round, size) in [16 * 1024, 64 * 1024, 4 * 1024].into_iter().enumerate() {
        let mut src = spawn_client(None);
        relayed_connection(&relay, &mut src, &mut dst).await;
        let opened = next_opened(&mut relay).await;
        exchange(&src, &dst, size, size / 2).await;
        close(&mut src, &mut dst).await;
        let closed = next_closed(&mut relay).await;
        assert_eq!(closed.circuit_id, opened.circuit_id);
        assert_exact(&closed, &src.wire(0), &dst.wire(round));
        aggregate += closed.bytes.total();
        endpoint_total += dst.wire(round).read() + src.wire(0).read();
        let _ = src.cmd.unbounded_send(Cmd::Shutdown);
    }
    assert_eq!(aggregate, endpoint_total);
}

#[tokio::test]
async fn concurrent_circuits_between_the_same_peers_stay_isolated() {
    let mut relay = spawn_relay(large_quota());
    let mut dst = reserved_destination(&relay).await;
    let mut src = spawn_client(None);
    // Two circuits between the same pair of peers.
    relayed_connection(&relay, &mut src, &mut dst).await;
    src.cmd
        .unbounded_send(Cmd::Dial(
            DialOpts::peer_id(dst.peer)
                .addresses(vec![circuit_addr(&relay, dst.peer)])
                .condition(PeerCondition::Always)
                .build(),
        ))
        .unwrap();
    let (src_peer, dst_peer) = (src.peer, dst.peer);
    futures::join!(src.wait_connected(dst_peer), dst.wait_connected(src_peer));
    let first = next_opened(&mut relay).await;
    let second = next_opened(&mut relay).await;
    assert_ne!(first.circuit_id, second.circuit_id, "distinct circuit ids");

    exchange(&src, &dst, 128 * 1024, 32 * 1024).await;
    exchange(&src, &dst, 8 * 1024, 2 * 1024).await;

    src.cmd.unbounded_send(Cmd::Disconnect(dst.peer)).unwrap();
    let a = next_closed(&mut relay).await;
    let b = next_closed(&mut relay).await;
    let mut ids = [a.circuit_id, b.circuit_id];
    ids.sort();
    let mut expected = [first.circuit_id, second.circuit_id];
    expected.sort();
    assert_eq!(ids, expected);

    // Each circuit's bytes match exactly one connection's raw wire.
    let src_wires = src.wires.lock().unwrap().clone();
    let dst_wires = dst.wires.lock().unwrap().clone();
    for closed in [&a, &b] {
        let matched = src_wires.iter().zip(dst_wires.iter()).filter(|(s, d)| {
            closed.bytes.src_to_dst == d.read() && closed.bytes.dst_to_src == s.read()
        });
        assert_eq!(
            matched.count(),
            1,
            "circuit {:?} matches one connection",
            closed.circuit_id
        );
    }
    let billed: u64 = [&a, &b].iter().map(|c| c.bytes.total()).sum();
    let seen: u64 = src_wires
        .iter()
        .chain(dst_wires.iter())
        .map(|w| w.read())
        .sum();
    assert_eq!(billed, seen);
}

#[tokio::test]
async fn byte_quota_termination_reports_the_bytes_counted_toward_it() {
    const QUOTA: u64 = 8 * 1024;
    let mut relay = spawn_relay(relay::Config {
        max_circuit_bytes: QUOTA,
        max_circuit_duration: Duration::from_secs(600),
        ..Default::default()
    });
    let mut dst = reserved_destination(&relay).await;
    let mut src = spawn_client(None);
    relayed_connection(&relay, &mut src, &mut dst).await;
    let opened = next_opened(&mut relay).await;

    let control = src.control.clone();
    let dst_peer = dst.peer;
    let _incoming = dst.control.clone().accept(BLOB).unwrap();
    tokio::spawn(async move {
        if let Ok(mut stream) = control.clone().open_stream(dst_peer, BLOB).await {
            let _ = stream.write_all(&vec![0xC0; 256 * 1024]).await;
        }
    });

    let closed = next_closed(&mut relay).await;
    assert_eq!(closed.circuit_id, opened.circuit_id);
    let error = closed.error.expect("quota closes with an error");
    assert_eq!(error.to_string(), "Max circuit bytes reached.");
    // The quota tripped on the billed counter, which exceeds it.
    assert!(closed.bytes.total() > QUOTA);
    assert_eq!(opened.usage.snapshot(), closed.bytes);
    // Nothing reached the destination beyond what was billed toward it.
    assert!(dst.wire(0).read() <= closed.bytes.src_to_dst);
}

#[tokio::test]
async fn duration_limit_termination_reports_final_totals() {
    let mut relay = spawn_relay(relay::Config {
        max_circuit_bytes: 0,
        max_circuit_duration: Duration::from_secs(2),
        ..Default::default()
    });
    let mut dst = reserved_destination(&relay).await;
    let mut src = spawn_client(None);
    relayed_connection(&relay, &mut src, &mut dst).await;
    let opened = next_opened(&mut relay).await;
    exchange(&src, &dst, 32 * 1024, 4 * 1024).await;

    let closed = next_closed(&mut relay).await;
    assert_eq!(closed.circuit_id, opened.circuit_id);
    let error = closed.error.expect("duration limit closes with an error");
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert_eq!(opened.usage.snapshot(), closed.bytes);
    assert!(closed.bytes.src_to_dst >= 32 * 1024);
    assert!(dst.wire(0).read() <= closed.bytes.src_to_dst);
    assert!(src.wire(0).read() <= closed.bytes.dst_to_src);
}

#[tokio::test]
async fn rejected_circuit_accrues_no_bytes() {
    let mut relay = spawn_relay(large_quota());
    // Destination has no reservation: the relay denies the circuit.
    let unreserved = spawn_client(None);
    let src = spawn_client(None);
    src.cmd
        .unbounded_send(Cmd::Dial(
            DialOpts::unknown_peer_id()
                .address(circuit_addr(&relay, unreserved.peer))
                .build(),
        ))
        .unwrap();
    loop {
        let event = tokio::time::timeout(STEP, relay.events.next())
            .await
            .expect("relay event")
            .expect("relay alive");
        match event {
            relay::Event::CircuitReqDenied { dst_peer_id, .. } => {
                assert_eq!(dst_peer_id, unreserved.peer);
                break;
            }
            relay::Event::CircuitReqAccepted { .. } | relay::Event::CircuitClosed { .. } => {
                panic!("a denied circuit must never be accepted or metered: {event:?}")
            }
            _ => {}
        }
    }
    assert!(
        src.wires.lock().unwrap().is_empty(),
        "no relayed stream was created"
    );
}

#[tokio::test]
async fn abrupt_disconnect_still_reports_exact_relay_totals() {
    let mut relay = spawn_relay(large_quota());
    let mut dst = reserved_destination(&relay).await;
    let mut src = spawn_client(None);
    relayed_connection(&relay, &mut src, &mut dst).await;
    let opened = next_opened(&mut relay).await;
    exchange(&src, &dst, 64 * 1024, 16 * 1024).await;
    let before_crash = opened.usage.snapshot();

    // The destination vanishes without closing anything.
    dst.cmd.unbounded_send(Cmd::Shutdown).unwrap();
    let closed = next_closed(&mut relay).await;
    assert_eq!(closed.circuit_id, opened.circuit_id);
    assert!(
        closed.error.is_some(),
        "abrupt loss is reported as an error"
    );
    assert_eq!(opened.usage.snapshot(), closed.bytes);
    // Totals never go backwards and cover everything delivered before.
    assert!(closed.bytes.src_to_dst >= before_crash.src_to_dst);
    assert!(closed.bytes.dst_to_src >= before_crash.dst_to_src);
    assert!(closed.bytes.src_to_dst >= dst.wire(0).read());
    assert_eq!(closed.bytes.dst_to_src, src.wire(0).read());
}

/// p2p-net regression: a relayed dial issued while the connection to the
/// relay is still being dialed must wait for it rather than be canceled.
#[tokio::test]
async fn circuit_dial_during_pending_relay_dial_succeeds() {
    let mut relay = spawn_relay(large_quota());
    let mut dst = reserved_destination(&relay).await;
    let mut src = spawn_client(None);
    // Dial the relay directly and, before that completes, dial through it.
    src.cmd
        .unbounded_send(Cmd::Dial(
            DialOpts::peer_id(relay.peer)
                .addresses(vec![relay.addr.clone()])
                .build(),
        ))
        .unwrap();
    relayed_connection(&relay, &mut src, &mut dst).await;
    let opened = next_opened(&mut relay).await;
    exchange(&src, &dst, 4 * 1024, 1024).await;
    close(&mut src, &mut dst).await;
    let closed = next_closed(&mut relay).await;
    assert_eq!(closed.circuit_id, opened.circuit_id);
    assert_exact(&closed, &src.wire(0), &dst.wire(0));
}
