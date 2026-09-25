//! Playwright-only QA exports (`browser-tests` feature). They drive the real
//! browser storage, profile lock, and node lifecycle; they are never part of a
//! production build.

use std::time::Duration;

use wasm_bindgen::prelude::*;

use super::{js_error, net_error, WasmNode};
use crate::platform::{BrowserNodeStorage, NodeStorage};
use crate::NodeConfig;

#[wasm_bindgen(js_name = qaStorageWrite)]
pub async fn qa_storage_write(namespace: String) -> Result<(), JsValue> {
    let storage = BrowserNodeStorage::open(namespace)
        .await
        .map_err(net_error)?;
    storage
        .write_secret("identity", b"stable-peer-key")
        .map_err(net_error)?;
    storage
        .write_public("peer-cache", b"cached-peer")
        .map_err(net_error)?;
    storage.flush().await.map_err(net_error)
}

#[wasm_bindgen(js_name = qaStorageRead)]
pub async fn qa_storage_read(namespace: String) -> Result<(), JsValue> {
    let storage = BrowserNodeStorage::open(namespace)
        .await
        .map_err(net_error)?;
    let identity = storage.read_secret("identity").map_err(net_error)?;
    if identity.as_deref() != Some(b"stable-peer-key".as_slice()) {
        return Err(js_error(
            "qa_storage",
            "IndexedDB identity journal did not survive page reload",
        ));
    }
    let peer_cache = storage.read("peer-cache").map_err(net_error)?;
    if peer_cache.as_deref() != Some(b"cached-peer".as_slice()) {
        return Err(js_error(
            "qa_storage",
            "IndexedDB public journal did not survive page reload",
        ));
    }
    Ok(())
}

const QA_PROFILE_STAGE_TIMEOUT: Duration = Duration::from_secs(20);

fn qa_profile_config() -> NodeConfig {
    let mut config = NodeConfig::default();
    config.discovery.public_bootstrap =
        crate::connectivity::public_fallback::PublicBootstrapConfig::private_infrastructure_only();
    config.discovery.dht.enabled = false;
    config.discovery.relay_discovery.enabled = false;
    config.discovery.lan.enabled = false;
    config.public_ip_probe.enabled = false;
    config.dnsaddr.enabled = false;
    config
}

async fn qa_start_node(
    config: JsValue,
    namespace: String,
    stage: &str,
) -> Result<WasmNode, JsValue> {
    crate::runtime::timeout(QA_PROFILE_STAGE_TIMEOUT, WasmNode::start(config, namespace))
        .await
        .map_err(|_| {
            js_error(
                "qa_profile_lifecycle",
                &format!("{stage} timed out after 20s"),
            )
        })?
}

async fn qa_shutdown_node(node: &mut WasmNode, stage: &str) -> Result<(), JsValue> {
    crate::runtime::timeout(QA_PROFILE_STAGE_TIMEOUT, node.shutdown())
        .await
        .map_err(|_| {
            js_error(
                "qa_profile_lifecycle",
                &format!("{stage} timed out after 20s"),
            )
        })?
}

#[wasm_bindgen(js_name = qaProfileLifecycle)]
pub async fn qa_profile_lifecycle(namespace: String) -> Result<String, JsValue> {
    let config = serde_wasm_bindgen::to_value(&qa_profile_config())
        .map_err(|e| js_error("serialize", &e.to_string()))?;

    let mut first = qa_start_node(config.clone(), namespace.clone(), "initial node start").await?;
    let peer_id = first.peer_id();

    match crate::runtime::timeout(
        QA_PROFILE_STAGE_TIMEOUT,
        WasmNode::start(config.clone(), namespace.clone()),
    )
    .await
    {
        Err(()) => {
            let _ = qa_shutdown_node(&mut first, "cleanup after duplicate lock timeout").await;
            return Err(js_error(
                "qa_profile_lifecycle",
                "duplicate profile-lock check timed out after 20s",
            ));
        }
        Ok(Ok(mut duplicate)) => {
            let _ = qa_shutdown_node(&mut duplicate, "duplicate node cleanup").await;
            let _ = qa_shutdown_node(&mut first, "first node cleanup").await;
            return Err(js_error(
                "qa_profile_lock",
                "duplicate browser profile owner was accepted",
            ));
        }
        Ok(Err(_)) => {}
    }

    qa_shutdown_node(&mut first, "initial node shutdown").await?;

    let mut reopened = qa_start_node(config, namespace, "node reopen").await?;
    let reopened_peer_id = reopened.peer_id();
    qa_shutdown_node(&mut reopened, "reopened node shutdown").await?;

    if reopened_peer_id != peer_id {
        return Err(js_error(
            "qa_identity",
            "PeerId changed after browser profile reopen",
        ));
    }

    Ok(peer_id)
}
