//! TURN credential broker events: browsers ask peers that advertise the
//! broker protocol, brokers answer from their credential cache.

use libp2p::request_response::{Event, Message, OutboundFailure};
use libp2p::{PeerId, StreamProtocol, Swarm};
use web_time::Instant;

use crate::connectivity::turn::TURN_CREDENTIALS_PROTOCOL;
use crate::stack::{MeshBehaviour, TurnCredentialsEvent};

use super::super::turn::TurnRuntime;
use super::SwarmEventContext;

/// Identify told us which protocols `peer` speaks; ask brokers for TURN
/// credentials.
pub(super) fn on_identify(
    peer: PeerId,
    protocols: &[StreamProtocol],
    swarm: &mut Swarm<MeshBehaviour>,
    turn: &mut TurnRuntime,
) {
    if protocols
        .iter()
        .any(|protocol| protocol.as_ref() == TURN_CREDENTIALS_PROTOCOL)
    {
        request_credentials(peer, swarm, turn, Instant::now());
    }
}

/// Send a credential request to `peer` when the client policy allows it.
pub(crate) fn request_credentials(
    peer: PeerId,
    swarm: &mut Swarm<MeshBehaviour>,
    turn: &mut TurnRuntime,
    now: Instant,
) {
    let Some(behaviour) = swarm.behaviour_mut().turn_credentials.as_mut() else {
        return;
    };
    if let Some(request) = turn.request_for(peer, now) {
        behaviour.send_request(&peer, request);
    }
}

pub(super) async fn handle_event(
    event: TurnCredentialsEvent,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) {
    let now = Instant::now();
    let line = match event {
        Event::Message {
            peer,
            message: Message::Request { channel, .. },
            ..
        } => {
            let response = ctx.turn.answer(peer, now);
            let granted = matches!(
                response,
                crate::connectivity::turn::CredentialResponse::Granted(_)
            );
            if let Some(behaviour) = swarm.behaviour_mut().turn_credentials.as_mut() {
                let _ = behaviour.send_response(channel, response);
            }
            granted.then(|| format!("TURN_CREDENTIALS_GRANTED peer={peer}"))
        }
        Event::Message {
            peer,
            message: Message::Response { response, .. },
            ..
        } => ctx.turn.on_response(peer, response, now),
        Event::OutboundFailure {
            peer,
            error: OutboundFailure::UnsupportedProtocols,
            ..
        } => {
            ctx.turn.mark_unsupported(peer);
            None
        }
        Event::OutboundFailure { peer, error, .. } => Some(format!(
            "TURN_CREDENTIALS_REQUEST_FAILED peer={peer} error={error}"
        )),
        Event::InboundFailure { .. } | Event::ResponseSent { .. } => None,
    };
    if let Some(line) = line {
        ctx.observability.pulse(line);
    }
    ctx.snapshot.lock().await.turn = ctx.turn.snapshot().clone();
}
