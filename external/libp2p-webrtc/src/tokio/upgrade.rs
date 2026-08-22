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
    future::Future,
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use futures::{channel::oneshot, future::Either};
use futures_timer::Delay;
use libp2p_identity as identity;
use libp2p_identity::PeerId;
use libp2p_webrtc_utils::{noise, Fingerprint};
use webrtc::{
    api::{setting_engine::SettingEngine, APIBuilder},
    data::data_channel::DataChannel,
    data_channel::data_channel_init::RTCDataChannelInit,
    dtls::extension::extension_use_srtp::SrtpProtectionProfile,
    dtls_transport::dtls_role::DTLSRole,
    ice::{network_type::NetworkType, udp_mux::UDPMux, udp_network::UDPNetwork},
    peer_connection::{configuration::RTCConfiguration, RTCPeerConnection},
};

use crate::tokio::{error::Error, sdp, sdp::random_ufrag, stream::Stream, Connection};

const CONNECTION_SETUP_TIMEOUT: Duration = Duration::from_secs(20);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

struct MuxConnCleanup {
    udp_mux: Arc<dyn UDPMux + Send + Sync>,
    ufrag: Option<String>,
}

impl MuxConnCleanup {
    fn new(udp_mux: Arc<dyn UDPMux + Send + Sync>, ufrag: String) -> Self {
        Self {
            udp_mux,
            ufrag: Some(ufrag),
        }
    }

    fn disarm(&mut self) {
        self.ufrag = None;
    }
}

impl Drop for MuxConnCleanup {
    fn drop(&mut self) {
        let Some(ufrag) = self.ufrag.take() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let udp_mux = self.udp_mux.clone();
        drop(handle.spawn(async move {
            let cleanup = udp_mux.remove_conn_by_ufrag(&ufrag);
            if tokio::time::timeout(CLEANUP_TIMEOUT, cleanup).await.is_err() {
                tracing::trace!("WebRTC mux cleanup timed out for ufrag={ufrag}");
            }
        }));
    }
}

struct PendingPeerConnection(Option<RTCPeerConnection>);

impl PendingPeerConnection {
    fn new(connection: RTCPeerConnection) -> Self {
        Self(Some(connection))
    }

    fn get(&self) -> &RTCPeerConnection {
        self.0.as_ref().expect("pending peer connection")
    }

    fn take(mut self) -> RTCPeerConnection {
        self.0.take().expect("pending peer connection")
    }
}

impl Drop for PendingPeerConnection {
    fn drop(&mut self) {
        let Some(connection) = self.0.take() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        drop(handle.spawn(async move {
            let close = connection.close();
            match tokio::time::timeout(CLEANUP_TIMEOUT, close).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    tracing::trace!("WebRTC setup close-on-drop finished with error: {err}");
                }
                Err(_) => tracing::trace!("WebRTC setup close-on-drop timed out"),
            }
        }));
    }
}

async fn setup_with_timeout<T>(future: impl Future<Output = Result<T, Error>>) -> Result<T, Error> {
    match futures::future::select(Box::pin(future), Delay::new(CONNECTION_SETUP_TIMEOUT)).await {
        Either::Left((result, _)) => result,
        Either::Right(((), _)) => Err(Error::Internal(format!(
            "WebRTC connection setup took longer than {} seconds",
            CONNECTION_SETUP_TIMEOUT.as_secs()
        ))),
    }
}

/// Creates a new outbound WebRTC connection.
pub(crate) async fn outbound(
    addr: SocketAddr,
    config: RTCConfiguration,
    udp_mux: Arc<dyn UDPMux + Send + Sync>,
    client_fingerprint: Fingerprint,
    server_fingerprint: Fingerprint,
    id_keys: identity::Keypair,
) -> Result<(PeerId, Connection), Error> {
    tracing::debug!(address=%addr, "new outbound connection to address");

    let ufrag = random_ufrag();
    let mut cleanup = MuxConnCleanup::new(udp_mux.clone(), ufrag.clone());
    let result = setup_with_timeout(outbound_inner(
        addr,
        config,
        udp_mux,
        client_fingerprint,
        server_fingerprint,
        id_keys,
        &ufrag,
    ))
    .await;
    if result.is_ok() {
        cleanup.disarm();
    }
    result
}

async fn outbound_inner(
    addr: SocketAddr,
    config: RTCConfiguration,
    udp_mux: Arc<dyn UDPMux + Send + Sync>,
    client_fingerprint: Fingerprint,
    server_fingerprint: Fingerprint,
    id_keys: identity::Keypair,
    ufrag: &str,
) -> Result<(PeerId, Connection), Error> {
    let peer_connection = PendingPeerConnection::new(
        new_outbound_connection(addr, config, udp_mux.clone(), ufrag).await?,
    );

    let offer = peer_connection.get().create_offer(None).await?;
    tracing::debug!(offer=%offer.sdp, "created SDP offer for outbound connection");
    peer_connection.get().set_local_description(offer).await?;

    let answer = sdp::answer(addr, server_fingerprint, ufrag);
    tracing::debug!(?answer, "calculated SDP answer for outbound connection");
    peer_connection.get().set_remote_description(answer).await?; // This will start the gathering of ICE candidates.

    let data_channel = create_substream_for_noise_handshake(peer_connection.get()).await?;
    let peer_id = noise::outbound(
        id_keys,
        data_channel,
        server_fingerprint,
        client_fingerprint,
    )
    .await?;

    Ok((
        peer_id,
        Connection::new(peer_connection.take(), udp_mux, ufrag.to_owned()).await,
    ))
}

/// Creates a new inbound WebRTC connection.
pub(crate) async fn inbound(
    addr: SocketAddr,
    config: RTCConfiguration,
    udp_mux: Arc<dyn UDPMux + Send + Sync>,
    server_fingerprint: Fingerprint,
    remote_ufrag: String,
    id_keys: identity::Keypair,
) -> Result<(PeerId, Connection), Error> {
    tracing::debug!(address=%addr, ufrag=%remote_ufrag, "new inbound connection from address");

    let mut cleanup = MuxConnCleanup::new(udp_mux.clone(), remote_ufrag.clone());
    let result = setup_with_timeout(inbound_inner(
        addr,
        config,
        udp_mux,
        server_fingerprint,
        &remote_ufrag,
        id_keys,
    ))
    .await;
    if result.is_ok() {
        cleanup.disarm();
    }
    result
}

async fn inbound_inner(
    addr: SocketAddr,
    config: RTCConfiguration,
    udp_mux: Arc<dyn UDPMux + Send + Sync>,
    server_fingerprint: Fingerprint,
    remote_ufrag: &str,
    id_keys: identity::Keypair,
) -> Result<(PeerId, Connection), Error> {
    let peer_connection = PendingPeerConnection::new(
        new_inbound_connection(addr, config, udp_mux.clone(), remote_ufrag).await?,
    );

    let offer = sdp::offer(addr, remote_ufrag);
    tracing::debug!(?offer, "calculated SDP offer for inbound connection");
    peer_connection.get().set_remote_description(offer).await?;

    let answer = peer_connection.get().create_answer(None).await?;
    tracing::debug!(?answer, "created SDP answer for inbound connection");
    peer_connection.get().set_local_description(answer).await?; // This will start the gathering of ICE candidates.

    let data_channel = create_substream_for_noise_handshake(peer_connection.get()).await?;
    let client_fingerprint = get_remote_fingerprint(peer_connection.get()).await;
    let peer_id = noise::inbound(
        id_keys,
        data_channel,
        client_fingerprint,
        server_fingerprint,
    )
    .await?;

    Ok((
        peer_id,
        Connection::new(
            peer_connection.take(),
            udp_mux,
            remote_ufrag.to_owned(),
        )
        .await,
    ))
}

async fn new_outbound_connection(
    addr: SocketAddr,
    config: RTCConfiguration,
    udp_mux: Arc<dyn UDPMux + Send + Sync>,
    ufrag: &str,
) -> Result<RTCPeerConnection, Error> {
    let se = setting_engine(udp_mux, ufrag, addr);

    let connection = APIBuilder::new()
        .with_setting_engine(se)
        .build()
        .new_peer_connection(config)
        .await?;

    Ok(connection)
}

async fn new_inbound_connection(
    addr: SocketAddr,
    config: RTCConfiguration,
    udp_mux: Arc<dyn UDPMux + Send + Sync>,
    ufrag: &str,
) -> Result<RTCPeerConnection, Error> {
    let mut se = setting_engine(udp_mux, ufrag, addr);
    {
        se.set_lite(true);
        se.disable_certificate_fingerprint_verification(true);
        // Act as a DTLS server (one which waits for a connection).
        //
        // NOTE: removing this seems to break DTLS setup (both sides send `ClientHello` messages,
        // but none end up responding).
        se.set_answering_dtls_role(DTLSRole::Server)?;
    }

    let connection = APIBuilder::new()
        .with_setting_engine(se)
        .build()
        .new_peer_connection(config)
        .await?;

    Ok(connection)
}

fn setting_engine(
    udp_mux: Arc<dyn UDPMux + Send + Sync>,
    ufrag: &str,
    addr: SocketAddr,
) -> SettingEngine {
    let mut se = SettingEngine::default();

    // Set both ICE user and password to our fingerprint because that's what the client is
    // expecting..
    se.set_ice_credentials(ufrag.to_owned(), ufrag.to_owned());

    se.set_udp_network(UDPNetwork::Muxed(udp_mux.clone()));

    // Allow detaching data channels.
    se.detach_data_channels();

    // webrtc-rs 0.12 advertises AES-256-GCM by default, but the SRTP cipher
    // path in that dependency version constructs an AES-128-GCM cipher from
    // the negotiated key bytes. If a remote peer picks AES-256, worker threads
    // panic inside generic-array with `left: 32 right: 16`. Keep WebRTC-direct
    // browser-compatible while constraining SRTP to the AES-128 profiles that
    // this stack handles correctly.
    se.set_srtp_protection_profiles(vec![
        SrtpProtectionProfile::Srtp_Aead_Aes_128_Gcm,
        SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80,
        SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_32,
    ]);

    // Set the desired network type.
    //
    // NOTE: if not set, a [`webrtc_ice::agent::Agent`] might pick a wrong local candidate
    // (e.g. IPv6 `[::1]` while dialing an IPv4 `10.11.12.13`).
    let network_type = match addr {
        SocketAddr::V4(_) => NetworkType::Udp4,
        SocketAddr::V6(_) => NetworkType::Udp6,
    };
    se.set_network_types(vec![network_type]);

    // Select only the first address of the local candidates.
    // See https://github.com/libp2p/rust-libp2p/pull/5448#discussion_r2017418520.
    // TODO: remove when https://github.com/webrtc-rs/webrtc/issues/662 get's addressed.
    se.set_ip_filter(Box::new({
        let once = AtomicBool::new(true);
        move |_ip| {
            if once.load(Ordering::Relaxed) {
                once.store(false, Ordering::Relaxed);
                return true;
            }
            false
        }
    }));

    se
}

/// Returns the SHA-256 fingerprint of the remote.
async fn get_remote_fingerprint(conn: &RTCPeerConnection) -> Fingerprint {
    let cert_bytes = conn.sctp().transport().get_remote_certificate().await;

    Fingerprint::from_certificate(&cert_bytes)
}

async fn create_substream_for_noise_handshake(conn: &RTCPeerConnection) -> Result<Stream, Error> {
    // NOTE: the data channel w/ `negotiated` flag set to `true` MUST be created on both ends.
    let data_channel = conn
        .create_data_channel(
            "",
            Some(RTCDataChannelInit {
                negotiated: Some(0), // 0 is reserved for the Noise substream
                ..RTCDataChannelInit::default()
            }),
        )
        .await?;

    let (tx, rx) = oneshot::channel::<Arc<DataChannel>>();

    // Wait until the data channel is opened and detach it.
    crate::tokio::connection::register_data_channel_open_handler(data_channel, tx).await;

    let channel = match futures::future::select(rx, Delay::new(Duration::from_secs(10))).await {
        Either::Left((Ok(channel), _)) => channel,
        Either::Left((Err(_), _)) => {
            return Err(Error::Internal("failed to open data channel".to_owned()))
        }
        Either::Right(((), _)) => {
            return Err(Error::Internal(
                "data channel opening took longer than 10 seconds (see logs)".into(),
            ))
        }
    };

    let (substream, drop_listener) = Stream::new(channel);
    drop(drop_listener); // Don't care about cancelled substreams during initial handshake.

    Ok(substream)
}
