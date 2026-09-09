use libp2p::swarm::ConnectionId;
use libp2p::{PeerId, Swarm};

use crate::stack::release_application_keep_alive;
use crate::stack::MeshBehaviour;

use super::super::super::push_pulse;
use super::super::{sync_swarm_connection_snapshot, SwarmEventContext};

/// Connection-close details retained from the swarm event for application-peer
/// diagnostics. Keeping the endpoint and close cause makes relay churn visible
/// instead of collapsing every disconnect into an anonymous reconnect.
pub(crate) struct ClosedConnection {
    pub(crate) peer_id: PeerId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) relayed_endpoint: bool,
    pub(crate) remaining_established: u32,
    pub(crate) endpoint_debug: String,
    pub(crate) cause_debug: String,
}

pub(crate) async fn handle_connection_closed(
    connection: ClosedConnection,
    swarm: &mut Swarm<MeshBehaviour>,
    ctx: &mut SwarmEventContext<'_>,
) {
    let ClosedConnection {
        peer_id,
        connection_id,
        relayed_endpoint,
        remaining_established,
        endpoint_debug,
        cause_debug,
    } = connection;
    let application_peer = ctx
        .peer_book
        .has_application_namespace(&peer_id, ctx.application_namespaces);

    ctx.connection_caps.record_closed(connection_id);
    if remaining_established == 0 {
        // Explicit Gossipsub peers are used only while a verified application
        // connection is live. Auto-connect remains the sole reconnect policy.
        swarm
            .behaviour_mut()
            .gossipsub
            .remove_explicit_peer(&peer_id);
        release_application_keep_alive(swarm, &peer_id);
        if application_peer {
            ctx.dht_state.mark_auto_connect_disconnected(&peer_id);
        }
        ctx.peer_book.record_disconnected_if_known(peer_id);
        ctx.relay_state.unverified_relayed_peers.remove(&peer_id);
    }

    let mut guard = ctx.snapshot.lock().await;
    sync_swarm_connection_snapshot(&mut guard, swarm, ctx);
    guard.connection_cap_disconnects = ctx.connection_caps.cap_disconnects;
    if application_peer {
        push_pulse(
            &mut guard.pulses,
            application_connection_closed_pulse(
                peer_id,
                relayed_endpoint,
                remaining_established,
                &cause_debug,
                &endpoint_debug,
            ),
        );
    }
}

fn application_connection_closed_pulse(
    peer_id: PeerId,
    relayed_endpoint: bool,
    remaining_established: u32,
    cause_debug: &str,
    endpoint_debug: &str,
) -> String {
    format!(
        "application connection closed peer={peer_id} relayed={relayed_endpoint} remaining={remaining_established} cause={cause_debug} endpoint={endpoint_debug}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_close_observability_retains_relay_cause_and_remaining_count() {
        let peer = PeerId::random();
        let line = application_connection_closed_pulse(
            peer,
            true,
            0,
            "Some(KeepAliveTimeout)",
            "Dialer { address: /p2p-circuit }",
        );

        assert!(line.contains(&format!("peer={peer}")));
        assert!(line.contains("relayed=true"));
        assert!(line.contains("remaining=0"));
        assert!(line.contains("cause=Some(KeepAliveTimeout)"));
        assert!(line.contains("endpoint=Dialer"));
    }
}
