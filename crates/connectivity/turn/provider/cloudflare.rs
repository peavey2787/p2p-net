//! Cloudflare Realtime TURN.
//!
//! `POST {api_base}/v1/turn/keys/{key_id}/credentials/generate-ice-servers`
//! with `Authorization: Bearer <api token>` and `{"ttl": seconds}` returns
//! `{"iceServers": [...]}`. The API token is long-lived, so this provider
//! runs only on servers: a p2p-net broker node, or the Worker template in
//! `providers/turn/cloudflare` that browsers reach through the HTTP provider.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::common::error::{config_error, NetError};
use crate::common::http::{fetch_json, JsonRequest};

use super::super::credentials::{TurnCredentials, MAX_CREDENTIAL_TTL_SECS};
use super::super::secret::SecretRef;
use super::http::{validate_endpoint_url, validate_timeout};

pub const CLOUDFLARE_TURN_HOST: &str = "turn.cloudflare.com";
const DEFAULT_API_BASE: &str = "https://rtc.live.cloudflare.com";
/// Browsers block port 53; Cloudflare recommends dropping those URLs.
const BROWSER_BLOCKED_PORT: u16 = 53;
const DEFAULT_TIMEOUT_SECS: u64 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudflareProviderConfig {
    /// TURN key ID from the Cloudflare dashboard (Realtime > TURN).
    pub key_id: String,
    /// The TURN key's API token. Use `{"env": "..."}` or `{"file": "..."}`.
    pub api_token: SecretRef,
    /// Override for tests or an egress proxy.
    #[serde(default = "default_api_base")]
    pub api_base: String,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_api_base() -> String {
    DEFAULT_API_BASE.to_string()
}

fn default_timeout_secs() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

impl CloudflareProviderConfig {
    pub fn new(key_id: impl Into<String>, api_token: SecretRef) -> Self {
        Self {
            key_id: key_id.into(),
            api_token,
            api_base: default_api_base(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }

    pub(super) fn validate(&self, field: &str) -> Result<(), NetError> {
        if self.key_id.is_empty()
            || !self
                .key_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(config_error(format!(
                "{field}.key_id must be the Cloudflare TURN key ID (letters, digits, '-')"
            )));
        }
        self.api_token.validate(&format!("{field}.api_token"))?;
        validate_endpoint_url(&format!("{field}.api_base"), &self.api_base)?;
        validate_timeout(field, self.timeout_secs)
    }

    pub(super) async fn issue(&self, ttl_secs: u64) -> Result<TurnCredentials, String> {
        let ttl = ttl_secs.min(MAX_CREDENTIAL_TTL_SECS);
        let request = JsonRequest::post(
            self.endpoint(),
            serde_json::json!({ "ttl": ttl }),
            Duration::from_secs(self.timeout_secs),
        )
        .header(
            "authorization",
            format!("Bearer {}", self.api_token.resolve()?),
        );
        let mut credentials: TurnCredentials = fetch_json(request).await?;
        credentials.ice_servers = credentials
            .ice_servers
            .into_iter()
            .filter_map(|server| server.without_port(BROWSER_BLOCKED_PORT))
            .collect();
        credentials.ttl.get_or_insert(ttl);
        Ok(credentials)
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/v1/turn/keys/{}/credentials/generate-ice-servers",
            self.api_base.trim_end_matches('/'),
            self.key_id
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> CloudflareProviderConfig {
        CloudflareProviderConfig::new(
            "0123456789abcdef",
            SecretRef::Env("CF_TURN_TOKEN".to_string()),
        )
    }

    #[test]
    fn builds_generate_ice_servers_endpoint() {
        assert_eq!(
            config().endpoint(),
            "https://rtc.live.cloudflare.com/v1/turn/keys/0123456789abcdef/credentials/generate-ice-servers"
        );
    }

    #[test]
    fn validates_key_id_and_api_base() {
        assert!(config().validate("cf").is_ok());
        let mut bad = config();
        bad.key_id = "../escape".to_string();
        assert!(bad.validate("cf").is_err());
        let mut insecure = config();
        insecure.api_base = "http://rtc.live.cloudflare.com".to_string();
        assert!(insecure.validate("cf").is_err());
    }

    #[test]
    fn json_uses_type_tag_and_defaults() {
        let provider: super::super::TurnProviderConfig = serde_json::from_str(
            r#"{"type":"cloudflare","key_id":"abc","api_token":{"env":"CF_TURN_TOKEN"}}"#,
        )
        .expect("cloudflare provider json");
        let super::super::TurnProviderConfig::Cloudflare(cfg) = provider else {
            panic!("expected cloudflare provider");
        };
        assert_eq!(cfg.api_base, DEFAULT_API_BASE);
        assert_eq!(cfg.timeout_secs, DEFAULT_TIMEOUT_SECS);
    }
}
