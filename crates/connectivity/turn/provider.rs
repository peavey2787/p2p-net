//! TURN credential providers.
//!
//! Every provider turns its own API into the shared [`TurnCredentials`]
//! bundle. Adding a provider means adding one module with a config struct,
//! `validate`, `trusted_hosts` and `issue`, plus one variant here; nothing
//! else in the node changes. Edge templates that wrap a provider's API in the
//! generic [`HttpProviderConfig`] contract live under `providers/turn/`.

use serde::{Deserialize, Serialize};

use crate::common::error::NetError;

use super::credentials::TurnCredentials;

mod cloudflare;
mod http;
mod shared_secret;

pub use cloudflare::{CloudflareProviderConfig, CLOUDFLARE_TURN_HOST};
pub use http::HttpProviderConfig;
pub use shared_secret::SharedSecretProviderConfig;

/// Where TURN credentials come from. JSON uses a `type` tag, e.g.
/// `{"type": "cloudflare", "key_id": "...", "api_token": {"env": "CF_TURN_TOKEN"}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnProviderConfig {
    /// Any HTTPS endpoint returning the [`TurnCredentials`] JSON contract
    /// (for example the Cloudflare Worker template in `providers/turn/cloudflare`).
    Http(HttpProviderConfig),
    /// Cloudflare Realtime TURN, called directly with an API token.
    Cloudflare(CloudflareProviderConfig),
    /// TURN REST API shared secret (coturn `use-auth-secret`, or another
    /// p2p-net node's built-in TURN server).
    SharedSecret(SharedSecretProviderConfig),
}

impl TurnProviderConfig {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Http(_) => "http",
            Self::Cloudflare(_) => "cloudflare",
            Self::SharedSecret(_) => "shared_secret",
        }
    }

    pub fn validate(&self, field: &str) -> Result<(), NetError> {
        match self {
            Self::Http(cfg) => cfg.validate(field),
            Self::Cloudflare(cfg) => cfg.validate(field),
            Self::SharedSecret(cfg) => cfg.validate(field),
        }
    }

    /// Hosts this provider's ICE URLs legitimately point at.
    pub fn trusted_hosts(&self) -> Vec<String> {
        match self {
            Self::Http(_) => Vec::new(),
            Self::Cloudflare(_) => vec![CLOUDFLARE_TURN_HOST.to_string()],
            Self::SharedSecret(cfg) => cfg.trusted_hosts(),
        }
    }

    /// Obtain fresh credentials valid for about `ttl_secs`, sanitized.
    pub async fn issue(&self, ttl_secs: u64) -> Result<TurnCredentials, String> {
        let mut credentials = match self {
            Self::Http(cfg) => cfg.issue().await?,
            Self::Cloudflare(cfg) => cfg.issue(ttl_secs).await?,
            Self::SharedSecret(cfg) => cfg.issue(ttl_secs)?,
        };
        credentials.sanitize();
        if credentials.ice_servers.is_empty() {
            return Err(format!(
                "{} provider returned no usable ICE servers",
                self.name()
            ));
        }
        Ok(credentials)
    }
}
