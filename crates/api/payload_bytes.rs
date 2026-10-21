//! Wire form of envelope payloads. Bytes are written as one standard base64
//! string (4/3 of the payload) instead of serde_json's default array of decimal
//! numbers (up to 4x, e.g. `[255,0,...]`). Decoding also accepts the number-array
//! form, so envelopes from peers running an earlier build are still read.
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserializer, Serializer};
use std::fmt;

pub(crate) fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&STANDARD.encode(bytes))
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    struct PayloadVisitor;
    impl<'de> Visitor<'de> for PayloadVisitor {
        type Value = Vec<u8>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a base64 string or an array of bytes")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Vec<u8>, E> {
            STANDARD.decode(v).map_err(E::custom)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<u8>, A::Error> {
            let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(64 * 1024));
            while let Some(b) = seq.next_element::<u8>()? {
                out.push(b);
            }
            Ok(out)
        }
    }
    deserializer.deserialize_any(PayloadVisitor)
}

#[cfg(test)]
mod tests {
    use super::super::*;

    fn sample(payload: Vec<u8>) -> AppMessage {
        AppMessage::broadcast(7, "wire", PeerId::random(), payload).expect("message")
    }

    #[test]
    fn payload_is_a_base64_string_on_the_wire() {
        let message = sample((0..=255u8).cycle().take(4096).collect());
        let wire = encode_app_message(&message).expect("encode");
        let json: serde_json::Value = serde_json::from_slice(&wire).expect("json");
        assert!(json["payload"].is_string());
        // 4096 bytes -> 5464 base64 chars plus a few hundred bytes of envelope.
        assert!(
            wire.len() < 4096 * 3 / 2,
            "envelope is {} bytes",
            wire.len()
        );
        assert_eq!(decode_app_message(&wire).expect("decode"), message);
    }

    #[test]
    fn number_array_payload_from_earlier_builds_still_decodes() {
        let message = sample(vec![0, 1, 2, 254, 255]);
        let mut json = serde_json::to_value(&message).expect("json");
        json["payload"] = serde_json::json!([0, 1, 2, 254, 255]);
        let decoded = decode_app_message(&serde_json::to_vec(&json).unwrap()).expect("decode");
        assert_eq!(decoded, message);
    }

    #[test]
    fn malformed_payloads_are_rejected() {
        let message = sample(vec![1, 2, 3]);
        for bad in [
            serde_json::json!("not base64!"),
            serde_json::json!([256]),
            serde_json::json!([-1]),
            serde_json::json!(12),
        ] {
            let mut json = serde_json::to_value(&message).expect("json");
            json["payload"] = bad;
            assert!(decode_app_message(&serde_json::to_vec(&json).unwrap()).is_err());
        }
    }

    #[test]
    fn empty_payload_round_trips() {
        let message = sample(Vec::new());
        let wire = encode_app_message(&message);
        if let Ok(wire) = wire {
            assert_eq!(decode_app_message(&wire).expect("decode"), message);
        }
    }
}
