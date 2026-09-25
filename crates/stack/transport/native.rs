use libp2p::core::upgrade::Version;
use libp2p::{noise, tcp, yamux, SwarmBuilder, Transport};
use libp2p_webrtc::tokio::{Certificate as WebRtcCertificate, Transport as WebRtcTransport};
use libp2p_websocket as websocket;

use super::{swarm_idle_connection_timeout, transport_plan, TransportPlan};
use crate::common::error::NetError;
use crate::platform::NodeStorage;
use crate::stack::behaviour::{build_behaviour, BehaviourBuildContext, MeshBehaviour};
use crate::stack::dns_transport::OsDnsTransport;
use crate::{NodeConfig, ResolvedNodeConfig};

fn load_or_create_webrtc_certificate(
    storage: &dyn NodeStorage,
    path: &str,
) -> Result<WebRtcCertificate, NetError> {
    if let Some(raw) = storage.read_secret(path)? {
        let pem = String::from_utf8(raw).map_err(|err| {
            NetError::Build(format!("invalid WebRTC certificate PEM encoding: {err}"))
        })?;
        return WebRtcCertificate::from_pem(&pem).map_err(|err| {
            NetError::Build(format!("invalid persisted WebRTC certificate: {err}"))
        });
    }

    let certificate = WebRtcCertificate::generate(&mut rand::thread_rng())
        .map_err(|err| NetError::Build(format!("failed to generate WebRTC certificate: {err}")))?;
    let pem = certificate.serialize_pem();
    if storage.write_secret_if_absent(path, pem.as_bytes())? {
        return Ok(certificate);
    }
    let raw = storage.read_secret(path)?.ok_or_else(|| {
        NetError::Build("WebRTC certificate create race produced no durable value".to_string())
    })?;
    let pem = String::from_utf8(raw).map_err(|err| {
        NetError::Build(format!(
            "invalid raced WebRTC certificate PEM encoding: {err}"
        ))
    })?;
    WebRtcCertificate::from_pem(&pem)
        .map_err(|err| NetError::Build(format!("invalid raced WebRTC certificate: {err}")))
}

/// dtls resolves rustls' process-level CryptoProvider, which rustls can only infer
/// when exactly one backend is compiled in. libp2p-quic enables aws-lc-rs and dtls
/// enables ring, so pick ring (dtls' own backend) explicitly. An `Err` means the
/// embedding application already installed a provider, which we respect.
fn ensure_rustls_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

pub(super) async fn build_swarm(
    local_key: libp2p::identity::Keypair,
    cfg: &NodeConfig,
    resolved_cfg: &ResolvedNodeConfig,
    storage: &dyn NodeStorage,
) -> Result<(libp2p::Swarm<MeshBehaviour>, TransportPlan), NetError> {
    ensure_rustls_crypto_provider();
    let local_peer = libp2p::PeerId::from(local_key.public());
    let relay_cfg = cfg.relay.clone();
    let certificate = load_or_create_webrtc_certificate(storage, &cfg.webrtc_certificate_path)?;

    let builder = SwarmBuilder::with_existing_identity(local_key)
        .with_tokio()
        .with_tcp(
            tcp::Config::default().nodelay(true),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_quic()
        .with_other_transport(move |key| {
            Ok::<_, Box<dyn std::error::Error + Send + Sync + 'static>>(WebRtcTransport::new(
                key.clone(),
                certificate.clone(),
            ))
        })
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_other_transport(|key| {
            let noise = noise::Config::new(key).map_err(
                |err| -> Box<dyn std::error::Error + Send + Sync + 'static> { Box::new(err) },
            )?;
            let tcp = tcp::tokio::Transport::new(tcp::Config::default().nodelay(true));
            let websocket = websocket::Config::new(OsDnsTransport::new(tcp))
                .upgrade(Version::V1Lazy)
                .authenticate(noise)
                .multiplex(yamux::Config::default());
            Ok::<_, Box<dyn std::error::Error + Send + Sync + 'static>>(websocket)
        })
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .map_err(|e| NetError::Build(e.to_string()))?;

    let mut swarm = builder
        .with_behaviour(|key, relay_behaviour| {
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
            })
        })
        .map_err(|e| NetError::Build(e.to_string()))?
        .with_swarm_config(|c| {
            c.with_idle_connection_timeout(swarm_idle_connection_timeout(cfg.ping_interval_secs))
        })
        .build();

    for addr in cfg.enabled_listen_addresses()? {
        swarm
            .listen_on(addr.clone())
            .map_err(|e| NetError::Listen {
                addr: addr.to_string(),
                reason: e.to_string(),
            })?;
    }
    Ok((swarm, transport_plan(cfg, resolved_cfg)))
}

#[cfg(test)]
mod tests {
    use super::load_or_create_webrtc_certificate;
    use crate::platform::MemoryNodeStorage;

    #[test]
    fn persisted_webrtc_certificate_keeps_stable_certhash() {
        let storage = MemoryNodeStorage::new();
        let first = load_or_create_webrtc_certificate(&storage, "webrtc-cert.pem")
            .expect("first certificate");
        let second = load_or_create_webrtc_certificate(&storage, "webrtc-cert.pem")
            .expect("reloaded certificate");

        assert_eq!(first.fingerprint(), second.fingerprint());
    }
}
