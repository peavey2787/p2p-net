use wasm_bindgen::{JsCast, JsValue};

/// Errors that may happen on the [`Transport`](crate::Transport) or the
/// [`Connection`](crate::Connection).
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Invalid multiaddr: {0}")]
    InvalidMultiaddr(String),

    #[error("JavaScript error: {0}")]
    Js(String),

    #[error("JavaScript typecasting failed")]
    JsCastFailed,

    #[error("Unknown remote peer ID")]
    UnknownRemotePeerId,

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Authentication error")]
    Authentication(#[from] AuthenticationError),

    /// The ICE connectivity check of a browser-to-browser `/webrtc` upgrade
    /// did not produce a connected peer connection.
    #[error("ICE check {outcome}: ICE state {ice_state}")]
    IceCheck {
        outcome: IceCheckOutcome,
        /// The browser's final `iceConnectionState` (e.g. `failed`, `checking`).
        ice_state: String,
    },
}

/// How an unsuccessful ICE check ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceCheckOutcome {
    /// The browser reported the peer connection as failed.
    Failed,
    /// No connection within the configured establishment checks.
    TimedOut,
}

impl std::fmt::Display for IceCheckOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            IceCheckOutcome::Failed => "failed",
            IceCheckOutcome::TimedOut => "timeout",
        })
    }
}

/// New-type wrapper to hide `libp2p_noise` from the public API.
#[derive(thiserror::Error, Debug)]
#[error(transparent)]
pub struct AuthenticationError(pub(crate) libp2p_webrtc_utils::noise::Error);

impl Error {
    pub(crate) fn from_js_value(value: JsValue) -> Self {
        let s = if value.is_instance_of::<js_sys::Error>() {
            js_sys::Error::from(value)
                .to_string()
                .as_string()
                .unwrap_or_else(|| "Unknown error".to_string())
        } else {
            "Unknown error".to_string()
        };

        Error::Js(s)
    }
}

impl From<JsValue> for Error {
    fn from(value: JsValue) -> Self {
        Error::from_js_value(value)
    }
}

impl From<String> for Error {
    fn from(value: String) -> Self {
        Error::Js(value)
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Error::Js(value.to_string())
    }
}
