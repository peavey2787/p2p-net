//! Platform adapters for the shared P2P core.

pub mod desktop;
pub mod memory;
pub mod mobile;
pub mod traits;

pub use desktop::DesktopPlatformRuntime;
pub use memory::MemoryNodeStorage;
pub use mobile::MobilePlatformRuntime;
pub use traits::{NodeStorage, PlatformRuntime};
