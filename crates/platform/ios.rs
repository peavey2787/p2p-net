//! iOS runtime policy adapter.
//!
//! iOS shell/binding code is intentionally outside the shared networking core.

use std::path::PathBuf;

use crate::PlatformKind;

use super::PlatformRuntime;

#[derive(Debug, Clone)]
pub struct IosPlatformRuntime {
    data_dir: Option<PathBuf>,
    can_listen_tcp: bool,
    can_listen_quic: bool,
    can_accept_inbound: Option<bool>,
    battery_sensitive: bool,
    background_restricted: bool,
}

impl IosPlatformRuntime {
    pub fn new(data_dir: Option<PathBuf>) -> Self {
        Self {
            data_dir,
            can_listen_tcp: false,
            can_listen_quic: false,
            can_accept_inbound: Some(false),
            battery_sensitive: true,
            background_restricted: true,
        }
    }

    pub fn with_listen_capability(mut self, tcp: bool, quic: bool) -> Self {
        self.can_listen_tcp = tcp;
        self.can_listen_quic = quic;
        self
    }

    pub fn with_background_restricted(mut self, restricted: bool) -> Self {
        self.background_restricted = restricted;
        self
    }

    pub fn with_battery_sensitive(mut self, sensitive: bool) -> Self {
        self.battery_sensitive = sensitive;
        self
    }

    pub fn with_inbound_capability(mut self, can_accept_inbound: Option<bool>) -> Self {
        self.can_accept_inbound = can_accept_inbound;
        self
    }
}

impl PlatformRuntime for IosPlatformRuntime {
    fn runtime_name(&self) -> &'static str {
        "ios"
    }

    fn platform_kind(&self) -> PlatformKind {
        PlatformKind::Ios
    }

    fn default_data_dir(&self) -> Option<PathBuf> {
        self.data_dir.clone()
    }

    fn can_listen_tcp(&self) -> bool {
        self.can_listen_tcp
    }

    fn can_listen_quic(&self) -> bool {
        self.can_listen_quic
    }

    fn can_accept_inbound(&self) -> Option<bool> {
        self.can_accept_inbound
    }

    fn is_battery_sensitive(&self) -> bool {
        self.battery_sensitive
    }

    fn is_background_restricted(&self) -> bool {
        self.background_restricted
    }
}
