// Copyright 2022 Parity Technologies (UK) Ltd.
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

use std::{
    collections::{HashMap, VecDeque},
    io,
    io::ErrorKind,
    net::SocketAddr,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use futures::{
    channel::oneshot,
    future::{BoxFuture, FutureExt, OptionFuture},
    stream::FuturesUnordered,
    StreamExt,
};
use stun::{
    attributes::ATTR_USERNAME,
    message::{is_message as is_stun_message, Message as STUNMessage},
};
use thiserror::Error;
use tokio::{io::ReadBuf, net::UdpSocket};
use webrtc::{
    ice::udp_mux::{UDPMux, UDPMuxConn, UDPMuxConnParams, UDPMuxWriter},
    util::{Conn, Error},
};

use crate::tokio::req_res_chan;

const RECEIVE_MTU: usize = 8192;
// Unknown STUN endpoints are only pre-handshake deduplication state. Keeping them
// forever lets failed/spoofed handshakes grow memory without bound on public
// WebRTC-direct listeners. This does not limit established WebRTC connections.
const MAX_PENDING_NEW_ADDRS: usize = 4096;
const PENDING_NEW_ADDR_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
struct PendingNewAddrs {
    seen: HashMap<SocketAddr, Instant>,
    order: VecDeque<(Instant, SocketAddr)>,
}

impl PendingNewAddrs {
    fn remember(&mut self, addr: SocketAddr) -> bool {
        let now = Instant::now();
        self.prune(now);
        if self.seen.contains_key(&addr) {
            return false;
        }

        self.seen.insert(addr, now);
        self.order.push_back((now, addr));
        self.prune(now);
        true
    }

    fn remove(&mut self, addr: &SocketAddr) {
        if self.seen.remove(addr).is_some() {
            self.order.retain(|(_, queued)| queued != addr);
        }
    }

    fn clear(&mut self) {
        self.seen.clear();
        self.order.clear();
    }

    fn prune(&mut self, now: Instant) {
        while let Some(&(seen_at, addr)) = self.order.front() {
            let is_current = self.seen.get(&addr).copied() == Some(seen_at);
            if !is_current {
                self.order.pop_front();
                continue;
            }

            let expired = now.saturating_duration_since(seen_at) >= PENDING_NEW_ADDR_TTL;
            let over_capacity = self.seen.len() > MAX_PENDING_NEW_ADDRS;
            if !expired && !over_capacity {
                break;
            }

            self.order.pop_front();
            self.seen.remove(&addr);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.seen.len()
    }

    #[cfg(test)]
    fn queued_len(&self) -> usize {
        self.order.len()
    }
}

/// A previously unseen address of a remote which has sent us an ICE binding request.
#[derive(Debug)]
pub(crate) struct NewAddr {
    pub(crate) addr: SocketAddr,
    pub(crate) ufrag: String,
}

/// An event emitted by [`UDPMuxNewAddr`] when it's polled.
#[derive(Debug)]
pub(crate) enum UDPMuxEvent {
    /// Connection error. UDP mux should be stopped.
    Error(std::io::Error),
    /// Got a [`NewAddr`] from the socket.
    NewAddr(NewAddr),
}

/// A modified version of [`webrtc::ice::udp_mux::UDPMuxDefault`].
///
/// - It has been rewritten to work without locks and channels instead.
/// - It reports previously unseen addresses instead of ignoring them.
pub(crate) struct UDPMuxNewAddr {
    udp_sock: UdpSocket,

    listen_addr: SocketAddr,

    /// Maps from ufrag to the underlying connection.
    conns: HashMap<String, UDPMuxConn>,

    /// Maps from socket address to the underlying connection.
    address_map: HashMap<SocketAddr, UDPMuxConn>,

    /// Bounded/expiring pre-handshake address deduplication state.
    new_addrs: PendingNewAddrs,

    /// `true` when UDP mux is closed.
    is_closed: bool,

    send_buffer: Option<(Vec<u8>, SocketAddr, oneshot::Sender<Result<usize, Error>>)>,

    close_futures: FuturesUnordered<BoxFuture<'static, ()>>,
    write_future: OptionFuture<BoxFuture<'static, ()>>,

    close_command: req_res_chan::Receiver<(), Result<(), Error>>,
    get_conn_command: req_res_chan::Receiver<String, Result<Arc<dyn Conn + Send + Sync>, Error>>,
    remove_conn_command: req_res_chan::Receiver<String, ()>,
    registration_command: req_res_chan::Receiver<(UDPMuxConn, SocketAddr), ()>,
    send_command: req_res_chan::Receiver<(Vec<u8>, SocketAddr), Result<usize, Error>>,

    udp_mux_handle: Arc<UdpMuxHandle>,
    udp_mux_writer_handle: Arc<UdpMuxWriterHandle>,
}

impl UDPMuxNewAddr {
    pub(crate) fn listen_on(addr: SocketAddr) -> Result<Self, io::Error> {
        let std_sock = std::net::UdpSocket::bind(addr)?;
        std_sock.set_nonblocking(true)?;

        let tokio_socket = UdpSocket::from_std(std_sock)?;
        let listen_addr = tokio_socket.local_addr()?;

        let (udp_mux_handle, close_command, get_conn_command, remove_conn_command) =
            UdpMuxHandle::new();
        let (udp_mux_writer_handle, registration_command, send_command) = UdpMuxWriterHandle::new();

        Ok(Self {
            udp_sock: tokio_socket,
            listen_addr,
            conns: HashMap::default(),
            address_map: HashMap::default(),
            new_addrs: PendingNewAddrs::default(),
            is_closed: false,
            send_buffer: None,
            close_futures: FuturesUnordered::default(),
            write_future: OptionFuture::default(),
            close_command,
            get_conn_command,
            remove_conn_command,
            registration_command,
            send_command,
            udp_mux_handle: Arc::new(udp_mux_handle),
            udp_mux_writer_handle: Arc::new(udp_mux_writer_handle),
        })
    }

    pub(crate) fn listen_addr(&self) -> SocketAddr {
        self.listen_addr
    }

    pub(crate) fn udp_mux_handle(&self) -> Arc<UdpMuxHandle> {
        self.udp_mux_handle.clone()
    }

    /// Create a muxed connection for a given ufrag.
    fn create_muxed_conn(&self, ufrag: &str) -> Result<UDPMuxConn, Error> {
        let local_addr = self.udp_sock.local_addr()?;

        let params = UDPMuxConnParams {
            local_addr,
            key: ufrag.into(),
            udp_mux: Arc::downgrade(
                &(self.udp_mux_writer_handle.clone() as Arc<dyn UDPMuxWriter + Send + Sync>),
            ),
        };

        Ok(UDPMuxConn::new(params))
    }

    /// Returns a muxed connection if the `ufrag` from the given STUN message matches an existing
    /// connection.
    fn conn_from_stun_message(
        &self,
        buffer: &[u8],
        addr: &SocketAddr,
    ) -> Option<Result<UDPMuxConn, ConnQueryError>> {
        match ufrag_from_stun_message(buffer, true) {
            Ok(ufrag) => {
                if let Some(conn) = self.conns.get(&ufrag) {
                    let associated_addrs = conn.get_addresses();
                    // This basically ensures only one address is registered per ufrag.
                    if associated_addrs.is_empty() || associated_addrs.contains(addr) {
                        return Some(Ok(conn.clone()));
                    } else {
                        return Some(Err(ConnQueryError::UfragAlreadyTaken { associated_addrs }));
                    }
                }
                None
            }
            Err(e) => {
                tracing::debug!(address=%addr, "{}", e);
                None
            }
        }
    }

    /// Reads from the underlying UDP socket and either reports a new address or proxies data to the
    /// muxed connection.
    pub(crate) fn poll(&mut self, cx: &mut Context) -> Poll<UDPMuxEvent> {
        let mut recv_buf = [0u8; RECEIVE_MTU];

        loop {
            // => Send data to target
            match self.send_buffer.take() {
                None => {
                    if let Poll::Ready(Some(((buf, target), response))) =
                        self.send_command.poll_next_unpin(cx)
                    {
                        self.send_buffer = Some((buf, target, response));
                        continue;
                    }
                }
                Some((buf, target, response)) => {
                    match self.udp_sock.poll_send_to(cx, &buf, target) {
                        Poll::Ready(result) => {
                            let _ = response.send(result.map_err(|e| Error::Io(e.into())));
                            continue;
                        }
                        Poll::Pending => {
                            self.send_buffer = Some((buf, target, response));
                        }
                    }
                }
            }

            // => Register a new connection
            if let Poll::Ready(Some(((conn, addr), response))) =
                self.registration_command.poll_next_unpin(cx)
            {
                let key = conn.key();

                self.address_map
                    .entry(addr)
                    .and_modify(|e| {
                        if e.key() != key {
                            e.remove_address(&addr);
                            *e = conn.clone();
                        }
                    })
                    .or_insert_with(|| conn.clone());

                // remove addr from new_addrs once conn is established
                self.new_addrs.remove(&addr);

                let _ = response.send(());

                continue;
            }

            // => Get connection with the given ufrag
            if let Poll::Ready(Some((ufrag, response))) = self.get_conn_command.poll_next_unpin(cx)
            {
                if self.is_closed {
                    let _ = response.send(Err(Error::ErrUseClosedNetworkConn));
                    continue;
                }

                if let Some(conn) = self.conns.get(&ufrag).cloned() {
                    let _ = response.send(Ok(Arc::new(conn)));
                    continue;
                }

                let muxed_conn = match self.create_muxed_conn(&ufrag) {
                    Ok(conn) => conn,
                    Err(e) => {
                        let _ = response.send(Err(e));
                        continue;
                    }
                };
                let mut close_rx = muxed_conn.close_rx();

                self.close_futures.push({
                    let ufrag = ufrag.clone();
                    let udp_mux_handle = self.udp_mux_handle.clone();

                    Box::pin(async move {
                        let _ = close_rx.changed().await;
                        udp_mux_handle.remove_conn_by_ufrag(&ufrag).await;
                    })
                });

                self.conns.insert(ufrag, muxed_conn.clone());

                let _ = response.send(Ok(Arc::new(muxed_conn) as Arc<dyn Conn + Send + Sync>));

                continue;
            }

            // => Close UDPMux
            if let Poll::Ready(Some(((), response))) = self.close_command.poll_next_unpin(cx) {
                if self.is_closed {
                    let _ = response.send(Err(Error::ErrAlreadyClosed));
                    continue;
                }

                for (_, conn) in self.conns.drain() {
                    conn.close();
                }

                // NOTE: This is important, we need to drop all instances of `UDPMuxConn` to
                // avoid a retain cycle due to the use of [`std::sync::Arc`] on both sides.
                self.address_map.clear();

                // NOTE: This is important, we need to drop all instances of `UDPMuxConn` to
                // avoid a retain cycle due to the use of [`std::sync::Arc`] on both sides.
                self.new_addrs.clear();

                let _ = response.send(Ok(()));

                self.is_closed = true;

                continue;
            }

            // => Remove connection with the given ufrag
            if let Poll::Ready(Some((ufrag, response))) =
                self.remove_conn_command.poll_next_unpin(cx)
            {
                // Pion's ice implementation has both `RemoveConnByFrag` and `RemoveConn`, but since
                // `conns` is keyed on `ufrag` their implementation is equivalent.

                if let Some(removed_conn) = self.conns.remove(&ufrag) {
                    for address in removed_conn.get_addresses() {
                        self.address_map.remove(&address);
                    }
                    // Explicit removal is also used by failed/cancelled handshake
                    // cleanup. Close the underlying mux connection so ICE resources
                    // are released rather than only dropping our lookup entry.
                    removed_conn.close();
                }

                let _ = response.send(());

                continue;
            }

            // => Remove all closed connections that are ready. Draining the ready
            // set prevents completed cleanup futures from piling up during churn.
            loop {
                match self.close_futures.poll_next_unpin(cx) {
                    Poll::Ready(Some(())) => continue,
                    Poll::Ready(None) | Poll::Pending => break,
                }
            }

            // => Write previously received data to local connections
            match self.write_future.poll_unpin(cx) {
                Poll::Ready(Some(())) => {
                    self.write_future = OptionFuture::default();
                    continue;
                }
                Poll::Ready(None) => {
                    // => Read from the socket
                    let mut read = ReadBuf::new(&mut recv_buf);

                    match self.udp_sock.poll_recv_from(cx, &mut read) {
                        Poll::Ready(Ok(addr)) => {
                            // Find connection based on previously having seen this source address
                            let conn = self.address_map.get(&addr);

                            let conn = match conn {
                                // If we couldn't find the connection based on source address, see
                                // if this is a STUN message and if
                                // so if we can find the connection based on ufrag.
                                None if is_stun_message(read.filled()) => {
                                    match self.conn_from_stun_message(read.filled(), &addr) {
                                        Some(Ok(s)) => Some(s),
                                        Some(Err(e)) => {
                                            tracing::debug!(address=%&addr, "Error when querying existing connections: {}", e);
                                            continue;
                                        }
                                        None => None,
                                    }
                                }
                                Some(s) => Some(s.to_owned()),
                                _ => None,
                            };

                            match conn {
                                None => {
                                    match ufrag_from_stun_message(read.filled(), false) {
                                        Ok(ufrag) if self.new_addrs.remember(addr) => {
                                            tracing::trace!(
                                                address=%&addr,
                                                %ufrag,
                                                "Notifying about new address from ufrag",
                                            );
                                            return Poll::Ready(UDPMuxEvent::NewAddr(NewAddr {
                                                addr,
                                                ufrag,
                                            }));
                                        }
                                        Ok(_) => {}
                                        Err(e) => {
                                            tracing::debug!(
                                                address=%&addr,
                                                "Unknown address (non STUN packet: {})",
                                                e
                                            );
                                        }
                                    }
                                }
                                Some(conn) => {
                                    let mut packet = vec![0u8; read.filled().len()];
                                    packet.copy_from_slice(read.filled());
                                    self.write_future = OptionFuture::from(Some(
                                        async move {
                                            if let Err(err) = conn.write_packet(&packet, addr).await
                                            {
                                                tracing::error!(
                                                    address=%addr,
                                                    "Failed to write packet: {}",
                                                    err,
                                                );
                                            }
                                        }
                                        .boxed(),
                                    ));
                                }
                            }

                            continue;
                        }
                        Poll::Pending => {}
                        Poll::Ready(Err(err)) if err.kind() == ErrorKind::TimedOut => {}
                        Poll::Ready(Err(err)) if err.kind() == ErrorKind::ConnectionReset => {
                            tracing::debug!("ConnectionReset by remote client {err:?}")
                        }
                        Poll::Ready(Err(err)) => {
                            tracing::error!("Could not read udp packet: {}", err);
                            return Poll::Ready(UDPMuxEvent::Error(err));
                        }
                    }
                }
                Poll::Pending => {}
            }

            return Poll::Pending;
        }
    }
}

/// Handle which utilizes [`req_res_chan`] to transmit commands (e.g. remove connection) from the
/// WebRTC ICE agent to [`UDPMuxNewAddr::poll`].
pub(crate) struct UdpMuxHandle {
    close_sender: req_res_chan::Sender<(), Result<(), Error>>,
    get_conn_sender: req_res_chan::Sender<String, Result<Arc<dyn Conn + Send + Sync>, Error>>,
    remove_sender: req_res_chan::Sender<String, ()>,
}

impl UdpMuxHandle {
    /// Returns a new `UdpMuxHandle` and `close`, `get_conn` and `remove` receivers.
    pub(crate) fn new() -> (
        Self,
        req_res_chan::Receiver<(), Result<(), Error>>,
        req_res_chan::Receiver<String, Result<Arc<dyn Conn + Send + Sync>, Error>>,
        req_res_chan::Receiver<String, ()>,
    ) {
        let (sender1, receiver1) = req_res_chan::new(1);
        let (sender2, receiver2) = req_res_chan::new(1);
        let (sender3, receiver3) = req_res_chan::new(1);

        let this = Self {
            close_sender: sender1,
            get_conn_sender: sender2,
            remove_sender: sender3,
        };

        (this, receiver1, receiver2, receiver3)
    }
}

#[async_trait]
impl UDPMux for UdpMuxHandle {
    async fn close(&self) -> Result<(), Error> {
        self.close_sender
            .send(())
            .await
            .map_err(|e| Error::Io(e.into()))??;

        Ok(())
    }

    async fn get_conn(self: Arc<Self>, ufrag: &str) -> Result<Arc<dyn Conn + Send + Sync>, Error> {
        let conn = self
            .get_conn_sender
            .send(ufrag.to_owned())
            .await
            .map_err(|e| Error::Io(e.into()))??;

        Ok(conn)
    }

    async fn remove_conn_by_ufrag(&self, ufrag: &str) {
        if let Err(e) = self.remove_sender.send(ufrag.to_owned()).await {
            tracing::debug!("Failed to send message through channel: {:?}", e);
        }
    }
}

/// Handle which utilizes [`req_res_chan`] to transmit commands from [`UDPMuxConn`] connections to
/// [`UDPMuxNewAddr::poll`].
pub(crate) struct UdpMuxWriterHandle {
    registration_channel: req_res_chan::Sender<(UDPMuxConn, SocketAddr), ()>,
    send_channel: req_res_chan::Sender<(Vec<u8>, SocketAddr), Result<usize, Error>>,
}

impl UdpMuxWriterHandle {
    /// Returns a new `UdpMuxWriterHandle` and `registration`, `send` receivers.
    fn new() -> (
        Self,
        req_res_chan::Receiver<(UDPMuxConn, SocketAddr), ()>,
        req_res_chan::Receiver<(Vec<u8>, SocketAddr), Result<usize, Error>>,
    ) {
        let (sender1, receiver1) = req_res_chan::new(1);
        let (sender2, receiver2) = req_res_chan::new(1);

        let this = Self {
            registration_channel: sender1,
            send_channel: sender2,
        };

        (this, receiver1, receiver2)
    }
}

#[async_trait]
impl UDPMuxWriter for UdpMuxWriterHandle {
    async fn register_conn_for_address(&self, conn: &UDPMuxConn, addr: SocketAddr) {
        match self
            .registration_channel
            .send((conn.to_owned(), addr))
            .await
        {
            Ok(()) => {}
            Err(e) => {
                tracing::debug!("Failed to send message through channel: {:?}", e);
                return;
            }
        }

        tracing::debug!(address=%addr, connection=%conn.key(), "Registered address for connection");
    }

    async fn send_to(&self, buf: &[u8], target: &SocketAddr) -> Result<usize, Error> {
        let bytes_written = self
            .send_channel
            .send((buf.to_owned(), target.to_owned()))
            .await
            .map_err(|e| Error::Io(e.into()))??;

        Ok(bytes_written)
    }
}

/// Gets the ufrag from the given STUN message or returns an error, if failed to decode or the
/// username attribute is not present.
fn ufrag_from_stun_message(buffer: &[u8], local_ufrag: bool) -> Result<String, Error> {
    let (result, message) = {
        let mut m = STUNMessage::new();

        (m.unmarshal_binary(buffer), m)
    };

    if let Err(err) = result {
        Err(Error::Other(format!("failed to handle decode ICE: {err}")))
    } else {
        let (attr, found) = message.attributes.get(ATTR_USERNAME);
        if !found {
            return Err(Error::Other("no username attribute in STUN message".into()));
        }

        match String::from_utf8(attr.value) {
            // Per the RFC this shouldn't happen
            // https://datatracker.ietf.org/doc/html/rfc5389#section-15.3
            Err(err) => Err(Error::Other(format!(
                "failed to decode USERNAME from STUN message as UTF-8: {err}"
            ))),
            Ok(s) => {
                // s is a combination of the local_ufrag and the remote ufrag separated by `:`.
                let res = if local_ufrag {
                    s.split(':').next()
                } else {
                    s.split(':').next_back()
                };
                match res {
                    Some(s) => Ok(s.to_owned()),
                    None => Err(Error::Other("can't get ufrag from username".into())),
                }
            }
        }
    }
}

#[cfg(test)]
mod pending_new_addr_tests {
    use super::*;

    #[test]
    fn pending_new_addresses_are_bounded() {
        let mut pending = PendingNewAddrs::default();
        for port in 1..=(MAX_PENDING_NEW_ADDRS + 512) {
            let addr = SocketAddr::from(([127, 0, 0, 1], (port % u16::MAX as usize) as u16));
            pending.remember(addr);
        }
        assert!(pending.len() <= MAX_PENDING_NEW_ADDRS);
        assert!(pending.queued_len() <= MAX_PENDING_NEW_ADDRS);
    }

    #[test]
    fn removing_pending_address_allows_it_to_be_reported_again() {
        let mut pending = PendingNewAddrs::default();
        let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
        for _ in 0..10_000 {
            assert!(pending.remember(addr));
            assert!(!pending.remember(addr));
            pending.remove(&addr);
        }
        assert_eq!(pending.len(), 0);
        assert_eq!(pending.queued_len(), 0);
        assert!(pending.remember(addr));
    }
}

#[derive(Error, Debug)]
enum ConnQueryError {
    #[error("ufrag is already taken (associated_addrs={associated_addrs:?})")]
    UfragAlreadyTaken { associated_addrs: Vec<SocketAddr> },
}
