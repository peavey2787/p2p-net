//! Relay-assisted direct upgrade lifecycle (browser WebRTC private-to-private).
//! See `connectivity::direct_upgrade` for the model and the diagnostic tags.

use libp2p::swarm::ConnectionId;
use libp2p::{Multiaddr, PeerId, Swarm};
use libp2p_webrtc_websys::browser::SignalingEvent;
use libp2p_webrtc_websys::Error as WebRtcError;

use super::super::push_pulse;
use super::SwarmEventContext;
use crate::connectivity::direct_upgrade::is_direct_browser_webrtc;
use crate::stack::MeshBehaviour;

/// Track relayed connections and, when a direct browser WebRTC connection to
/// the same peer comes up, move the peer's traffic to it by closing the
/// relayed connections. The relay reservation itself is untouched.
pub(super) async fn on_connection_established(
    peer: PeerId,
    connection: ConnectionId,
    remote_addr: &Multiaddr,
    relayed: bool,
    outgoing: bool,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) {
    let line = if relayed {
        ctx.relay_state
            .direct_upgrade
            .relayed_established(peer, connection);
        format!("RELAY_CONNECT peer={peer}")
    } else if is_direct_browser_webrtc(remote_addr) {
        let replaced = ctx.relay_state.direct_upgrade.take_relayed(&peer);
        for relayed in &replaced {
            swarm.close_connection(*relayed);
        }
        if !replaced.is_empty() {
            ctx.relay_state.direct_upgrade.paths_migrated = ctx
                .relay_state
                .direct_upgrade
                .paths_migrated
                .saturating_add(1);
        }
        format!(
            "PATH_MIGRATED_TO_DIRECT peer={peer} relayed_closed={}",
            replaced.len()
        )
    } else if outgoing {
        format!("DIRECT_DIAL peer={peer} addr={remote_addr}")
    } else {
        return;
    };
    let mut guard = ctx.snapshot.lock().await;
    guard.direct_upgrade = ctx.relay_state.direct_upgrade.snapshot();
    push_pulse(&mut guard.pulses, line);
}

pub(super) fn on_connection_closed(
    peer: PeerId,
    connection: ConnectionId,
    ctx: &mut SwarmEventContext<'_>,
) {
    ctx.relay_state
        .direct_upgrade
        .connection_closed(peer, connection);
}

pub(super) async fn on_signaling_event(event: SignalingEvent, ctx: &mut SwarmEventContext<'_>) {
    let state = &mut ctx.relay_state.direct_upgrade;
    let lines = match event {
        SignalingEvent::SignalingStarted { peer_id, initiator } => {
            vec![format!(
                "WEBRTC_SIGNALING peer={peer_id} initiator={initiator}"
            )]
        }
        SignalingEvent::NewWebRTCConnection { peer_id } => {
            state.upgrades_succeeded = state.upgrades_succeeded.saturating_add(1);
            vec![
                format!("ICE_CHECK peer={peer_id} outcome=connected"),
                format!("DIRECT_UPGRADE_SUCCESS peer={peer_id}"),
            ]
        }
        SignalingEvent::WebRTCConnectionError { peer_id, error } => {
            state.upgrades_failed = state.upgrades_failed.saturating_add(1);
            let mut lines = Vec::with_capacity(3);
            // Only failures of the connectivity check itself are ICE outcomes;
            // a signaling or handshake error never reached ICE.
            if let WebRtcError::IceCheck { outcome, ice_state } = &error {
                lines.push(format!(
                    "ICE_CHECK peer={peer_id} outcome={outcome} ice_state={ice_state}"
                ));
            }
            lines.push(format!(
                "DIRECT_UPGRADE_FAILED peer={peer_id} error={error}"
            ));
            if state.has_relayed(&peer_id) {
                lines.push(format!("RELAY_FALLBACK peer={peer_id}"));
            }
            lines
        }
    };
    let mut guard = ctx.snapshot.lock().await;
    guard.direct_upgrade = ctx.relay_state.direct_upgrade.snapshot();
    for line in lines {
        push_pulse(&mut guard.pulses, line);
    }
}
