//! Binding-safe facade for app shells.
//!
//! This module does not implement a second P2P node. It exposes small,
//! serialization-friendly helpers that Kotlin, Swift, desktop, or future FFI
//! layers can wrap while still starting the same shared Rust core through
//! `start_node_with_platform`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::common::error::NetError;
use crate::{
    snapshot_to_json, BehaviourSet, EnvironmentConfig, EnvironmentReport, NodeConfig, NodeProfile,
    NodeRole, NodeSnapshot, PlatformKind, PlatformRuntime, ResolvedNodeConfig,
};

/// Coarse app-shell target for generated or handwritten bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BindingTarget {
    /// Windows, Linux, and macOS desktop shells.
    #[default]
    Desktop,
    /// Android phones and tablets.
    Android,
    /// iPhone and iPad shells.
    Ios,
    /// Browser/WASM-style hosts where long-running listen sockets are not assumed.
    Wasm,
}

impl BindingTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Android => "android",
            Self::Ios => "ios",
            Self::Wasm => "wasm",
        }
    }

    pub fn platform_kind(self) -> PlatformKind {
        match self {
            Self::Desktop => PlatformKind::current(),
            Self::Android => PlatformKind::Android,
            Self::Ios => PlatformKind::Ios,
            Self::Wasm => PlatformKind::Wasm,
        }
    }

    pub fn default_can_listen_tcp(self) -> bool {
        matches!(self, Self::Desktop)
    }

    pub fn default_can_listen_quic(self) -> bool {
        matches!(self, Self::Desktop)
    }

    pub fn default_can_accept_inbound(self) -> Option<bool> {
        if matches!(self, Self::Desktop) {
            None
        } else {
            Some(false)
        }
    }

    pub fn default_battery_sensitive(self) -> bool {
        !matches!(self, Self::Desktop)
    }

    pub fn default_background_restricted(self) -> bool {
        !matches!(self, Self::Desktop)
    }
}

/// Storage strategy requested by a binding layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BindingStorageStrategy {
    /// Use the platform adapter's default storage strategy.
    #[default]
    PlatformDefault,
    /// The embedding shell supplies a `NodeStorage` implementation.
    ExternalPlatformStorage,
    /// In-memory storage, intended only for tests, previews, and ephemeral demos.
    MemoryTestingOnly,
}

/// Storage obligation produced by `prepare_binding_start_plan`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingStorageRequirement {
    DesktopFilesystem,
    ExternalPlatformStorage,
    MemoryTestingOnly,
}

impl BindingStorageRequirement {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DesktopFilesystem => "desktop_filesystem",
            Self::ExternalPlatformStorage => "external_platform_storage",
            Self::MemoryTestingOnly => "memory_testing_only",
        }
    }
}

/// Runtime hints supplied by a binding host before starting the shared node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BindingRuntimeSpec {
    pub target: BindingTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<String>,
    #[serde(default)]
    pub storage: BindingStorageStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_listen_tcp: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_listen_quic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_accept_inbound: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery_sensitive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_restricted: Option<bool>,
}

impl BindingRuntimeSpec {
    pub fn desktop(data_dir: Option<String>) -> Self {
        Self {
            target: BindingTarget::Desktop,
            data_dir,
            ..Self::default()
        }
    }

    pub fn android(data_dir: Option<String>) -> Self {
        Self {
            target: BindingTarget::Android,
            data_dir,
            ..Self::default()
        }
    }

    pub fn ios(data_dir: Option<String>) -> Self {
        Self {
            target: BindingTarget::Ios,
            data_dir,
            ..Self::default()
        }
    }

    pub fn wasm() -> Self {
        Self {
            target: BindingTarget::Wasm,
            ..Self::default()
        }
    }

    pub fn runtime(&self) -> BindingPlatformRuntime {
        BindingPlatformRuntime { spec: self.clone() }
    }

    pub fn environment_config(&self) -> EnvironmentConfig {
        EnvironmentConfig {
            platform_hint: Some(self.target.platform_kind()),
            reachability_hint: None,
            nat_hint: None,
            can_listen_tcp: Some(
                self.can_listen_tcp
                    .unwrap_or(self.target.default_can_listen_tcp()),
            ),
            can_listen_quic: Some(
                self.can_listen_quic
                    .unwrap_or(self.target.default_can_listen_quic()),
            ),
            can_accept_inbound: self
                .can_accept_inbound
                .or(self.target.default_can_accept_inbound()),
            likely_cgnat: None,
            battery_sensitive: Some(
                self.battery_sensitive
                    .unwrap_or(self.target.default_battery_sensitive()),
            ),
            background_restricted: Some(
                self.background_restricted
                    .unwrap_or(self.target.default_background_restricted()),
            ),
        }
    }

    pub fn storage_requirement(&self) -> BindingStorageRequirement {
        match self.storage {
            BindingStorageStrategy::MemoryTestingOnly => {
                BindingStorageRequirement::MemoryTestingOnly
            }
            BindingStorageStrategy::ExternalPlatformStorage => {
                BindingStorageRequirement::ExternalPlatformStorage
            }
            BindingStorageStrategy::PlatformDefault
                if matches!(self.target, BindingTarget::Desktop) =>
            {
                BindingStorageRequirement::DesktopFilesystem
            }
            BindingStorageStrategy::PlatformDefault => {
                BindingStorageRequirement::ExternalPlatformStorage
            }
        }
    }
}

/// Runtime adapter derived from `BindingRuntimeSpec`.
#[derive(Debug, Clone)]
pub struct BindingPlatformRuntime {
    spec: BindingRuntimeSpec,
}

impl PlatformRuntime for BindingPlatformRuntime {
    fn runtime_name(&self) -> &'static str {
        match self.spec.target {
            BindingTarget::Desktop => "binding_desktop",
            BindingTarget::Android => "binding_android",
            BindingTarget::Ios => "binding_ios",
            BindingTarget::Wasm => "binding_wasm",
        }
    }

    fn platform_kind(&self) -> PlatformKind {
        self.spec.target.platform_kind()
    }

    fn default_data_dir(&self) -> Option<PathBuf> {
        self.spec
            .data_dir
            .as_ref()
            .map(|path| PathBuf::from(path.as_str()))
    }

    fn can_listen_tcp(&self) -> bool {
        self.spec
            .can_listen_tcp
            .unwrap_or(self.spec.target.default_can_listen_tcp())
    }

    fn can_listen_quic(&self) -> bool {
        self.spec
            .can_listen_quic
            .unwrap_or(self.spec.target.default_can_listen_quic())
    }

    fn can_accept_inbound(&self) -> Option<bool> {
        self.spec
            .can_accept_inbound
            .or(self.spec.target.default_can_accept_inbound())
    }

    fn is_battery_sensitive(&self) -> bool {
        self.spec
            .battery_sensitive
            .unwrap_or(self.spec.target.default_battery_sensitive())
    }

    fn is_background_restricted(&self) -> bool {
        self.spec
            .background_restricted
            .unwrap_or(self.spec.target.default_background_restricted())
    }
}

/// Binding-friendly summary of the resolved startup plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingStartPlan {
    pub target: BindingTarget,
    pub platform: PlatformKind,
    pub runtime_name: String,
    pub storage_requirement: BindingStorageRequirement,
    pub profile: NodeProfile,
    pub resolved_role: NodeRole,
    pub should_listen: bool,
    pub enabled_behaviours: BehaviourSet,
    pub relay_discovery_enabled: bool,
    pub dcutr_enabled: bool,
    pub environment: EnvironmentReport,
    pub warnings: Vec<String>,
}

/// Static support matrix for app-shell planning and documentation UIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingSupportMatrix {
    pub shared_rust_core: bool,
    pub separate_node_implementations_required: bool,
    pub recommended_binding_layer: String,
    pub targets: Vec<BindingTargetInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingTargetInfo {
    pub target: BindingTarget,
    pub platform: PlatformKind,
    pub app_shell: String,
    pub runtime_adapter: String,
    pub storage_requirement: BindingStorageRequirement,
    pub background_limited: bool,
}

pub fn binding_support_matrix() -> BindingSupportMatrix {
    let targets = [
        (
            BindingTarget::Desktop,
            "Tauri, egui, native desktop, or CLI",
        ),
        (BindingTarget::Android, "Kotlin/Android shell"),
        (BindingTarget::Ios, "Swift/iOS shell"),
        (
            BindingTarget::Wasm,
            "browser or WebView host with restricted networking",
        ),
    ]
    .into_iter()
    .map(|(target, app_shell)| {
        let spec = BindingRuntimeSpec {
            target,
            ..BindingRuntimeSpec::default()
        };
        BindingTargetInfo {
            target,
            platform: target.platform_kind(),
            app_shell: app_shell.to_string(),
            runtime_adapter: spec.runtime().runtime_name().to_string(),
            storage_requirement: spec.storage_requirement(),
            background_limited: target.default_background_restricted(),
        }
    })
    .collect();

    BindingSupportMatrix {
        shared_rust_core: true,
        separate_node_implementations_required: false,
        recommended_binding_layer: "Use this JSON/enum facade from UniFFI, a C ABI, or a native host wrapper; keep networking in p2p-net.".to_string(),
        targets,
    }
}

/// Parse and validate a user config supplied through a binding layer.
pub fn node_config_from_json(raw: &str) -> Result<NodeConfig, NetError> {
    let cfg: NodeConfig = serde_json::from_str(raw).map_err(|err| NetError::Config {
        path: "<binding-json>".to_string(),
        reason: err.to_string(),
    })?;
    cfg.validate()?;
    Ok(cfg)
}

/// Serialize config back to pretty JSON for binding hosts that store config as text.
pub fn node_config_to_json(cfg: &NodeConfig) -> Result<String, NetError> {
    cfg.to_pretty_json()
}

/// Serialize snapshots to a stable JSON string for host UI layers.
pub fn node_snapshot_to_json_string(snapshot: &NodeSnapshot) -> Result<String, NetError> {
    serde_json::to_string_pretty(&snapshot_to_json(snapshot)).map_err(|err| NetError::Config {
        path: "<binding-snapshot-json>".to_string(),
        reason: err.to_string(),
    })
}

/// Resolve the config exactly as a binding host would before calling
/// `start_node_with_platform(config, runtime, storage)`.
pub fn prepare_binding_start_plan(
    config_json: &str,
    runtime_spec: &BindingRuntimeSpec,
) -> Result<BindingStartPlan, NetError> {
    let cfg = node_config_from_json(config_json)?;
    let runtime = runtime_spec.runtime();
    let environment = cfg.environment_report_with_runtime(&runtime);
    let resolved = cfg.try_resolved_for_environment(&environment)?;
    let warnings = binding_warnings(runtime_spec, &resolved);

    Ok(BindingStartPlan {
        target: runtime_spec.target,
        platform: environment.platform,
        runtime_name: runtime.runtime_name().to_string(),
        storage_requirement: runtime_spec.storage_requirement(),
        profile: resolved.profile,
        resolved_role: resolved.role,
        should_listen: resolved.should_listen,
        enabled_behaviours: resolved.enabled_behaviours.clone(),
        relay_discovery_enabled: resolved.relay_discovery_enabled,
        dcutr_enabled: resolved.dcutr_enabled,
        environment,
        warnings,
    })
}

fn binding_warnings(
    runtime_spec: &BindingRuntimeSpec,
    resolved: &ResolvedNodeConfig,
) -> Vec<String> {
    let mut warnings = Vec::new();

    if matches!(
        runtime_spec.storage_requirement(),
        BindingStorageRequirement::MemoryTestingOnly
    ) {
        warnings.push(
            "memory storage is ephemeral and should not be used for production identities"
                .to_string(),
        );
    }

    if matches!(
        runtime_spec.storage_requirement(),
        BindingStorageRequirement::ExternalPlatformStorage
    ) {
        warnings.push(
            "host shell must provide durable NodeStorage for identity and peer cache persistence"
                .to_string(),
        );
    }

    if runtime_spec.target.default_background_restricted() && resolved.should_listen {
        warnings.push(
            "background-restricted targets should not run listener/infrastructure roles"
                .to_string(),
        );
    }

    warnings
}
