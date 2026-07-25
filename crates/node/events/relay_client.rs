use std::sync::Arc;

use libp2p::relay;
use libp2p::{Multiaddr, Swarm};
use tokio::sync::Mutex;

use super::super::snapshot::NodeSnapshot;
use crate::connectivity::relay::RelayState;
use crate::stack::{add_external_address_candidate, MeshBehaviour};

use super::super::push_pulse;

pub(crate) async fn handle_event(
    ev: relay::client::Event,
    swarm: &mut Swarm<MeshBehaviour>,
    snapshot: &Arc<Mutex<NodeSnapshot>>,
    relay_state: &mut RelayState,
) {
    let line = match ev {
        relay::client::Event::ReservationReqAccepted {
            relay_peer_id,
            renewal,
            limit: _,
        } => {
            relay_state.reservation_attempted = true;
            relay_state.relay_client_reservations.insert(relay_peer_id);
            if let Some(addresses) = relay_state
                .pending_relay_listen_addrs
                .remove(&relay_peer_id)
            {
                for address in addresses {
                    if let Ok(addr) = address.parse::<Multiaddr>() {
                        add_external_address_candidate(swarm, addr);
                    }
                    relay_state.relayed_listen_addrs.insert(address);
                }
            }
            format!("relay_client reservation accepted relay={relay_peer_id} renewal={renewal}")
        }
        relay::client::Event::OutboundCircuitEstablished {
            relay_peer_id,
            limit: _,
        } => format!("relay_client outbound relayed circuit established relay={relay_peer_id}"),
        relay::client::Event::InboundCircuitEstablished {
            src_peer_id,
            limit: _,
        } => format!("relay_client inbound relayed circuit established src={src_peer_id}"),
    };

    let mut guard = snapshot.lock().await;
    guard.apply_relay_state(relay_state);
    push_pulse(&mut guard.pulses, line);
}
