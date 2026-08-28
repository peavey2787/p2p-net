//! Native Android bridge for the shared `p2p-net` Rust core.
//!
//! This crate is the only Rust layer allowed to contain C-ABI pointer handling.
//! The shared core remains `#![forbid(unsafe_code)]`. JNI conversion itself is
//! kept in the Android C++ shim so Java/Kotlin concerns never enter `crates/`.

#![deny(unsafe_op_in_unsafe_fn)]

use std::collections::{HashMap, VecDeque};
use std::ffi::{c_char, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::ptr;
use std::slice;
use std::str;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use p2p_net::{
    node_config_from_json, snapshot_to_json, start_node_with_platform, AndroidPlatformRuntime,
    AppMessage, Multiaddr, NodeConfig, NodeHandle, NodeProfile, NodeStorage, PeerId,
    PlatformRuntime, MAX_APP_MESSAGE_BYTES, MAX_APP_TOPIC_LEN,
};
use serde_json::{json, Value};
use tokio::runtime::{Builder, Handle, Runtime};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

const RUNTIME_WORKER_THREADS: usize = 2;
const RUNTIME_MAX_BLOCKING_THREADS: usize = 4;
const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const MESSAGE_QUEUE_CAPACITY: usize = 128;
const MESSAGE_QUEUE_MAX_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;
const MAX_SUBSCRIPTIONS: usize = 64;
const MAX_DRAIN_MESSAGES: usize = 64;
const MAX_DRAIN_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;
const MAX_CONFIG_JSON_BYTES: usize = 256 * 1024;
const MAX_DATA_DIR_BYTES: usize = 4 * 1024;
const MAX_MULTIADDR_BYTES: usize = 4 * 1024;
const MAX_PEER_ID_BYTES: usize = 256;
const MAX_RESPONSE_JSON_BYTES: usize = 4 * 1024 * 1024;
const MAX_BRIDGE_PEERS: usize = 512;

static CONTROLLER: OnceLock<Mutex<AndroidNodeController>> = OnceLock::new();

#[derive(Default)]
struct MessageQueueState {
    messages: VecDeque<AppMessage>,
    queued_payload_bytes: usize,
    dropped_messages: u64,
}

struct AndroidNodeController {
    runtime: Option<Runtime>,
    node: Option<NodeHandle>,
    subscriptions: HashMap<String, JoinHandle<()>>,
    messages: Arc<Mutex<MessageQueueState>>,
}

impl AndroidNodeController {
    fn new() -> Self {
        Self {
            runtime: None,
            node: None,
            subscriptions: HashMap::new(),
            messages: Arc::new(Mutex::new(MessageQueueState {
                messages: VecDeque::with_capacity(MESSAGE_QUEUE_CAPACITY),
                queued_payload_bytes: 0,
                dropped_messages: 0,
            })),
        }
    }

    fn start(&mut self, config_json: &str, data_dir: &str) -> Result<Value, String> {
        if self.node.is_some() {
            return Err("Android node is already running".to_string());
        }
        if data_dir.trim().is_empty() {
            return Err("Android app-private data directory is required".to_string());
        }

        let cfg = parse_and_validate_config(config_json)?;
        let tokio_runtime = build_runtime()?;
        let adapter = Arc::new(AndroidPlatformRuntime::foreground_service(PathBuf::from(
            data_dir,
        )));
        let platform: Arc<dyn PlatformRuntime> = adapter.clone();
        let storage: Arc<dyn NodeStorage> = adapter;
        let handle = tokio_runtime
            .block_on(start_node_with_platform(cfg, platform, storage))
            .map_err(|err| err.to_string())?;
        let peer_id = handle.peer_id.to_string();
        self.runtime = Some(tokio_runtime);
        self.node = Some(handle);
        Ok(json!({"peer_id": peer_id}))
    }

    fn stop(&mut self) -> Result<Value, String> {
        let subscription_tasks = self
            .subscriptions
            .drain()
            .map(|(_, task)| task)
            .collect::<Vec<_>>();
        for task in &subscription_tasks {
            task.abort();
        }
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.block_on(async {
                for task in subscription_tasks {
                    let _ = task.await;
                }
            });
        }
        if let Some(handle) = self.node.take() {
            if let Some(runtime) = self.runtime.as_ref() {
                runtime.block_on(handle.shutdown());
            }
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
        }
        let mut messages = self
            .messages
            .lock()
            .map_err(|_| "Android message queue mutex poisoned".to_string())?;
        messages.messages.clear();
        messages.queued_payload_bytes = 0;
        messages.dropped_messages = 0;
        Ok(json!({}))
    }

    fn revision(&self) -> u64 {
        self.node
            .as_ref()
            .map(NodeHandle::snapshot_revision)
            .unwrap_or(0)
    }

    fn subscribe(&mut self, topic: &str) -> Result<Value, String> {
        if self.subscriptions.contains_key(topic) {
            return Ok(json!({"topic": topic}));
        }
        if self.subscriptions.len() >= MAX_SUBSCRIPTIONS {
            return Err(format!(
                "Android subscription limit of {MAX_SUBSCRIPTIONS} reached"
            ));
        }
        let handle = self.running_node()?.clone();
        let runtime = self.running_runtime_handle()?;
        let mut subscription = runtime
            .block_on(handle.subscribe(topic.to_string()))
            .map_err(|err| err.to_string())?;
        let messages = Arc::clone(&self.messages);
        let topic_key = topic.to_string();
        let task = runtime.spawn(async move {
            loop {
                match subscription.recv().await {
                    Ok(message) => push_bounded_message(&messages, message),
                    Err(RecvError::Lagged(skipped)) => record_dropped_messages(&messages, skipped),
                    Err(RecvError::Closed) => break,
                }
            }
        });
        self.subscriptions.insert(topic_key, task);
        Ok(json!({"topic": topic}))
    }

    fn pending_message_count(&self) -> usize {
        self.messages
            .lock()
            .map(|queue| queue.messages.len())
            .unwrap_or(0)
    }

    fn drain_messages(&self, requested: usize) -> Result<Value, String> {
        let count = requested.min(MAX_DRAIN_MESSAGES);
        let mut queue = self
            .messages
            .lock()
            .map_err(|_| "Android message queue mutex poisoned".to_string())?;
        let mut drained_payload_bytes = 0usize;
        let mut messages = Vec::with_capacity(count);
        while messages.len() < count {
            let Some(next) = queue.messages.front() else {
                break;
            };
            let next_bytes = next.payload.len();
            if !messages.is_empty()
                && drained_payload_bytes.saturating_add(next_bytes) > MAX_DRAIN_PAYLOAD_BYTES
            {
                break;
            }
            let Some(message) = queue.messages.pop_front() else {
                break;
            };
            queue.queued_payload_bytes = queue.queued_payload_bytes.saturating_sub(next_bytes);
            drained_payload_bytes = drained_payload_bytes.saturating_add(next_bytes);
            messages.push(json!({
                "topic": message.topic,
                "source_peer_id": message.source_peer_id,
                "target_peer_id": message.target_peer_id,
                "timestamp_ns": message.timestamp_ns,
                "payload_len": message.payload.len(),
                "payload_base64": BASE64_STANDARD.encode(message.payload),
            }));
        }
        Ok(json!({"messages": messages}))
    }

    fn bridge_stats(&self) -> Result<Value, String> {
        let queue = self
            .messages
            .lock()
            .map_err(|_| "Android message queue mutex poisoned".to_string())?;
        Ok(json!({
            "runtime_worker_threads": RUNTIME_WORKER_THREADS,
            "runtime_max_blocking_threads": RUNTIME_MAX_BLOCKING_THREADS,
            "runtime_shutdown_timeout_ms": RUNTIME_SHUTDOWN_TIMEOUT.as_millis() as u64,
            "message_queue_capacity": MESSAGE_QUEUE_CAPACITY,
            "message_queue_max_payload_bytes": MESSAGE_QUEUE_MAX_PAYLOAD_BYTES,
            "pending_messages": queue.messages.len(),
            "queued_payload_bytes": queue.queued_payload_bytes,
            "dropped_messages": queue.dropped_messages,
            "subscriptions": self.subscriptions.len(),
            "max_subscriptions": MAX_SUBSCRIPTIONS,
            "max_response_json_bytes": MAX_RESPONSE_JSON_BYTES,
            "max_bridge_peers": MAX_BRIDGE_PEERS,
        }))
    }

    fn running_node(&self) -> Result<&NodeHandle, String> {
        self.node
            .as_ref()
            .ok_or_else(|| "Android node is not running".to_string())
    }

    fn running_runtime_handle(&self) -> Result<Handle, String> {
        self.runtime
            .as_ref()
            .map(|runtime| runtime.handle().clone())
            .ok_or_else(|| "Android runtime is not running".to_string())
    }
}

fn build_runtime() -> Result<Runtime, String> {
    Builder::new_multi_thread()
        .worker_threads(RUNTIME_WORKER_THREADS)
        .max_blocking_threads(RUNTIME_MAX_BLOCKING_THREADS)
        .thread_keep_alive(Duration::from_secs(10))
        .thread_name("p2p-android")
        .enable_all()
        .build()
        .map_err(|err| format!("failed to create Android Tokio runtime: {err}"))
}

fn full_node_default_config() -> NodeConfig {
    NodeConfig {
        profile: NodeProfile::Full,
        ..NodeConfig::default()
    }
}

fn parse_and_validate_config(config_json: &str) -> Result<NodeConfig, String> {
    let cfg = if config_json.trim().is_empty() {
        full_node_default_config()
    } else {
        node_config_from_json(config_json).map_err(|err| err.to_string())?
    };
    cfg.validate().map_err(|err| err.to_string())?;
    Ok(cfg)
}

fn push_bounded_message(queue: &Mutex<MessageQueueState>, message: AppMessage) {
    if let Ok(mut queue) = queue.lock() {
        let incoming_bytes = message.payload.len();
        if incoming_bytes > MESSAGE_QUEUE_MAX_PAYLOAD_BYTES {
            queue.dropped_messages = queue.dropped_messages.saturating_add(1);
            return;
        }
        while queue.messages.len() >= MESSAGE_QUEUE_CAPACITY
            || queue.queued_payload_bytes.saturating_add(incoming_bytes)
                > MESSAGE_QUEUE_MAX_PAYLOAD_BYTES
        {
            let Some(dropped) = queue.messages.pop_front() else {
                break;
            };
            queue.queued_payload_bytes = queue
                .queued_payload_bytes
                .saturating_sub(dropped.payload.len());
            queue.dropped_messages = queue.dropped_messages.saturating_add(1);
        }
        queue.queued_payload_bytes = queue.queued_payload_bytes.saturating_add(incoming_bytes);
        queue.messages.push_back(message);
    }
}

fn record_dropped_messages(queue: &Mutex<MessageQueueState>, skipped: u64) {
    if let Ok(mut queue) = queue.lock() {
        queue.dropped_messages = queue.dropped_messages.saturating_add(skipped);
    }
}

fn with_controller<T>(
    operation: impl FnOnce(&mut AndroidNodeController) -> Result<T, String>,
) -> Result<T, String> {
    let controller = CONTROLLER.get_or_init(|| Mutex::new(AndroidNodeController::new()));
    let mut controller = controller
        .lock()
        .map_err(|_| "Android node controller mutex poisoned".to_string())?;
    operation(&mut controller)
}

fn running_node_parts() -> Result<(Handle, NodeHandle), String> {
    with_controller(|controller| {
        Ok((
            controller.running_runtime_handle()?,
            controller.running_node()?.clone(),
        ))
    })
}

fn snapshot_value() -> Result<Value, String> {
    let (runtime, handle) = running_node_parts()?;
    let revision = handle.snapshot_revision();
    let snapshot = runtime.block_on(async { handle.snapshot.lock().await.clone() });
    Ok(json!({
        "revision": revision,
        "snapshot": snapshot_to_json(&snapshot),
    }))
}

fn peers_value() -> Result<Value, String> {
    let (runtime, handle) = running_node_parts()?;
    let mut peers = runtime
        .block_on(handle.get_peers())
        .map_err(|err| err.to_string())?;
    peers.truncate(MAX_BRIDGE_PEERS);
    Ok(json!({"peers": peers}))
}

fn metrics_value() -> Result<Value, String> {
    let (runtime, handle) = running_node_parts()?;
    let metrics = runtime
        .block_on(handle.get_metrics(None))
        .map_err(|err| err.to_string())?;
    Ok(json!({
        "uptime_seconds": metrics.uptime_seconds,
        "total_bytes_sent": metrics.bandwidth.total_bytes_sent,
        "total_bytes_received": metrics.bandwidth.total_bytes_received,
        "total_chunks_stored": metrics.storage.total_chunks_stored,
        "total_bytes_stored": metrics.storage.total_bytes_stored,
        "execution_cycles_estimated": metrics.compute.execution_cycles_estimated,
        "active_request_count": metrics.compute.active_request_count,
        "choked_peers_count": metrics.compute.choked_peers_count,
    }))
}

fn connect_value(addr: &str) -> Result<Value, String> {
    let (runtime, handle) = running_node_parts()?;
    let addr = addr
        .parse::<Multiaddr>()
        .map_err(|err| format!("invalid peer multiaddr: {err}"))?;
    runtime
        .block_on(handle.connect_peer(addr))
        .map_err(|err| err.to_string())?;
    Ok(json!({}))
}

fn disconnect_value(peer_id: &str) -> Result<Value, String> {
    let (runtime, handle) = running_node_parts()?;
    let peer_id = peer_id
        .parse::<PeerId>()
        .map_err(|err| format!("invalid peer id: {err}"))?;
    runtime
        .block_on(handle.disconnect_peer(peer_id))
        .map_err(|err| err.to_string())?;
    Ok(json!({}))
}

fn broadcast_value(topic: &str, payload: Vec<u8>) -> Result<Value, String> {
    let (runtime, handle) = running_node_parts()?;
    runtime
        .block_on(handle.broadcast(topic.to_string(), payload))
        .map_err(|err| err.to_string())?;
    Ok(json!({}))
}

fn send_value(peer_id: &str, topic: &str, payload: Vec<u8>) -> Result<Value, String> {
    let (runtime, handle) = running_node_parts()?;
    let peer_id = peer_id
        .parse::<PeerId>()
        .map_err(|err| format!("invalid peer id: {err}"))?;
    runtime
        .block_on(handle.send_message(peer_id, topic.to_string(), payload))
        .map_err(|err| err.to_string())?;
    Ok(json!({}))
}

fn response(value: Result<Value, String>) -> *mut c_char {
    let value = match value {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(error) => json!({"ok": false, "error": error}),
    };
    let serialized = value.to_string();
    if serialized.len() > MAX_RESPONSE_JSON_BYTES {
        return string_to_raw(
            json!({
                "ok": false,
                "error": format!(
                    "native response exceeds {MAX_RESPONSE_JSON_BYTES}-byte limit"
                ),
            })
            .to_string(),
        );
    }
    string_to_raw(serialized)
}

fn guarded_response(operation: impl FnOnce() -> Result<Value, String>) -> *mut c_char {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(result) => response(result),
        Err(_) => response(Err("native bridge panicked".to_string())),
    }
}

fn string_to_raw(value: String) -> *mut c_char {
    let sanitized = value.replace('\0', "\\u0000");
    CString::new(sanitized)
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

unsafe fn text_from_raw(
    ptr: *const u8,
    len: usize,
    name: &str,
    max_len: usize,
) -> Result<String, String> {
    if len > max_len {
        return Err(format!("{name} exceeds {max_len}-byte limit"));
    }
    if len == 0 {
        return Ok(String::new());
    }
    if ptr.is_null() {
        return Err(format!("non-empty {name} pointer must not be null"));
    }
    // SAFETY: the JNI shim passes a buffer containing at least `len` bytes and
    // retains ownership until this function returns; this function copies it.
    let bytes = unsafe { slice::from_raw_parts(ptr, len) };
    let value = str::from_utf8(bytes).map_err(|err| format!("{name} is not valid UTF-8: {err}"))?;
    if value.contains('\0') {
        return Err(format!("{name} must not contain NUL characters"));
    }
    Ok(value.to_owned())
}

unsafe fn payload_from_raw(ptr: *const u8, len: usize) -> Result<Vec<u8>, String> {
    if len > MAX_APP_MESSAGE_BYTES {
        return Err(format!(
            "payload exceeds {MAX_APP_MESSAGE_BYTES}-byte application-message limit"
        ));
    }
    if len == 0 {
        return Ok(Vec::new());
    }
    if ptr.is_null() {
        return Err("non-empty payload pointer must not be null".to_string());
    }
    // SAFETY: the JNI shim passes a buffer containing at least `len` bytes and
    // retains ownership until this function returns; we immediately copy it.
    Ok(unsafe { slice::from_raw_parts(ptr, len) }.to_vec())
}

#[no_mangle]
pub extern "C" fn p2p_android_default_config_json() -> *mut c_char {
    guarded_response(|| {
        let config = full_node_default_config()
            .to_pretty_json()
            .map_err(|err| err.to_string())?;
        Ok(json!({"config": config}))
    })
}

/// Validate an Android node configuration supplied as UTF-8 JSON.
///
/// # Safety
///
/// If `config_len` is non-zero, `config_json` must point to at least
/// `config_len` readable bytes that remain valid for the duration of this call.
/// A null pointer is permitted only when `config_len` is zero.
#[no_mangle]
pub unsafe extern "C" fn p2p_android_validate_config(
    config_json: *const u8,
    config_len: usize,
) -> *mut c_char {
    guarded_response(|| {
        // SAFETY: validated and copied immediately by `text_from_raw`.
        let config_json = unsafe {
            text_from_raw(
                config_json,
                config_len,
                "config_json",
                MAX_CONFIG_JSON_BYTES,
            )
        }?;
        let cfg = parse_and_validate_config(&config_json)?;
        Ok(json!({"profile": cfg.profile.as_str()}))
    })
}

/// Start the Android node using UTF-8 configuration and data-directory inputs.
///
/// # Safety
///
/// For each `(pointer, length)` pair, a non-zero length requires the pointer to
/// reference at least that many readable bytes that remain valid for the duration
/// of this call. A null pointer is permitted only for a zero-length input.
#[no_mangle]
pub unsafe extern "C" fn p2p_android_start(
    config_json: *const u8,
    config_len: usize,
    data_dir: *const u8,
    data_dir_len: usize,
) -> *mut c_char {
    guarded_response(|| {
        // SAFETY: validated and copied immediately by `text_from_raw`.
        let config_json = unsafe {
            text_from_raw(
                config_json,
                config_len,
                "config_json",
                MAX_CONFIG_JSON_BYTES,
            )
        }?;
        // SAFETY: validated and copied immediately by `text_from_raw`.
        let data_dir =
            unsafe { text_from_raw(data_dir, data_dir_len, "data_dir", MAX_DATA_DIR_BYTES) }?;
        with_controller(|controller| controller.start(&config_json, &data_dir))
    })
}

#[no_mangle]
pub extern "C" fn p2p_android_stop() -> *mut c_char {
    guarded_response(|| with_controller(AndroidNodeController::stop))
}

#[no_mangle]
pub extern "C" fn p2p_android_revision() -> u64 {
    catch_unwind(AssertUnwindSafe(|| {
        with_controller(|controller| Ok(controller.revision())).unwrap_or(0)
    }))
    .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn p2p_android_snapshot_json() -> *mut c_char {
    guarded_response(snapshot_value)
}

#[no_mangle]
pub extern "C" fn p2p_android_peers_json() -> *mut c_char {
    guarded_response(peers_value)
}

#[no_mangle]
pub extern "C" fn p2p_android_metrics_json() -> *mut c_char {
    guarded_response(metrics_value)
}

#[no_mangle]
pub extern "C" fn p2p_android_bridge_stats_json() -> *mut c_char {
    guarded_response(|| with_controller(|controller| controller.bridge_stats()))
}

/// Connect to the UTF-8 multiaddress supplied by the caller.
///
/// # Safety
///
/// If `addr_len` is non-zero, `addr` must point to at least `addr_len` readable
/// bytes that remain valid for the duration of this call. A null pointer is
/// permitted only when `addr_len` is zero.
#[no_mangle]
pub unsafe extern "C" fn p2p_android_connect(addr: *const u8, addr_len: usize) -> *mut c_char {
    guarded_response(|| {
        // SAFETY: validated and copied immediately by `text_from_raw`.
        let addr = unsafe { text_from_raw(addr, addr_len, "addr", MAX_MULTIADDR_BYTES) }?;
        connect_value(&addr)
    })
}

/// Disconnect the application peer identified by the supplied UTF-8 peer ID.
///
/// # Safety
///
/// If `peer_id_len` is non-zero, `peer_id` must point to at least `peer_id_len`
/// readable bytes that remain valid for the duration of this call. A null
/// pointer is permitted only when `peer_id_len` is zero.
#[no_mangle]
pub unsafe extern "C" fn p2p_android_disconnect(
    peer_id: *const u8,
    peer_id_len: usize,
) -> *mut c_char {
    guarded_response(|| {
        // SAFETY: validated and copied immediately by `text_from_raw`.
        let peer_id = unsafe { text_from_raw(peer_id, peer_id_len, "peer_id", MAX_PEER_ID_BYTES) }?;
        disconnect_value(&peer_id)
    })
}

/// Broadcast a payload on the supplied UTF-8 application topic.
///
/// # Safety
///
/// For each `(pointer, length)` pair, a non-zero length requires the pointer to
/// reference at least that many readable bytes that remain valid for the duration
/// of this call. A null pointer is permitted only for a zero-length input.
#[no_mangle]
pub unsafe extern "C" fn p2p_android_broadcast(
    topic: *const u8,
    topic_len: usize,
    payload: *const u8,
    payload_len: usize,
) -> *mut c_char {
    guarded_response(|| {
        // SAFETY: validated and copied immediately by `text_from_raw`.
        let topic = unsafe { text_from_raw(topic, topic_len, "topic", MAX_APP_TOPIC_LEN) }?;
        // SAFETY: validated and copied immediately by `payload_from_raw`.
        let payload = unsafe { payload_from_raw(payload, payload_len) }?;
        broadcast_value(&topic, payload)
    })
}

/// Send a payload to a specific application peer and UTF-8 topic.
///
/// # Safety
///
/// For each `(pointer, length)` pair, a non-zero length requires the pointer to
/// reference at least that many readable bytes that remain valid for the duration
/// of this call. A null pointer is permitted only for a zero-length input.
#[no_mangle]
pub unsafe extern "C" fn p2p_android_send(
    peer_id: *const u8,
    peer_id_len: usize,
    topic: *const u8,
    topic_len: usize,
    payload: *const u8,
    payload_len: usize,
) -> *mut c_char {
    guarded_response(|| {
        // SAFETY: validated and copied immediately by `text_from_raw`.
        let peer_id = unsafe { text_from_raw(peer_id, peer_id_len, "peer_id", MAX_PEER_ID_BYTES) }?;
        // SAFETY: validated and copied immediately by `text_from_raw`.
        let topic = unsafe { text_from_raw(topic, topic_len, "topic", MAX_APP_TOPIC_LEN) }?;
        // SAFETY: validated and copied immediately by `payload_from_raw`.
        let payload = unsafe { payload_from_raw(payload, payload_len) }?;
        send_value(&peer_id, &topic, payload)
    })
}

/// Subscribe to the supplied UTF-8 application topic.
///
/// # Safety
///
/// If `topic_len` is non-zero, `topic` must point to at least `topic_len`
/// readable bytes that remain valid for the duration of this call. A null
/// pointer is permitted only when `topic_len` is zero.
#[no_mangle]
pub unsafe extern "C" fn p2p_android_subscribe(topic: *const u8, topic_len: usize) -> *mut c_char {
    guarded_response(|| {
        // SAFETY: validated and copied immediately by `text_from_raw`.
        let topic = unsafe { text_from_raw(topic, topic_len, "topic", MAX_APP_TOPIC_LEN) }?;
        with_controller(|controller| controller.subscribe(&topic))
    })
}

#[no_mangle]
pub extern "C" fn p2p_android_pending_message_count() -> u32 {
    catch_unwind(AssertUnwindSafe(|| {
        with_controller(|controller| Ok(controller.pending_message_count()))
            .ok()
            .and_then(|count| u32::try_from(count).ok())
            .unwrap_or(0)
    }))
    .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn p2p_android_drain_messages_json(max_messages: u32) -> *mut c_char {
    guarded_response(|| {
        with_controller(|controller| controller.drain_messages(max_messages as usize))
    })
}

/// Free a C string previously returned by this native bridge.
///
/// # Safety
///
/// `value` must either be null or be a pointer returned by this bridge from
/// `CString::into_raw`. Every non-null pointer may be passed to this function
/// exactly once and must not be used after this call returns.
#[no_mangle]
pub unsafe extern "C" fn p2p_android_string_free(value: *mut c_char) {
    if value.is_null() {
        return;
    }
    // SAFETY: callers may pass only pointers returned by `CString::into_raw`
    // from this bridge, exactly once.
    unsafe {
        drop(CString::from_raw(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_message(index: usize) -> AppMessage {
        AppMessage {
            schema_version: 2,
            network_id: 1,
            topic: "test/topic".to_string(),
            source_peer_id: format!("peer-{index}"),
            target_peer_id: None,
            timestamp_ns: index as u64,
            nonce_hex: format!("{index:064x}"),
            payload: vec![index as u8],
        }
    }

    #[test]
    fn message_queue_is_bounded_and_counts_drops() {
        let queue = Mutex::new(MessageQueueState::default());
        for index in 0..(MESSAGE_QUEUE_CAPACITY + 20) {
            push_bounded_message(&queue, test_message(index));
        }
        let queue = queue.lock().expect("queue lock");
        assert_eq!(queue.messages.len(), MESSAGE_QUEUE_CAPACITY);
        assert_eq!(queue.queued_payload_bytes, MESSAGE_QUEUE_CAPACITY);
        assert_eq!(
            queue
                .messages
                .front()
                .expect("oldest retained")
                .timestamp_ns,
            20
        );
        assert_eq!(queue.dropped_messages, 20);
    }

    #[test]
    fn default_android_config_is_full_profile() {
        assert_eq!(full_node_default_config().profile, NodeProfile::Full);
    }

    #[test]
    fn config_validation_rejects_invalid_json_before_restart() {
        assert!(parse_and_validate_config("{not-json").is_err());
        assert!(parse_and_validate_config("").is_ok());
    }

    #[test]
    fn payload_guard_rejects_oversized_input_before_dereference() {
        // SAFETY: oversized length is rejected before the null pointer is read.
        let result = unsafe { payload_from_raw(ptr::null(), MAX_APP_MESSAGE_BYTES + 1) };
        assert!(result.expect_err("oversized payload").contains("exceeds"));
    }

    #[test]
    fn text_guard_rejects_oversized_input_before_dereference() {
        // SAFETY: oversized length is rejected before the null pointer is read.
        let result = unsafe {
            text_from_raw(
                ptr::null(),
                MAX_CONFIG_JSON_BYTES + 1,
                "config_json",
                MAX_CONFIG_JSON_BYTES,
            )
        };
        assert!(result.expect_err("oversized text").contains("exceeds"));
    }
}
