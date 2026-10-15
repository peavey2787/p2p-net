//! References to secrets (API tokens, TURN shared secrets) that keep the
//! secret itself out of config files and logs.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::common::error::{config_error, NetError};

/// Where a secret comes from. Prefer `env` or `file`; `value` embeds the
/// secret in the config and is meant for local testing.
///
/// JSON: `{"env": "CLOUDFLARE_TURN_API_TOKEN"}`, `{"file": "/run/secrets/turn"}`
/// or `{"value": "..."}`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretRef {
    Env(String),
    File(String),
    Value(String),
}

impl fmt::Debug for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Env(name) => f.debug_tuple("Env").field(name).finish(),
            Self::File(path) => f.debug_tuple("File").field(path).finish(),
            Self::Value(_) => f.write_str("Value(<redacted>)"),
        }
    }
}

impl SecretRef {
    pub fn validate(&self, field: &str) -> Result<(), NetError> {
        let empty = match self {
            Self::Env(raw) | Self::File(raw) | Self::Value(raw) => raw.trim().is_empty(),
        };
        if empty {
            return Err(config_error(format!("{field} must not be empty")));
        }
        Ok(())
    }

    /// Read the secret. Environment variables and files are native-only.
    pub fn resolve(&self) -> Result<String, String> {
        let secret = match self {
            Self::Value(value) => value.clone(),
            Self::Env(name) => read_env(name)?,
            Self::File(path) => read_file(path)?,
        };
        let secret = secret.trim().to_string();
        if secret.is_empty() {
            return Err("secret is empty".to_string());
        }
        Ok(secret)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("environment variable {name} is not set"))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_file(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|err| format!("cannot read secret file {path}: {err}"))
}

#[cfg(target_arch = "wasm32")]
fn read_env(name: &str) -> Result<String, String> {
    Err(format!(
        "environment variable {name} is unavailable in the browser"
    ))
}

#[cfg(target_arch = "wasm32")]
fn read_file(path: &str) -> Result<String, String> {
    Err(format!("secret file {path} is unavailable in the browser"))
}

#[cfg(test)]
mod tests {
    use super::SecretRef;

    #[test]
    fn json_shape_is_a_single_keyed_object() {
        let secret: SecretRef = serde_json::from_str(r#"{"env":"TURN_TOKEN"}"#).expect("env ref");
        assert_eq!(secret, SecretRef::Env("TURN_TOKEN".to_string()));
    }

    #[test]
    fn debug_redacts_inline_values() {
        let rendered = format!("{:?}", SecretRef::Value("hunter2".to_string()));
        assert!(!rendered.contains("hunter2"));
    }

    #[test]
    fn resolve_trims_and_rejects_empty_values() {
        assert_eq!(
            SecretRef::Value(" s3cret\n".to_string()).resolve(),
            Ok("s3cret".to_string())
        );
        assert!(SecretRef::Value("  ".to_string()).resolve().is_err());
        assert!(SecretRef::Env(String::new()).validate("secret").is_err());
    }

    #[test]
    fn missing_environment_variable_is_reported() {
        let err = SecretRef::Env("P2P_NET_TEST_SECRET_THAT_IS_NEVER_SET".to_string())
            .resolve()
            .expect_err("unset variable");
        assert!(err.contains("P2P_NET_TEST_SECRET_THAT_IS_NEVER_SET"));
    }
}
