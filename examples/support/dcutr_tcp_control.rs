//! A 60-second WAN TCP/DCUtR control, not production-node acceptance.
//!
//! Runs stock TCP/Noise/Yamux, relay, Identify and DCUtR only. A fresh shared
//! directory exchanges identities and readiness, never direct dial addresses.
//! Only the relay's observation of the supplied public IPv4 is offered to DCUtR;
//! no guessed ports, LAN candidates, app planner or DHT are involved. Alice dials
//! Bob's public relay circuit after both have reservations and observed addresses.
//! Invoke both roles through `live_dcutr_process_probe --tcp-control`.

use std::{net::Ipv4Addr, path::PathBuf, time::Duration};

use futures::StreamExt;
use libp2p::{
    dcutr, identify,
    multiaddr::Protocol,
    noise, relay,
    swarm::{behaviour::FromSwarm, NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId, SwarmBuilder,
};
use serde::{Deserialize, Serialize};

#[derive(NetworkBehaviour)]
struct Behaviour {
    relay: relay::client::Behaviour,
    identify: identify::Behaviour,
    dcutr: dcutr::Behaviour,
}

#[derive(Serialize, Deserialize)]
struct Status {
    peer_id: String,
    elapsed_seconds: u64,
    ready: bool,
    relay_seen: bool,
    direct_endpoint: Option<String>,
    dcutr_success: bool,
}

pub async fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.len() != 5 || !matches!(args[0].as_str(), "alice" | "bob") {
        return Err("expected ROLE(alice|bob) FRESH_SESSION RELAY LOCAL_PORT PUBLIC_IPV4".into());
    }
    let role = &args[0];
    let session = PathBuf::from(&args[1]);
    let relay_addr: Multiaddr = args[2].parse()?;
    let Some(Protocol::P2p(relay_id)) = relay_addr.iter().last() else {
        return Err("relay address must end in /p2p/RELAY_ID".into());
    };
    let port: u16 = args[3].parse()?;
    let public_ip: Ipv4Addr = args[4].parse()?;
    if port == 0
        || public_ip.is_private()
        || public_ip.is_loopback()
        || public_ip.is_unspecified()
        || public_ip.is_link_local()
        || public_ip.is_multicast()
        || public_ip.is_broadcast()
        || public_ip.is_documentation()
        || public_ip.octets()[0] == 0
        || (public_ip.octets()[0] == 100 && (64..128).contains(&public_ip.octets()[1]))
        || public_ip.octets()[0] >= 240
    {
        return Err("supply a nonzero listener port and the verified WAN IPv4".into());
    }
    std::fs::create_dir_all(&session)?;
    // A persistent marker prevents accidentally reusing stale peer readiness.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(session.join(format!("{role}.started")))?;
    let own_path = session.join(format!("{role}.control.json"));
    let other_path = session.join(if role == "alice" {
        "bob.control.json"
    } else {
        "alice.control.json"
    });
    let mut swarm = SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default().nodelay(true),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_relay_client(noise::Config::new, yamux::Config::default)?
        .with_behaviour(|key, relay| Behaviour {
            relay,
            identify: identify::Behaviour::new(identify::Config::new(
                "/p2p-net/tcp-control/1".into(),
                key.public(),
            )),
            dcutr: dcutr::Behaviour::new(key.public().to_peer_id()),
        })?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(65)))
        .build();
    let mut status = Status {
        peer_id: swarm.local_peer_id().to_string(),
        elapsed_seconds: 0,
        ready: false,
        relay_seen: false,
        direct_endpoint: None,
        dcutr_success: false,
    };
    let started = tokio::time::Instant::now();
    let deadline = started + Duration::from_secs(60);
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let mut observed = false;
    let mut reserved = false;
    let mut dialed = false;
    let mut relay_dialed = false;
    let mut target = None;
    swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{port}").parse()?)?;
    // Reserve only after Identify supplies a real observed port. Otherwise
    // an early relay circuit can capture an empty DCUtR candidate list.
    loop {
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(deadline) => break,
            _ = tick.tick() => {
                status.elapsed_seconds = started.elapsed().as_secs();
                status.ready = observed && reserved;
                std::fs::write(&own_path, serde_json::to_vec(&status)?)?;
                let other = std::fs::read(&other_path).ok()
                    .and_then(|bytes| serde_json::from_slice::<Status>(&bytes).ok());
                if let Some(other) = other {
                    let other_peer: PeerId = other.peer_id.parse()?;
                    target = Some(other_peer);
                    if role == "alice" && status.ready && other.ready && !dialed {
                        let addr = relay_addr.clone().with(Protocol::P2pCircuit).with(Protocol::P2p(other_peer));
                        println!("elapsed_ms={} relay_dial={addr}", started.elapsed().as_millis());
                        swarm.dial(addr)?;
                        dialed = true;
                    }
                    if status.dcutr_success && status.direct_endpoint.is_some() && status.relay_seen
                        && other.dcutr_success && other.direct_endpoint.is_some() && other.relay_seen {
                        println!("TCP_CONTROL_RESULT=success role={role} production_acceptance=false");
                        return Ok(());
                    }
                }
            }
            event = swarm.select_next_some() => {
                println!("elapsed_ms={} event={event:?}", started.elapsed().as_millis());
                match event {
                    SwarmEvent::NewListenAddr { address, .. } if !relay_dialed => {
                        // Wildcard TCP listeners register reusable ports while
                        // polling interface events, not inside listen_on(). An
                        // earlier dial would observe an ephemeral, non-listening
                        // port at the relay and invalidate the control.
                        if address.iter().any(|p| matches!(p, Protocol::Ip4(ip) if !ip.is_loopback())) {
                            swarm.dial(relay_addr.clone())?;
                            relay_dialed = true;
                        }
                    }
                    SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received { peer_id, info, .. })) if peer_id == relay_id => {
                        let mut parts = info.observed_addr.iter();
                        let usable = parts.next() == Some(Protocol::Ip4(public_ip))
                            && matches!(parts.next(), Some(Protocol::Tcp(p)) if p != 0)
                            && parts.next().is_none();
                        if usable && !observed {
                            // Feed a candidate, not a reachability confirmation: an
                            // observed NAT mapping is not proof of unsolicited ingress.
                            swarm.behaviour_mut().dcutr.on_swarm_event(FromSwarm::NewExternalAddrCandidate(
                                libp2p::swarm::behaviour::NewExternalAddrCandidate { addr: &info.observed_addr },
                            ));
                            observed = true;
                            swarm.listen_on(relay_addr.clone().with(Protocol::P2pCircuit))?;
                        }
                    }
                    SwarmEvent::Behaviour(BehaviourEvent::Relay(relay::client::Event::ReservationReqAccepted { .. })) => reserved = true,
                    SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } if Some(peer_id) == target => {
                        if endpoint.is_relayed() {
                            status.relay_seen = true;
                        } else if status.relay_seen {
                            status.direct_endpoint = Some(format!("{endpoint:?}"));
                        }
                    }
                    SwarmEvent::Behaviour(BehaviourEvent::Dcutr(event)) if Some(event.remote_peer_id) == target => {
                        status.dcutr_success |= event.result.is_ok();
                    }
                    _ => {}
                }
            }
        }
    }
    status.elapsed_seconds = 60;
    std::fs::write(own_path, serde_json::to_vec(&status)?)?;
    Err(format!("TCP_CONTROL_RESULT=failed role={role} timeout=60s relay_seen={} direct={:?} dcutr_success={}", status.relay_seen, status.direct_endpoint, status.dcutr_success).into())
}
