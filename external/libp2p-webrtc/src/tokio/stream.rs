// Copyright 2023 Protocol Labs.
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
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::prelude::*;
use libp2p_webrtc_utils::MAX_MSG_LEN;
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};
use webrtc::data::data_channel::{DataChannel, PollDataChannel};

/// A substream on top of a WebRTC data channel.
///
/// To be a proper libp2p substream, we need to implement [`AsyncRead`] and [`AsyncWrite`] as well
/// as support a half-closed state which we do by framing messages in a protobuf envelope.
pub struct Stream {
    inner: libp2p_webrtc_utils::Stream<FullFrameChannel>,
}

pub(crate) type DropListener = libp2p_webrtc_utils::DropListener<FullFrameChannel>;

/// A `PollDataChannel` that can read a whole libp2p WebRTC frame.
///
/// `libp2p_webrtc_utils::Stream::new` reads through a *clone* of the channel
/// it is given, and `PollDataChannel::clone` resets the read buffer to
/// webrtc-rs's 8 KiB default. A peer (e.g. a browser) that sends frames up to
/// `MAX_MSG_LEN` (16 KiB) then fails every read of a larger frame with
/// `ErrShortBuffer { size: 8192 }`, killing the stream (and, on a relay, the
/// whole circuit). Cloning this wrapper keeps the full-frame read capacity.
pub(crate) struct FullFrameChannel(Compat<PollDataChannel>);

impl FullFrameChannel {
    fn new(data_channel: Arc<DataChannel>) -> Self {
        let mut channel = PollDataChannel::new(data_channel);
        channel.set_read_buf_capacity(MAX_MSG_LEN);
        Self(channel.compat())
    }
}

impl Clone for FullFrameChannel {
    fn clone(&self) -> Self {
        Self::new(self.0.get_ref().clone_inner())
    }
}

impl AsyncRead for FullFrameChannel {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
    }
}

impl AsyncWrite for FullFrameChannel {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        // `PollDataChannel` sends each `poll_write` buffer as one SCTP message,
        // but the framed writer above flushes only once its buffer reaches
        // ~`MAX_DATA_LEN`, so one flush can carry nearly two frames (~32 KiB).
        // webrtc-rs rejects anything over the 16 KiB message size ("outbound
        // packet larger than maximum message size"). The channel is a byte
        // stream to libp2p, so cap each message and let the caller continue.
        let len = buf.len().min(MAX_MSG_LEN);
        Pin::new(&mut self.get_mut().0).poll_write(cx, &buf[..len])
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_close(cx)
    }
}

impl Stream {
    /// Returns a new `Substream` and a listener, which will notify the receiver when/if the
    /// substream is dropped.
    pub(crate) fn new(data_channel: Arc<DataChannel>) -> (Self, DropListener) {
        let (inner, drop_listener) =
            libp2p_webrtc_utils::Stream::new(FullFrameChannel::new(data_channel));

        (Self { inner }, drop_listener)
    }
}
impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_close(cx)
    }
}
