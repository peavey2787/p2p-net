//! Platform adapters for the shared P2P core.
//!
//! Platform policy/storage stays in dedicated modules. UI, lifecycle service,
//! JNI, and operating-system application code live under `apps/`.

#[cfg(not(target_arch = "wasm32"))]
pub mod android;
#[cfg(not(target_arch = "wasm32"))]
pub mod desktop;
#[cfg(not(target_arch = "wasm32"))]
pub mod ios;
pub mod memory;
pub mod traits;
#[cfg(target_arch = "wasm32")]
pub mod wasm;

#[cfg(not(target_arch = "wasm32"))]
pub use android::AndroidPlatformRuntime;
#[cfg(not(target_arch = "wasm32"))]
pub use desktop::DesktopPlatformRuntime;
#[cfg(not(target_arch = "wasm32"))]
pub use ios::IosPlatformRuntime;
pub use memory::MemoryNodeStorage;
pub use traits::{NodeStorage, PlatformRuntime};
#[cfg(target_arch = "wasm32")]
pub use wasm::{BrowserNodeStorage, BrowserPlatformRuntime};
