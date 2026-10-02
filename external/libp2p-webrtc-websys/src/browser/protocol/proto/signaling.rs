//! `/webrtc-signaling/0.0.1` message, per the libp2p WebRTC spec:
//!
//! ```protobuf
//! message Message {
//!   enum Type { SDP_OFFER = 0; SDP_ANSWER = 1; ICE_CANDIDATE = 2; }
//!   optional Type type = 1;
//!   optional string data = 2;
//! }
//! ```
//!
//! p2p-net encodes it with prost (already in the dependency graph); the wire
//! bytes are identical to any other protobuf implementation of the spec.

use std::io;

use prost::Message as _;

/// One signaling message.
#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct SignalingMessage {
    pub type_pb: Option<mod_SignalingMessage::Type>,
    pub data: Option<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct Wire {
    #[prost(enumeration = "WireType", optional, tag = "1")]
    r#type: Option<i32>,
    #[prost(string, optional, tag = "2")]
    data: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
enum WireType {
    SdpOffer = 0,
    SdpAnswer = 1,
    IceCandidate = 2,
}

impl SignalingMessage {
    pub(crate) fn encode(&self) -> Vec<u8> {
        Wire {
            r#type: self.type_pb.map(|kind| kind as i32),
            data: self.data.clone(),
        }
        .encode_to_vec()
    }

    pub(crate) fn decode(bytes: &[u8]) -> io::Result<Self> {
        let wire = Wire::decode(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let type_pb = match wire.r#type {
            None => None,
            Some(value) => Some(mod_SignalingMessage::Type::try_from(value).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unknown signaling message type {value}"),
                )
            })?),
        };
        Ok(Self {
            type_pb,
            data: wire.data,
        })
    }
}

#[allow(non_snake_case)]
pub(crate) mod mod_SignalingMessage {
    #[allow(non_camel_case_types)]
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum Type {
        SDP_OFFER = 0,
        SDP_ANSWER = 1,
        ICE_CANDIDATE = 2,
    }

    impl TryFrom<i32> for Type {
        type Error = ();

        fn try_from(value: i32) -> Result<Self, ()> {
            match value {
                0 => Ok(Type::SDP_OFFER),
                1 => Ok(Type::SDP_ANSWER),
                2 => Ok(Type::ICE_CANDIDATE),
                _ => Err(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{mod_SignalingMessage::Type, *};

    #[test]
    fn round_trips_and_matches_spec_wire_bytes() {
        let message = SignalingMessage {
            type_pb: Some(Type::SDP_ANSWER),
            data: Some("v=0".to_owned()),
        };
        let bytes = message.encode();
        // tag 1 varint 1, tag 2 len 3 "v=0"
        assert_eq!(bytes, [0x08, 0x01, 0x12, 0x03, b'v', b'=', b'0']);
        assert_eq!(SignalingMessage::decode(&bytes).unwrap(), message);
    }

    #[test]
    fn rejects_unknown_types() {
        assert!(SignalingMessage::decode(&[0x08, 0x07]).is_err());
    }
}
