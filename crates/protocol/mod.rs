//! Internal protocol messages used by the node runtime.
//!
//! Application payload envelopes live in `crate::api` because they are part of
//! the stable app-facing API rather than heartbeat/reputation internals.

pub(crate) mod app_security;
pub mod pulse;
pub mod reputation;
