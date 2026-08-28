//! NAT / relay bookkeeping and on-disk peer address cache.

pub(crate) mod addr;
pub mod connection_strategy;
pub mod dcutr;
pub mod dht;
pub mod discovery;
pub mod dns;
pub mod identity;
pub mod lan;
pub mod limits;
pub mod mediator;
pub mod namespace;
pub mod peer_book;
pub mod peer_cache;
pub mod public_fallback;
pub mod relay;
pub mod relay_discovery;
pub mod rendezvous;
pub mod webrtc;
