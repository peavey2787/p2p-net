use std::sync::Arc;
use std::time::Duration;

use libp2p::{noise, websocket_websys, webtransport_websys, yamux, SwarmBuilder, Transport};
use libp2p_webrtc_websys as webrtc_websys;
use libp2p_webrtc_websys::browser as webrtc_browser;

use super::{swarm_idle_connection_timeout, transport_plan, TransportPlan};
use crate::common::error::NetError;
use crate::stack::behaviour::{build_behaviour, BehaviourBuildContext, MeshBehaviour};
use crate::{NodeConfig, ResolvedNodeConfig};

pub(super) async fn build_swarm(
    local_key: libp2p::identity::Keypair,
    cfg: &NodeConfig,
    resolved_cfg: &ResolvedNodeConfig,
) -> Result<(libp2p::Swarm<MeshBehaviour>, TransportPlan), NetError> {
    let local_peer = libp2p::PeerId::from(local_key.public());
    let relay_cfg = cfg.relay.clone();
    let (direct_upgrade_transport, webrtc_signaling) = webrtc_browser::Transport::new(
        webrtc_browser::Config {
            keypair: local_key.clone(),
        },
        signaling_config(cfg),
        Arc::default(),
    );
    let (relay_transport, relay_behaviour) = libp2p_relay::client::new(local_peer);

    let builder = SwarmBuilder::with_existing_identity(local_key)
        .with_wasm_bindgen()
        .with_other_transport(move |key| {
            // Circuit Relay v2 client from p2p-net-relay (metered relay);
            // same Noise + Yamux upgrade the libp2p relay builder applies.
            // It must precede the WebRTC transports: their address parser
            // accepts `.../webrtc-direct/.../p2p/<relay>/p2p-circuit/...` and
            // would dial the relay itself instead of through it.
            let noise = noise::Config::new(key).map_err(
                |err| -> Box<dyn std::error::Error + Send + Sync + 'static> { Box::new(err) },
            )?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync + 'static>>(
                relay_transport
                    .upgrade(libp2p::core::upgrade::Version::V1Lazy)
                    .authenticate(noise)
                    .multiplex(yamux::Config::default()),
            )
        })
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_other_transport(|key| webrtc_websys::Transport::new(webrtc_websys::Config::new(key)))
        .map_err(|e| NetError::Build(e.to_string()))?
        // Browser-to-browser `/webrtc`: connections negotiated over a relayed
        // connection by `webrtc_signaling` surface here.
        .with_other_transport(move |_| direct_upgrade_transport.boxed())
        .map_err(|e| NetError::Build(e.to_string()))?
        // Browser WebTransport dialing (servers advertising
        // `/quic-v1/webtransport/certhash/...`). rust-libp2p has no native
        // WebTransport listener, so this reaches non-Rust libp2p servers only.
        .with_other_transport(|key| {
            webtransport_websys::Transport::new(webtransport_websys::Config::new(key)).boxed()
        })
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_other_transport(|key| {
            let noise = noise::Config::new(key).map_err(
                |err| -> Box<dyn std::error::Error + Send + Sync + 'static> { Box::new(err) },
            )?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync + 'static>>(
                websocket_websys::Transport::default()
                    .upgrade(libp2p::core::upgrade::Version::V1Lazy)
                    .authenticate(noise)
                    .multiplex(yamux::Config::default()),
            )
        })
        .map_err(|e| NetError::Build(e.to_string()))?;

    let mut swarm = builder
        .with_behaviour(move |key| {
            build_behaviour(BehaviourBuildContext {
                local_key: key,
                local_peer,
                relay_behaviour,
                network_id: cfg.network_id,
                gossipsub_heartbeat_interval_secs: cfg.gossipsub_heartbeat_interval_secs,
                ping_interval_secs: cfg.ping_interval_secs,
                relay_cfg: &relay_cfg,
                connection_limits_cfg: &cfg.connection_limits,
                discovery_cfg: &cfg.discovery,
                resolved_cfg,
                webrtc_signaling: Some(webrtc_signaling),
            })
        })
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_swarm_config(|c| {
            c.with_idle_connection_timeout(swarm_idle_connection_timeout(cfg.ping_interval_secs))
        })
        .build();

    // Browsers cannot accept raw TCP/QUIC listeners. WebRTC/WSS/WebTransport are
    // dial transports here; inbound browser connectivity is provided through a
    // Circuit Relay reservation and remains invisible to applications. The
    // `/webrtc` listener only surfaces direct connections that the signaling
    // behaviour negotiated over an existing relayed connection.
    swarm
        .listen_on(
            WEBRTC_DIRECT_UPGRADE_LISTEN
                .parse()
                .expect("static multiaddr"),
        )
        .map_err(|e| NetError::Listen {
            addr: WEBRTC_DIRECT_UPGRADE_LISTEN.to_string(),
            reason: e.to_string(),
        })?;
    Ok((swarm, transport_plan(cfg, resolved_cfg)))
}

const WEBRTC_DIRECT_UPGRADE_LISTEN: &str = "/webrtc";

/// Relay-assisted direct upgrade timing: start signaling shortly after the
/// relayed connection is up, allow up to 15 s of ICE checks (60 x 250 ms),
/// retry twice, then stay on the relay.
fn signaling_config(cfg: &NodeConfig) -> webrtc_browser::SignalingConfig {
    let webrtc = &cfg.browser_webrtc;
    let ice_servers = (!webrtc.ice_servers.is_empty()).then(|| webrtc.ice_servers.clone());
    webrtc_browser::SignalingConfig::new(
        2,
        Duration::from_millis(300),
        Duration::from_millis(250),
        60,
        ice_servers,
    )
    .with_relay_only_ice(matches!(
        webrtc.ice_transport_policy,
        crate::IceTransportPolicy::Relay
    ))
}
