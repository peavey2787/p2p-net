//! Bounded internal fragmentation for large application payloads. This is a
//! transport detail: applications still send and receive exactly one AppMessage.

use std::collections::HashMap;
use std::time::Duration;
use web_time::Instant;

use libp2p::PeerId;
use serde::{Deserialize, Serialize};

use super::{
    normalize_app_topic, validate_app_message, AppMessage, APP_MESSAGE_SCHEMA_VERSION,
    MAX_APP_MESSAGE_BYTES,
};
use crate::common::error::NetError;

pub const APP_FRAGMENT_SCHEMA_VERSION: u16 = 1;
pub const APP_FRAGMENT_PAYLOAD_BYTES: usize = 16 * 1024;
pub const MAX_APP_FRAGMENTS: usize = MAX_APP_MESSAGE_BYTES.div_ceil(APP_FRAGMENT_PAYLOAD_BYTES);
const MAX_FRAGMENT_WIRE_BYTES: usize = 128 * 1024;
const MAX_REASSEMBLY_BYTES_PER_PEER: usize = 2 * MAX_APP_MESSAGE_BYTES;
const MAX_REASSEMBLY_BYTES_GLOBAL: usize = 8 * MAX_APP_MESSAGE_BYTES;
const FRAGMENT_EXPIRY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AppFragment {
    pub fragment_schema_version: u16,
    pub message_schema_version: u16,
    pub network_id: u32,
    pub topic: String,
    pub source_peer_id: String,
    pub target_peer_id: Option<String>,
    pub timestamp_ns: u64,
    pub nonce_hex: String,
    pub payload_hash_hex: String,
    pub payload_len: usize,
    pub index: usize,
    pub total: usize,
    pub payload: Vec<u8>,
}

pub(crate) fn fragment_message(message: &AppMessage) -> Result<Vec<Vec<u8>>, NetError> {
    validate_app_message(message)?;
    if message.payload.len() <= APP_FRAGMENT_PAYLOAD_BYTES {
        return Ok(vec![super::encode_app_message(message)?]);
    }
    let total = message.payload.len().div_ceil(APP_FRAGMENT_PAYLOAD_BYTES);
    if total == 0 || total > MAX_APP_FRAGMENTS {
        return Err(app_error(
            &message.topic,
            "fragment count exceeds bounded maximum",
        ));
    }
    let payload_hash_hex = blake3::hash(&message.payload).to_hex().to_string();
    message
        .payload
        .chunks(APP_FRAGMENT_PAYLOAD_BYTES)
        .enumerate()
        .map(|(index, payload)| {
            let fragment = AppFragment {
                fragment_schema_version: APP_FRAGMENT_SCHEMA_VERSION,
                message_schema_version: message.schema_version,
                network_id: message.network_id,
                topic: message.topic.clone(),
                source_peer_id: message.source_peer_id.clone(),
                target_peer_id: message.target_peer_id.clone(),
                timestamp_ns: message.timestamp_ns,
                nonce_hex: message.nonce_hex.clone(),
                payload_hash_hex: payload_hash_hex.clone(),
                payload_len: message.payload.len(),
                index,
                total,
                payload: payload.to_vec(),
            };
            let wire = serde_json::to_vec(&fragment)
                .map_err(|e| app_error(&message.topic, e.to_string()))?;
            if wire.len() > MAX_FRAGMENT_WIRE_BYTES {
                return Err(app_error(
                    &message.topic,
                    "encoded fragment exceeds wire bound",
                ));
            }
            Ok(wire)
        })
        .collect()
}

pub(crate) fn decode_fragment(raw: &[u8]) -> Result<AppFragment, NetError> {
    if raw.len() > MAX_FRAGMENT_WIRE_BYTES {
        return Err(app_error("<unknown>", "fragment wire exceeds bound"));
    }
    let fragment: AppFragment =
        serde_json::from_slice(raw).map_err(|e| app_error("<unknown>", e.to_string()))?;
    validate_fragment(&fragment)?;
    Ok(fragment)
}

fn validate_fragment(fragment: &AppFragment) -> Result<(), NetError> {
    if fragment.fragment_schema_version != APP_FRAGMENT_SCHEMA_VERSION
        || fragment.message_schema_version != APP_MESSAGE_SCHEMA_VERSION
    {
        return Err(app_error(
            &fragment.topic,
            "unsupported fragment/message schema version",
        ));
    }
    normalize_app_topic(&fragment.topic)?;
    fragment.source_peer_id.parse::<PeerId>().map_err(|e| {
        app_error(
            &fragment.topic,
            format!("invalid fragment source peer id: {e}"),
        )
    })?;
    if let Some(target) = &fragment.target_peer_id {
        target.parse::<PeerId>().map_err(|e| {
            app_error(
                &fragment.topic,
                format!("invalid fragment target peer id: {e}"),
            )
        })?;
    }
    if fragment.payload_len > MAX_APP_MESSAGE_BYTES
        || fragment.payload_len <= APP_FRAGMENT_PAYLOAD_BYTES
    {
        return Err(app_error(
            &fragment.topic,
            "invalid fragmented payload length",
        ));
    }
    let expected_total = fragment.payload_len.div_ceil(APP_FRAGMENT_PAYLOAD_BYTES);
    if fragment.total != expected_total
        || fragment.total == 0
        || fragment.total > MAX_APP_FRAGMENTS
        || fragment.index >= fragment.total
    {
        return Err(app_error(&fragment.topic, "invalid fragment index/total"));
    }
    let expected_len = if fragment.index + 1 == fragment.total {
        fragment.payload_len - APP_FRAGMENT_PAYLOAD_BYTES * (fragment.total - 1)
    } else {
        APP_FRAGMENT_PAYLOAD_BYTES
    };
    if fragment.payload.len() != expected_len {
        return Err(app_error(
            &fragment.topic,
            "fragment payload length does not match index/total",
        ));
    }
    if fragment.payload_hash_hex.len() != 64
        || !fragment
            .payload_hash_hex
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err(app_error(&fragment.topic, "invalid fragment payload hash"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ReassemblyKey {
    author: PeerId,
    nonce_hex: String,
    topic: String,
}

struct ReassemblyEntry {
    fragment: AppFragment,
    chunks: Vec<Option<Vec<u8>>>,
    bytes: usize,
    created: Instant,
}

#[derive(Default)]
pub(crate) struct AppFragmentReassembler {
    entries: HashMap<ReassemblyKey, ReassemblyEntry>,
    peer_bytes: HashMap<PeerId, usize>,
    global_bytes: usize,
}

impl AppFragmentReassembler {
    pub(crate) fn push(
        &mut self,
        author: PeerId,
        fragment: AppFragment,
    ) -> Result<Option<AppMessage>, NetError> {
        self.expire();
        if fragment.source_peer_id != author.to_string() {
            return Err(app_error(
                &fragment.topic,
                "fragment source does not match authenticated Gossipsub author",
            ));
        }
        let key = ReassemblyKey {
            author,
            nonce_hex: fragment.nonce_hex.clone(),
            topic: fragment.topic.clone(),
        };
        let current_peer = self.peer_bytes.get(&author).copied().unwrap_or(0);
        if current_peer.saturating_add(fragment.payload.len()) > MAX_REASSEMBLY_BYTES_PER_PEER
            || self.global_bytes.saturating_add(fragment.payload.len())
                > MAX_REASSEMBLY_BYTES_GLOBAL
        {
            return Err(app_error(
                &fragment.topic,
                "fragment reassembly memory ceiling exceeded",
            ));
        }
        let entry = self
            .entries
            .entry(key.clone())
            .or_insert_with(|| ReassemblyEntry {
                chunks: vec![None; fragment.total],
                fragment: fragment.clone(),
                bytes: 0,
                created: Instant::now(),
            });
        if !same_message(&entry.fragment, &fragment) {
            return Err(app_error(&fragment.topic, "fragment metadata mismatch"));
        }
        if entry.chunks[fragment.index].is_some() {
            return Ok(None); // duplicate fragment: bounded, idempotent ignore
        }
        entry.bytes = entry.bytes.saturating_add(fragment.payload.len());
        self.global_bytes = self.global_bytes.saturating_add(fragment.payload.len());
        *self.peer_bytes.entry(author).or_default() =
            current_peer.saturating_add(fragment.payload.len());
        entry.chunks[fragment.index] = Some(fragment.payload);
        if entry.chunks.iter().any(Option::is_none) {
            return Ok(None);
        }

        let entry = self
            .entries
            .remove(&key)
            .expect("completed reassembly exists");
        self.global_bytes = self.global_bytes.saturating_sub(entry.bytes);
        if let Some(bytes) = self.peer_bytes.get_mut(&author) {
            *bytes = bytes.saturating_sub(entry.bytes);
        }
        let mut payload = Vec::with_capacity(entry.fragment.payload_len);
        for chunk in entry.chunks {
            payload.extend(chunk.expect("all fragments complete"));
        }
        if payload.len() != entry.fragment.payload_len
            || blake3::hash(&payload).to_hex().to_string() != entry.fragment.payload_hash_hex
        {
            return Err(app_error(
                &entry.fragment.topic,
                "reassembled payload hash mismatch",
            ));
        }
        let message = AppMessage {
            schema_version: entry.fragment.message_schema_version,
            network_id: entry.fragment.network_id,
            topic: entry.fragment.topic,
            source_peer_id: entry.fragment.source_peer_id,
            target_peer_id: entry.fragment.target_peer_id,
            timestamp_ns: entry.fragment.timestamp_ns,
            nonce_hex: entry.fragment.nonce_hex,
            payload,
        };
        validate_app_message(&message)?;
        Ok(Some(message))
    }

    fn expire(&mut self) {
        let now = Instant::now();
        let expired: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(key, entry)| {
                (now.duration_since(entry.created) >= FRAGMENT_EXPIRY).then_some(key.clone())
            })
            .collect();
        for key in expired {
            if let Some(entry) = self.entries.remove(&key) {
                self.global_bytes = self.global_bytes.saturating_sub(entry.bytes);
                if let Some(bytes) = self.peer_bytes.get_mut(&key.author) {
                    *bytes = bytes.saturating_sub(entry.bytes);
                }
            }
        }
        self.peer_bytes.retain(|_, bytes| *bytes > 0);
    }
}

fn same_message(a: &AppFragment, b: &AppFragment) -> bool {
    a.message_schema_version == b.message_schema_version
        && a.network_id == b.network_id
        && a.topic == b.topic
        && a.source_peer_id == b.source_peer_id
        && a.target_peer_id == b.target_peer_id
        && a.timestamp_ns == b.timestamp_ns
        && a.nonce_hex == b.nonce_hex
        && a.payload_hash_hex == b.payload_hash_hex
        && a.payload_len == b.payload_len
        && a.total == b.total
}

fn app_error(topic: impl Into<String>, reason: impl Into<String>) -> NetError {
    NetError::AppMessage {
        topic: topic.into(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_payload_round_trips_through_bounded_fragments() {
        let peer = PeerId::random();
        let message =
            AppMessage::broadcast(1, "fragment-test", peer, vec![0x5a; MAX_APP_MESSAGE_BYTES])
                .unwrap();
        let wires = fragment_message(&message).unwrap();
        assert!(wires.len() > 1);
        assert!(wires
            .iter()
            .all(|wire| wire.len() <= MAX_FRAGMENT_WIRE_BYTES));
        let mut reassembler = AppFragmentReassembler::default();
        let mut result = None;
        for wire in wires {
            let fragment = decode_fragment(&wire).unwrap();
            result = reassembler.push(peer, fragment).unwrap().or(result);
        }
        assert_eq!(result, Some(message));
    }

    #[test]
    fn duplicate_fragment_does_not_double_count_or_complete_early() {
        let peer = PeerId::random();
        let message = AppMessage::broadcast(
            1,
            "fragment-test",
            peer,
            vec![7; APP_FRAGMENT_PAYLOAD_BYTES + 1],
        )
        .unwrap();
        let wires = fragment_message(&message).unwrap();
        let first = decode_fragment(&wires[0]).unwrap();
        let mut reassembler = AppFragmentReassembler::default();
        assert!(reassembler.push(peer, first.clone()).unwrap().is_none());
        assert!(reassembler.push(peer, first).unwrap().is_none());
        let second = decode_fragment(&wires[1]).unwrap();
        assert_eq!(reassembler.push(peer, second).unwrap(), Some(message));
    }
}
