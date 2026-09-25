//! Target-neutral runtime driver dispatch.

#[cfg(not(target_arch = "wasm32"))]
#[path = "driver_native.rs"]
mod native;
#[cfg(target_arch = "wasm32")]
#[path = "driver_wasm.rs"]
mod wasm;

#[cfg(not(target_arch = "wasm32"))]
pub(super) use native::run_node_runtime;
#[cfg(target_arch = "wasm32")]
pub(super) use wasm::run_node_runtime;
