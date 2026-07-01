//! Application discovery namespace derivation.
//!
//! Discovery tags let applications publish and search for rendezvous/DHT
//! namespaces without exposing raw contact names, invite phrases, or other
//! human-readable intent. The default mode hashes every tag into a deterministic
//! namespace key. Readable tags are available only when explicitly enabled for
//! local debugging.

use serde::{Deserialize, Serialize};

pub const DISCOVERY_NAMESPACE_PREFIX: &str = "p2p-net";
pub const DISCOVERY_NAMESPACE_HASH_CONTEXT: &str = "p2p-net.discovery.namespace.v1";
pub const MAX_DISCOVERY_APP_ID_LEN: usize = 64;
pub const MAX_DISCOVERY_TAG_LEN: usize = 256;
pub const MAX_DISCOVERY_TAGS: usize = 64;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryNamespacePrivacy {
    /// Publish only deterministic BLAKE3-derived tag hashes.
    #[default]
    Hashed,
    /// Publish readable tags. This leaks app/contact intent and is accepted only
    /// when `allow_readable_tags` is true.
    ReadableUnsafe,
}

impl DiscoveryNamespacePrivacy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hashed => "hashed",
            Self::ReadableUnsafe => "readable_unsafe",
        }
    }
}

/// Config for deriving app-level discovery namespaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoveryNamespaceConfig {
    /// Stable app identifier included in every derived namespace.
    pub app_id: String,
    /// Contact/group/app discovery tags. In normal mode these are hashed before
    /// publication; they should not appear on the wire in plaintext.
    pub tags: Vec<String>,
    /// Privacy mode for tag publication. Defaults to hashed.
    pub privacy: DiscoveryNamespacePrivacy,
    /// Required guardrail for readable debug tags.
    pub allow_readable_tags: bool,
}

impl Default for DiscoveryNamespaceConfig {
    fn default() -> Self {
        Self {
            app_id: "p2p-net".to_string(),
            tags: Vec::new(),
            privacy: DiscoveryNamespacePrivacy::Hashed,
            allow_readable_tags: false,
        }
    }
}

impl DiscoveryNamespaceConfig {
    pub fn validate(&self) -> Result<(), crate::common::error::NetError> {
        validate_app_id(&self.app_id, "discovery.namespace.app_id")?;
        if self.tags.len() > MAX_DISCOVERY_TAGS {
            return Err(config_error(format!(
                "discovery.namespace.tags supports at most {MAX_DISCOVERY_TAGS} entries"
            )));
        }
        if self.privacy == DiscoveryNamespacePrivacy::ReadableUnsafe && !self.allow_readable_tags {
            return Err(config_error(
                "discovery.namespace.privacy=readable_unsafe requires allow_readable_tags=true",
            ));
        }
        for (idx, tag) in self.tags.iter().enumerate() {
            validate_tag(tag, &format!("discovery.namespace.tags[{idx}]"))?;
        }
        Ok(())
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        !self.tags.is_empty()
    }

    pub fn derived_namespaces(
        &self,
        network_id: u32,
    ) -> Result<Vec<DiscoveryNamespace>, crate::common::error::NetError> {
        self.validate()?;
        let mut namespaces = Vec::new();
        for tag in &self.tags {
            let namespace = build_discovery_namespace(
                network_id,
                &self.app_id,
                tag,
                self.privacy,
                self.allow_readable_tags,
            )?;
            if !namespaces.iter().any(|known| known.namespace == namespace.namespace) {
                namespaces.push(namespace);
            }
        }
        Ok(namespaces)
    }
}

/// Fully derived namespace metadata. The raw tag is intentionally not stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryNamespace {
    pub network_id: u32,
    pub app_id: String,
    pub namespace: String,
    pub tag_fingerprint_hex: String,
    pub privacy: DiscoveryNamespacePrivacy,
}

/// Derive a deterministic namespace string from network, app, and tag.
pub fn build_discovery_namespace(
    network_id: u32,
    app_id: &str,
    tag: &str,
    privacy: DiscoveryNamespacePrivacy,
    allow_readable_tags: bool,
) -> Result<DiscoveryNamespace, crate::common::error::NetError> {
    validate_app_id(app_id, "app_id")?;
    validate_tag(tag, "tag")?;
    if privacy == DiscoveryNamespacePrivacy::ReadableUnsafe && !allow_readable_tags {
        return Err(config_error(
            "readable discovery namespaces require allow_readable_tags=true",
        ));
    }

    let app_id = normalize_namespace_segment(app_id)?;
    let fingerprint = discovery_tag_hash_hex(network_id, &app_id, tag);
    let tag_segment = match privacy {
        DiscoveryNamespacePrivacy::Hashed => fingerprint.clone(),
        DiscoveryNamespacePrivacy::ReadableUnsafe => normalize_namespace_segment(tag)?,
    };
    let namespace = format!("{DISCOVERY_NAMESPACE_PREFIX}/{network_id}/{app_id}/{tag_segment}");

    Ok(DiscoveryNamespace {
        network_id,
        app_id,
        namespace,
        tag_fingerprint_hex: fingerprint,
        privacy,
    })
}

/// Hash tag material with explicit domain separation.
#[must_use]
pub fn discovery_tag_hash_hex(network_id: u32, app_id: &str, tag: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(DISCOVERY_NAMESPACE_HASH_CONTEXT.as_bytes());
    hasher.update(&network_id.to_be_bytes());
    hasher.update(&[0]);
    hasher.update(app_id.as_bytes());
    hasher.update(&[0]);
    hasher.update(tag.as_bytes());
    hasher.finalize().to_hex().to_string()
}

pub fn normalize_namespace_segment(
    value: &str,
) -> Result<String, crate::common::error::NetError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(config_error("namespace segment must not be empty"));
    }
    let mut out = String::new();
    for ch in trimmed.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' => out.push(ch.to_ascii_lowercase()),
            '-' | '_' | '.' => out.push(ch),
            _ => out.push('-'),
        }
    }
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        return Err(config_error(
            "namespace segment must contain at least one alphanumeric character",
        ));
    }
    Ok(out)
}

fn validate_app_id(value: &str, field: &str) -> Result<(), crate::common::error::NetError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(config_error(format!("{field} must not be empty")));
    }
    if trimmed.len() > MAX_DISCOVERY_APP_ID_LEN {
        return Err(config_error(format!(
            "{field} must be at most {MAX_DISCOVERY_APP_ID_LEN} bytes"
        )));
    }
    Ok(())
}

fn validate_tag(value: &str, field: &str) -> Result<(), crate::common::error::NetError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(config_error(format!("{field} must not be empty")));
    }
    if trimmed.len() > MAX_DISCOVERY_TAG_LEN {
        return Err(config_error(format!(
            "{field} must be at most {MAX_DISCOVERY_TAG_LEN} bytes"
        )));
    }
    Ok(())
}

fn config_error(reason: impl Into<String>) -> crate::common::error::NetError {
    crate::common::error::NetError::Config {
        path: "<config>".to_string(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashed_namespace_hides_raw_tag() {
        let tag = "IAmJoeTheRealJoeYouWant213423432";
        let ns = build_discovery_namespace(
            1,
            "hydra-msg",
            tag,
            DiscoveryNamespacePrivacy::Hashed,
            false,
        )
        .expect("namespace");

        assert!(ns.namespace.starts_with("p2p-net/1/hydra-msg/"));
        assert!(!ns.namespace.contains(tag));
        assert_eq!(ns.tag_fingerprint_hex.len(), 64);
    }

    #[test]
    fn readable_namespace_requires_explicit_guardrail() {
        assert!(build_discovery_namespace(
            1,
            "hydra-msg",
            "Joe",
            DiscoveryNamespacePrivacy::ReadableUnsafe,
            false,
        )
        .is_err());
        let ns = build_discovery_namespace(
            1,
            "hydra-msg",
            "Joe",
            DiscoveryNamespacePrivacy::ReadableUnsafe,
            true,
        )
        .expect("readable debug namespace");
        assert!(ns.namespace.ends_with("/joe"));
    }
}
