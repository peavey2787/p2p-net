//! Target-neutral execution boundary used by the node core.
//!
//! Native builds use Tokio. Browser/WASM builds use the browser event loop via
//! `wasm_bindgen_futures::spawn_local` and `futures-timer`.  No browser object
//! crosses this boundary into `NodeHandle` or the protocol layers. `sleep` is
//! browser-only: the native driver schedules through Tokio intervals directly.

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::{spawn, timeout, TaskHandle};
#[cfg(target_arch = "wasm32")]
pub(crate) use wasm::{sleep, spawn, timeout, TaskHandle};
