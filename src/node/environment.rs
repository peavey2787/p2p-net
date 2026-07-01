//! Advisory platform and network environment detection for profile resolution.
//!
//! Detection is advisory-only: it lets callers resolve `profile = auto` against
//! an `EnvironmentReport`, but it does not change live swarm behaviour at
//! runtime. The detector combines static config, platform hints, and runtime
//! observations into one DRY policy input.

use serde::{Deserialize, Serialize};

use crate::platform::PlatformRuntime;

use super::types::NodeConfig;

/// Optional user/platform hints used by embedders that already know facts the
/// portable detector cannot know without network probing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EnvironmentConfig {
    /// Override compile-time platform detection. Useful for tests and for app
    /// shells that run the core through bindings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_hint: Option<PlatformKind>,
    /// Reachability result supplied by an external probe or operator config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reachability_hint: Option<NetworkReachability>,
    /// NAT result supplied by AutoNAT or an external probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nat_hint: Option<NatKind>,
    /// Whether TCP listen sockets are expected to work on this platform/env.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_listen_tcp: Option<bool>,
    /// Whether QUIC/UDP listen sockets are expected to work on this platform/env.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_listen_quic: Option<bool>,
    /// Whether peers are expected to reach this node directly from outside the LAN.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_accept_inbound: Option<bool>,
    /// Whether the node is likely behind CGNAT or equivalent inbound blocking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub likely_cgnat: Option<bool>,
    /// Whether runtime policy should avoid heavy always-on background work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery_sensitive: Option<bool>,
    /// Whether the app/platform may suspend background networking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_restricted: Option<bool>,
}

/// Portable platform category used for role selection. Keep this enum coarse so
/// the shared core does not learn UI-shell-specific details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformKind {
    Windows,
    Linux,
    Macos,
    Android,
    Ios,
    Wasm,
    Unknown,
}

impl PlatformKind {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "android") {
            Self::Android
        } else if cfg!(target_os = "ios") {
            Self::Ios
        } else if cfg!(target_arch = "wasm32") {
            Self::Wasm
        } else {
            Self::Unknown
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Android => "android",
            Self::Ios => "ios",
            Self::Wasm => "wasm",
            Self::Unknown => "unknown",
        }
    }

    pub fn is_mobile(self) -> bool {
        matches!(self, Self::Android | Self::Ios)
    }
}

/// Public reachability classification. This is intentionally separate from the
/// lower-level NAT kind because a node may know one but not the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkReachability {
    Public,
    PrivateNat,
    CgnatLikely,
    Unknown,
}

impl NetworkReachability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::PrivateNat => "private_nat",
            Self::CgnatLikely => "cgnat_likely",
            Self::Unknown => "unknown",
        }
    }
}

/// NAT status as understood by the profile resolver. Future event plumbing can
/// map libp2p AutoNAT events into this enum without exposing libp2p types here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NatKind {
    Public,
    Private,
    CgnatLikely,
    Symmetric,
    Restricted,
    Unknown,
}

impl NatKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
            Self::CgnatLikely => "cgnat_likely",
            Self::Symmetric => "symmetric",
            Self::Restricted => "restricted",
            Self::Unknown => "unknown",
        }
    }

    pub fn implies_cgnat_or_blocked(self) -> bool {
        matches!(self, Self::CgnatLikely | Self::Symmetric | Self::Restricted)
    }
}

/// Advisory report consumed by profile/capability resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentReport {
    pub platform: PlatformKind,
    pub reachability: NetworkReachability,
    pub nat_status: NatKind,
    pub can_listen_tcp: bool,
    pub can_listen_quic: bool,
    pub can_accept_inbound: bool,
    pub likely_cgnat: bool,
    pub battery_sensitive: bool,
    pub background_restricted: bool,
    pub observed_external_addresses: Vec<String>,
    pub listen_failures: usize,
}

impl EnvironmentReport {
    /// Build an advisory report from static config and compile-time platform facts.
    pub fn detect(cfg: &NodeConfig) -> Self {
        Self::detect_with_optional_runtime(cfg, None)
    }

    /// Build an advisory report from static config plus a platform runtime adapter.
    /// Explicit `NodeConfig.environment` fields override runtime-provided hints.
    pub fn detect_with_runtime(cfg: &NodeConfig, runtime: &dyn PlatformRuntime) -> Self {
        Self::detect_with_optional_runtime(cfg, Some(runtime))
    }

    fn detect_with_optional_runtime(
        cfg: &NodeConfig,
        runtime: Option<&dyn PlatformRuntime>,
    ) -> Self {
        let runtime_hints = runtime.map(|rt| rt.environment_config());
        let hints = &cfg.environment;
        let platform = hints
            .platform_hint
            .or(runtime_hints.as_ref().and_then(|v| v.platform_hint))
            .unwrap_or(PlatformKind::current());
        let can_listen_tcp = hints
            .can_listen_tcp
            .or(runtime_hints.as_ref().and_then(|v| v.can_listen_tcp))
            .unwrap_or(listen_addresses_include(&cfg.listen_addresses, "/tcp/"));
        let can_listen_quic = hints
            .can_listen_quic
            .or(runtime_hints.as_ref().and_then(|v| v.can_listen_quic))
            .unwrap_or(
                cfg.listen_addresses
                    .iter()
                    .any(|addr| addr.contains("/udp/") && addr.contains("/quic")),
            );
        let nat_status = hints
            .nat_hint
            .or(runtime_hints.as_ref().and_then(|v| v.nat_hint))
            .unwrap_or(NatKind::Unknown);
        let reachability_hint = hints
            .reachability_hint
            .or(runtime_hints.as_ref().and_then(|v| v.reachability_hint));
        let inferred_cgnat = nat_status.implies_cgnat_or_blocked()
            || matches!(reachability_hint, Some(NetworkReachability::CgnatLikely));
        let likely_cgnat = hints
            .likely_cgnat
            .or(runtime_hints.as_ref().and_then(|v| v.likely_cgnat))
            .unwrap_or(inferred_cgnat);
        let battery_sensitive = hints
            .battery_sensitive
            .or(runtime_hints.as_ref().and_then(|v| v.battery_sensitive))
            .unwrap_or(platform.is_mobile());
        let background_restricted = hints
            .background_restricted
            .or(runtime_hints.as_ref().and_then(|v| v.background_restricted))
            .unwrap_or(platform.is_mobile() || matches!(platform, PlatformKind::Wasm));
        let reachability = reachability_hint.unwrap_or(reachability_from_hints(
            nat_status,
            likely_cgnat,
            background_restricted,
        ));
        let can_accept_inbound = hints
            .can_accept_inbound
            .or(runtime_hints.as_ref().and_then(|v| v.can_accept_inbound))
            .unwrap_or(
                matches!(reachability, NetworkReachability::Public)
                    && (can_listen_tcp || can_listen_quic)
                    && !background_restricted,
            );

        Self {
            platform,
            reachability,
            nat_status,
            can_listen_tcp,
            can_listen_quic,
            can_accept_inbound,
            likely_cgnat,
            battery_sensitive,
            background_restricted,
            observed_external_addresses: Vec::new(),
            listen_failures: 0,
        }
    }

    /// Apply a runtime AutoNAT-style observation while keeping detection logic
    /// centralized and independent from libp2p event types.
    pub fn apply_nat_observation(&mut self, nat_status: NatKind) {
        self.nat_status = nat_status;
        self.reachability = match nat_status {
            NatKind::Public => NetworkReachability::Public,
            NatKind::Private => NetworkReachability::PrivateNat,
            NatKind::CgnatLikely | NatKind::Symmetric | NatKind::Restricted => {
                NetworkReachability::CgnatLikely
            }
            NatKind::Unknown => self.reachability,
        };
        self.likely_cgnat = nat_status.implies_cgnat_or_blocked();
        self.can_accept_inbound = matches!(self.reachability, NetworkReachability::Public)
            && (self.can_listen_tcp || self.can_listen_quic)
            && !self.background_restricted;
    }

    /// Record an externally observed address from identify/AutoNAT style events.
    pub fn add_observed_external_address(&mut self, addr: impl Into<String>) {
        let addr = addr.into();
        if !self.observed_external_addresses.contains(&addr) {
            self.observed_external_addresses.push(addr);
        }
    }

    /// Record a listen failure. Enough failures make the report conservative for
    /// future auto-profile resolution.
    pub fn record_listen_failure(&mut self) {
        self.listen_failures = self.listen_failures.saturating_add(1);
        if self.listen_failures > 0 && !self.can_accept_inbound {
            self.reachability = NetworkReachability::PrivateNat;
        }
    }
}

fn reachability_from_hints(
    nat_status: NatKind,
    likely_cgnat: bool,
    background_restricted: bool,
) -> NetworkReachability {
    if likely_cgnat {
        return NetworkReachability::CgnatLikely;
    }
    match nat_status {
        NatKind::Public => NetworkReachability::Public,
        NatKind::Private => NetworkReachability::PrivateNat,
        NatKind::CgnatLikely | NatKind::Symmetric | NatKind::Restricted => {
            NetworkReachability::CgnatLikely
        }
        NatKind::Unknown if background_restricted => NetworkReachability::PrivateNat,
        NatKind::Unknown => NetworkReachability::Unknown,
    }
}

fn listen_addresses_include(listen_addresses: &[String], needle: &str) -> bool {
    listen_addresses.iter().any(|addr| addr.contains(needle))
}
