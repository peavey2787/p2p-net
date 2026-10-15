//! TURN REST API shared-secret credentials (coturn `use-auth-secret`, and the
//! p2p-net built-in TURN server): `username = <expiry unix time>`,
//! `credential = base64(HMAC-SHA1(secret, username))`. Minting is local and
//! instant, but the secret must stay on the server.

use serde::{Deserialize, Serialize};

use crate::common::error::{config_error, NetError};

use super::super::credentials::TurnCredentials;
use super::super::secret::SecretRef;
use crate::connectivity::ice::IceServer;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedSecretProviderConfig {
    /// TURN server URLs, e.g. `["turn:turn.example.org:3478?transport=udp"]`.
    pub urls: Vec<String>,
    /// The server's `static-auth-secret`.
    pub secret: SecretRef,
}

impl SharedSecretProviderConfig {
    pub(super) fn validate(&self, field: &str) -> Result<(), NetError> {
        self.secret.validate(&format!("{field}.secret"))?;
        // Validate URL syntax with placeholder credentials; real ones are minted.
        let template = self.ice_server("u".to_string(), "c".to_string());
        template.validate(&format!("{field}.urls"))?;
        if !template.is_turn() {
            return Err(config_error(format!(
                "{field}.urls must include at least one turn: or turns: URL"
            )));
        }
        Ok(())
    }

    pub(super) fn trusted_hosts(&self) -> Vec<String> {
        self.ice_server(String::new(), String::new())
            .hosts()
            .collect()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn issue(&self, ttl_secs: u64) -> Result<TurnCredentials, String> {
        let secret = self.secret.resolve()?;
        let (username, credential) = turn::auth::generate_long_term_credentials(
            &secret,
            std::time::Duration::from_secs(ttl_secs),
        )
        .map_err(|err| format!("cannot mint TURN credentials: {err}"))?;
        Ok(TurnCredentials {
            ice_servers: vec![self.ice_server(username, credential)],
            ttl: Some(ttl_secs),
        })
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn issue(&self, _ttl_secs: u64) -> Result<TurnCredentials, String> {
        Err("shared_secret TURN credentials can only be minted on a server".to_string())
    }

    fn ice_server(&self, username: String, credential: String) -> IceServer {
        IceServer {
            urls: self.urls.clone(),
            username: Some(username),
            credential: Some(credential),
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn config() -> SharedSecretProviderConfig {
        SharedSecretProviderConfig {
            urls: vec!["turn:turn.example.org:3478?transport=udp".to_string()],
            secret: SecretRef::Value("shared".to_string()),
        }
    }

    #[test]
    fn mints_time_limited_credentials_for_the_configured_urls() {
        let credentials = config().issue(600).expect("minted");
        let server = &credentials.ice_servers[0];
        let expiry: u64 = server
            .username
            .as_deref()
            .expect("username")
            .parse()
            .expect("unix expiry username");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        assert!((now + 590..=now + 610).contains(&expiry));
        assert!(server.credential.as_deref().is_some_and(|c| !c.is_empty()));
        assert_eq!(credentials.ttl, Some(600));
    }

    #[test]
    fn requires_a_turn_url() {
        let mut stun_only = config();
        stun_only.urls = vec!["stun:turn.example.org".to_string()];
        assert!(stun_only.validate("p").is_err());
        assert_eq!(config().trusted_hosts(), vec!["turn.example.org"]);
    }
}
