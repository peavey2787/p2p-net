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

/// Track relayed connections and direct browser WebRTC connections. A direct
/// connection counts as a successful upgrade as soon as it is up locally; the
/// relayed path is retired only once the remote confirms it (see
/// [`on_identify_received`]).
pub(super) async fn on_connection_established(
    peer: PeerId,
    connection: ConnectionId,
    remote_addr: &Multiaddr,
    relayed: bool,
    outgoing: bool,
    ctx: &mut SwarmEventContext<'_>,
) {
    let state = &mut ctx.relay_state.direct_upgrade;
    let line = if relayed {
        state.relayed_established(peer, connection);
        format!("RELAY_CONNECT peer={peer}")
    } else if is_direct_browser_webrtc(remote_addr) {
        state.direct_established(peer, connection);
        state.upgrades_succeeded = state.upgrades_succeeded.saturating_add(1);
        format!("DIRECT_UPGRADE_SUCCESS peer={peer}")
    } else if outgoing {
        format!("DIRECT_DIAL peer={peer} addr={remote_addr}")
    } else {
        return;
    };
    record(ctx, [line]).await;
}

/// The remote's Identify arrived over `connection`, so the remote has that
/// connection established. If it is a pending direct connection, move the
/// peer's traffic to it by closing the relayed connections (the relay
/// reservation itself is untouched).
pub(super) async fn on_identify_received(
    peer: PeerId,
    connection: ConnectionId,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) {
    let state = &mut ctx.relay_state.direct_upgrade;
    let Some(replaced) = state.confirm_direct(&peer, connection) else {
        return;
    };
    for relayed in &replaced {
        swarm.close_connection(*relayed);
    }
    if !replaced.is_empty() {
        state.paths_migrated = state.paths_migrated.saturating_add(1);
    }
    let line = format!(
        "PATH_MIGRATED_TO_DIRECT peer={peer} relayed_closed={}",
        replaced.len()
    );
    record(ctx, [line]).await;
}

async fn record(ctx: &mut SwarmEventContext<'_>, lines: impl IntoIterator<Item = String>) {
    let mut guard = ctx.snapshot.lock().await;
    guard.direct_upgrade = ctx.relay_state.direct_upgrade.snapshot();
    for line in lines {
        push_pulse(&mut guard.pulses, line);
    }
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
        // Success is counted when the connection reaches the swarm
        // (`on_connection_established`), which both sides observe.
        SignalingEvent::NewWebRTCConnection { peer_id } => {
            vec![format!("ICE_CHECK peer={peer_id} outcome=connected")]
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
    record(ctx, lines).await;
}
