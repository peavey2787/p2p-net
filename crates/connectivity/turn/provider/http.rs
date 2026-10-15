//! Generic HTTPS credential endpoint: `GET url` returns the
//! [`TurnCredentials`] JSON contract. Safe for browsers because the endpoint,
//! not the client, holds the provider's long-lived secret.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::common::error::{config_error, NetError};
use crate::common::http::{fetch_json, JsonRequest};

use super::super::credentials::TurnCredentials;
use super::super::secret::SecretRef;

const DEFAULT_TIMEOUT_SECS: u64 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpProviderConfig {
    /// `https://` endpoint (`http://localhost` is allowed for development).
    pub url: String,
    /// Optional `Authorization: Bearer` token for private endpoints. Do not
    /// use one in browser configs: anything shipped to a browser is public.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer_token: Option<SecretRef>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_timeout_secs() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

impl HttpProviderConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            bearer_token: None,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }

    pub(crate) fn validate(&self, field: &str) -> Result<(), NetError> {
        validate_endpoint_url(&format!("{field}.url"), &self.url)?;
        if let Some(token) = &self.bearer_token {
            token.validate(&format!("{field}.bearer_token"))?;
        }
        validate_timeout(field, self.timeout_secs)
    }

    pub(super) async fn issue(&self) -> Result<TurnCredentials, String> {
        let mut request =
            JsonRequest::get(self.url.clone(), Duration::from_secs(self.timeout_secs));
        if let Some(token) = &self.bearer_token {
            request = request.header("authorization", format!("Bearer {}", token.resolve()?));
        }
        fetch_json(request).await
    }
}

/// HTTPS, or plain HTTP to a loopback development host.
pub(super) fn validate_endpoint_url(field: &str, raw: &str) -> Result<(), NetError> {
    let url = url::Url::parse(raw)
        .map_err(|err| config_error(format!("{field} `{raw}` is not a URL: {err}")))?;
    let local_dev = url.scheme() == "http"
        && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() != "https" && !local_dev {
        return Err(config_error(format!(
            "{field} must use https (http is allowed only for localhost)"
        )));
    }
    Ok(())
}

pub(super) fn validate_timeout(field: &str, timeout_secs: u64) -> Result<(), NetError> {
    if !(1..=30).contains(&timeout_secs) {
        return Err(config_error(format!(
            "{field}.timeout_secs must be between 1 and 30"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_https_except_for_localhost() {
        assert!(
            HttpProviderConfig::new("https://turn.example.workers.dev/ice-servers")
                .validate("p")
                .is_ok()
        );
        assert!(HttpProviderConfig::new("http://localhost:8787/ice-servers")
            .validate("p")
            .is_ok());
        assert!(HttpProviderConfig::new("http://turn.example.org/ice")
            .validate("p")
            .is_err());
        assert!(HttpProviderConfig::new("not a url").validate("p").is_err());
    }

    #[test]
    fn rejects_out_of_range_timeouts() {
        let mut cfg = HttpProviderConfig::new("https://example.org");
        cfg.timeout_secs = 0;
        assert!(cfg.validate("p").is_err());
    }
}
