//! Platform adapters for the shared P2P core.
//!
//! Platform policy/storage stays in dedicated modules. UI, lifecycle service,
//! JNI, and operating-system application code live under `apps/`.

pub mod android;
pub mod desktop;
pub mod ios;
pub mod memory;
pub mod traits;

pub use android::AndroidPlatformRuntime;
pub use desktop::DesktopPlatformRuntime;
pub use ios::IosPlatformRuntime;
pub use memory::MemoryNodeStorage;
pub use traits::{NodeStorage, PlatformRuntime};
