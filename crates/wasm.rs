//! First-class browser facade. The networking implementation remains the same
//! `NodeHandle`; this module only converts browser/JS types and lifecycle events.

use std::sync::Arc;

use futures::future::{AbortHandle, Abortable};
use js_sys::{Function, Object, Reflect, Uint8Array};
use std::time::Duration;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::Event;

use crate::api::{AppSubscription, NodeEvent, NodeEventSubscription};
use crate::connectivity::identity::load_or_create_identity_key_with_storage;
use crate::node::start_node_with_platform;
use crate::platform::{BrowserNodeStorage, BrowserPlatformRuntime, NodeStorage, PlatformRuntime};
use crate::{NodeConfig, NodeHandle};

#[wasm_bindgen(inline_js = r#"
export async function p2pNetAcquireProfileLock(name) {
  const lockName = `p2p-net:${name}`;
  if (navigator.locks && navigator.locks.request) {
    return await new Promise((resolve) => {
      let release;
      const held = new Promise((r) => { release = r; });
      let request;
      request = navigator.locks.request(lockName, { mode: 'exclusive', ifAvailable: true }, (lock) => {
        if (!lock) { resolve(null); return; }
        resolve({ release: () => { release(); return request; } });
        return held;
      });
      request.catch(() => resolve(null));
    });
  }

  // IndexedDB lease fallback. The read/write decision occurs in one exclusive
  // transaction, so simultaneous tabs cannot both acquire the same profile.
  const owner = `${Date.now()}-${Math.random()}-${Math.random()}`;
  const db = await new Promise((resolve, reject) => {
    const request = indexedDB.open('p2p-net-profile-locks', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('leases');
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const acquire = await new Promise((resolve, reject) => {
    const tx = db.transaction('leases', 'readwrite');
    const store = tx.objectStore('leases');
    const get = store.get(lockName);
    get.onsuccess = () => {
      const now = Date.now();
      const current = get.result;
      if (current && current.expires > now) { resolve(false); return; }
      store.put({ owner, expires: now + 15000 }, lockName);
      tx.oncomplete = () => resolve(true);
    };
    get.onerror = () => reject(get.error);
    tx.onerror = () => reject(tx.error);
  });
  if (!acquire) { db.close(); return null; }

  const renew = setInterval(() => {
    try {
      const tx = db.transaction('leases', 'readwrite');
      const store = tx.objectStore('leases');
      const get = store.get(lockName);
      get.onsuccess = () => {
        if (get.result && get.result.owner === owner) {
          store.put({ owner, expires: Date.now() + 15000 }, lockName);
        }
      };
    } catch (_) {}
  }, 5000);
  return { release: () => new Promise((resolve, reject) => {
    clearInterval(renew);
    try {
      const tx = db.transaction('leases', 'readwrite');
      const store = tx.objectStore('leases');
      const get = store.get(lockName);
      get.onsuccess = () => { if (get.result && get.result.owner === owner) store.delete(lockName); };
      get.onerror = () => reject(get.error);
      tx.oncomplete = () => { db.close(); resolve(); };
      tx.onabort = () => { db.close(); reject(tx.error); };
      tx.onerror = () => { db.close(); reject(tx.error); };
    } catch (error) { db.close(); reject(error); }
  })};
}
"#)]
extern "C" {
    #[wasm_bindgen(catch, js_name = p2pNetAcquireProfileLock)]
    async fn acquire_profile_lock(name: &str) -> Result<JsValue, JsValue>;
}

struct ProfileLock {
    handle: Option<JsValue>,
}
impl ProfileLock {
    async fn release(&mut self) -> Result<(), JsValue> {
        let Some(handle) = self.handle.as_ref() else {
            return Ok(());
        };
        let value = Reflect::get(handle, &JsValue::from_str("release"))?;
        if let Some(function) = value.dyn_ref::<Function>() {
            let result = function.call0(handle)?;
            if let Ok(promise) = result.dyn_into::<js_sys::Promise>() {
                wasm_bindgen_futures::JsFuture::from(promise).await?;
            }
        }
        self.handle = None;
        Ok(())
    }
}
impl Drop for ProfileLock {
    fn drop(&mut self) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        if let Ok(value) = Reflect::get(&handle, &JsValue::from_str("release")) {
            if let Some(function) = value.dyn_ref::<Function>() {
                let _ = function.call0(&handle);
            }
        }
    }
}

#[cfg(feature = "browser-tests")]
mod qa;
#[cfg(feature = "browser-tests")]
pub use qa::{qa_profile_lifecycle, qa_storage_read, qa_storage_write};

/// A page-lifecycle listener registered by a node: target, event name, callback.
type LifecycleListener = (
    web_sys::EventTarget,
    &'static str,
    Closure<dyn FnMut(Event)>,
);

#[wasm_bindgen]
pub struct WasmNode {
    handle: NodeHandle,
    storage: Arc<BrowserNodeStorage>,
    profile_lock: Option<ProfileLock>,
    persistence_abort: AbortHandle,
    lifecycle: Vec<LifecycleListener>,
}

#[wasm_bindgen]
impl WasmNode {
    #[wasm_bindgen(js_name = start)]
    pub async fn start(config: JsValue, storage_namespace: String) -> Result<WasmNode, JsValue> {
        let cfg: NodeConfig = serde_wasm_bindgen::from_value(config)
            .map_err(|e| js_error("config", &e.to_string()))?;
        let lock_value = acquire_profile_lock(&storage_namespace)
            .await
            .map_err(|e| js_error("profile_lock", &format!("{e:?}")))?;
        if lock_value.is_null() || lock_value.is_undefined() {
            return Err(js_error(
                "profile_in_use",
                "this browser profile is already active in another tab",
            ));
        }
        let profile_lock = ProfileLock {
            handle: Some(lock_value),
        };
        let storage = BrowserNodeStorage::open(storage_namespace)
            .await
            .map_err(net_error)?;

        // Identity durability is a startup barrier: never start a swarm with an
        // identity that has not already reached IndexedDB.
        load_or_create_identity_key_with_storage(&cfg.identity_key_path, storage.as_ref())
            .map_err(net_error)?;
        storage.flush().await.map_err(net_error)?;

        let runtime: Arc<dyn PlatformRuntime> = Arc::new(BrowserPlatformRuntime);
        let core_storage: Arc<dyn NodeStorage> = storage.clone();
        let handle = start_node_with_platform(cfg, runtime, core_storage)
            .await
            .map_err(net_error)?;
        let persistence_abort = start_persistence_driver(storage.clone());
        let lifecycle = install_lifecycle(&handle, &storage)?;
        Ok(Self {
            handle,
            storage,
            profile_lock: Some(profile_lock),
            persistence_abort,
            lifecycle,
        })
    }

    #[wasm_bindgen(js_name = peerId)]
    pub fn peer_id(&self) -> String {
        self.handle.peer_id.to_string()
    }

    #[wasm_bindgen(js_name = connectPeer)]
    pub async fn connect_peer(&self, address: String) -> Result<(), JsValue> {
        let addr = address
            .parse()
            .map_err(|e| js_error("multiaddr", &format!("{e}")))?;
        self.handle.connect_peer(addr).await.map_err(net_error)
    }

    #[wasm_bindgen(js_name = disconnectPeer)]
    pub async fn disconnect_peer(&self, peer_id: String) -> Result<(), JsValue> {
        let peer = peer_id
            .parse()
            .map_err(|e| js_error("peer_id", &format!("{e}")))?;
        self.handle.disconnect_peer(peer).await.map_err(net_error)
    }

    #[wasm_bindgen(js_name = sendMessage)]
    pub async fn send_message(
        &self,
        peer_id: String,
        topic: String,
        payload: Uint8Array,
    ) -> Result<(), JsValue> {
        let peer = peer_id
            .parse()
            .map_err(|e| js_error("peer_id", &format!("{e}")))?;
        self.handle
            .send_message(peer, topic, payload.to_vec())
            .await
            .map_err(net_error)
    }

    pub async fn broadcast(&self, topic: String, payload: Uint8Array) -> Result<(), JsValue> {
        self.handle
            .broadcast(topic, payload.to_vec())
            .await
            .map_err(net_error)
    }

    pub async fn subscribe(&self, topic: String) -> Result<WasmSubscription, JsValue> {
        let inner = self.handle.subscribe(topic).await.map_err(net_error)?;
        Ok(WasmSubscription { inner })
    }

    #[wasm_bindgen(js_name = subscribeEvents)]
    pub fn subscribe_events(&self) -> WasmEventSubscription {
        WasmEventSubscription {
            inner: self.handle.subscribe_events(),
        }
    }

    #[wasm_bindgen(js_name = getPeers)]
    pub async fn get_peers(&self) -> Result<JsValue, JsValue> {
        let peers = self.handle.get_peers().await.map_err(net_error)?;
        serde_wasm_bindgen::to_value(&peers).map_err(|e| js_error("serialize", &e.to_string()))
    }

    #[wasm_bindgen(js_name = getMetrics)]
    pub async fn get_metrics(&self, peer_id: Option<String>) -> Result<JsValue, JsValue> {
        let peer = match peer_id {
            Some(raw) => Some(
                raw.parse()
                    .map_err(|e| js_error("peer_id", &format!("{e}")))?,
            ),
            None => None,
        };
        let metrics = self.handle.get_metrics(peer).await.map_err(net_error)?;
        serde_wasm_bindgen::to_value(&metrics).map_err(|e| js_error("serialize", &e.to_string()))
    }

    #[wasm_bindgen(js_name = localBinding)]
    pub async fn local_binding(&self) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(&self.handle.local_binding().await)
            .map_err(|e| js_error("serialize", &e.to_string()))
    }

    pub async fn shutdown(&mut self) -> Result<(), JsValue> {
        remove_lifecycle(&mut self.lifecycle);
        self.persistence_abort.abort();
        self.handle.shutdown().await;
        self.storage.flush().await.map_err(net_error)?;
        if let Some(mut profile_lock) = self.profile_lock.take() {
            profile_lock
                .release()
                .await
                .map_err(|e| js_error("profile_lock_release", &format!("{e:?}")))?;
        }
        Ok(())
    }
}

#[wasm_bindgen]
pub struct WasmSubscription {
    inner: AppSubscription,
}

#[wasm_bindgen]
impl WasmSubscription {
    pub async fn recv(&mut self) -> Result<JsValue, JsValue> {
        let message = self
            .inner
            .recv()
            .await
            .map_err(|e| js_error("subscription", &e.to_string()))?;
        message_to_js(&message)
    }
}

#[wasm_bindgen]
pub struct WasmEventSubscription {
    inner: NodeEventSubscription,
}

#[wasm_bindgen]
impl WasmEventSubscription {
    pub async fn recv(&mut self) -> Result<JsValue, JsValue> {
        let event = self.inner.recv().await.map_err(net_error)?;
        serde_wasm_bindgen::to_value(&event).map_err(|e| js_error("serialize", &e.to_string()))
    }
}

fn message_to_js(message: &crate::AppMessage) -> Result<JsValue, JsValue> {
    let object = Object::new();
    let set = |name: &str, value: &JsValue| {
        Reflect::set(&object, &JsValue::from_str(name), value).map(|_| ())
    };
    set(
        "schemaVersion",
        &JsValue::from_f64(message.schema_version as f64),
    )?;
    set("networkId", &JsValue::from_f64(message.network_id as f64))?;
    set("topic", &JsValue::from_str(&message.topic))?;
    set("sourcePeerId", &JsValue::from_str(&message.source_peer_id))?;
    match &message.target_peer_id {
        Some(v) => set("targetPeerId", &JsValue::from_str(v))?,
        None => set("targetPeerId", &JsValue::NULL)?,
    }
    set(
        "timestampNs",
        &JsValue::from_str(&message.timestamp_ns.to_string()),
    )?;
    set("nonceHex", &JsValue::from_str(&message.nonce_hex))?;
    set(
        "payload",
        &Uint8Array::from(message.payload.as_slice()).into(),
    )?;
    Ok(object.into())
}
fn start_persistence_driver(storage: Arc<BrowserNodeStorage>) -> AbortHandle {
    let (abort, registration) = AbortHandle::new_pair();
    spawn_local(async move {
        let task = async move {
            loop {
                futures_timer::Delay::new(Duration::from_secs(5)).await;
                // A transient IndexedDB failure must not kill networking or the
                // durability pump; the journal remains dirty and is retried.
                let _ = storage.flush().await;
            }
        };
        let _ = Abortable::new(task, registration).await;
    });
    abort
}

fn install_lifecycle(
    handle: &NodeHandle,
    storage: &Arc<BrowserNodeStorage>,
) -> Result<Vec<LifecycleListener>, JsValue> {
    let window =
        web_sys::window().ok_or_else(|| js_error("window", "browser window unavailable"))?;
    let target: web_sys::EventTarget = window.clone().into();
    let mut registrations = Vec::new();

    let online_handle = handle.clone();
    add_listener(&target, "online", &mut registrations, move |_| {
        let handle = online_handle.clone();
        spawn_local(async move {
            let _ = handle.events_tx.send(NodeEvent::Online);
            let _ = handle.browser_wake().await;
        });
    })?;
    let offline_handle = handle.clone();
    add_listener(&target, "offline", &mut registrations, move |_| {
        let _ = offline_handle.events_tx.send(NodeEvent::Offline);
    })?;
    let page_storage = storage.clone();
    add_listener(&target, "pagehide", &mut registrations, move |_| {
        let storage = page_storage.clone();
        spawn_local(async move {
            let _ = storage.flush().await;
        });
    })?;

    if let Some(document) = window.document() {
        let doc_target: web_sys::EventTarget = document.clone().into();
        let visibility_handle = handle.clone();
        let visibility_storage = storage.clone();
        add_listener(
            &doc_target,
            "visibilitychange",
            &mut registrations,
            move |_| {
                if document.visibility_state() == web_sys::VisibilityState::Visible {
                    let handle = visibility_handle.clone();
                    spawn_local(async move {
                        let _ = handle.browser_wake().await;
                    });
                } else {
                    let storage = visibility_storage.clone();
                    spawn_local(async move {
                        let _ = storage.flush().await;
                    });
                }
            },
        )?;
    }
    Ok(registrations)
}

fn add_listener<F>(
    target: &web_sys::EventTarget,
    event: &'static str,
    out: &mut Vec<LifecycleListener>,
    f: F,
) -> Result<(), JsValue>
where
    F: FnMut(Event) + 'static,
{
    let closure = Closure::<dyn FnMut(Event)>::new(f);
    target.add_event_listener_with_callback(event, closure.as_ref().unchecked_ref())?;
    out.push((target.clone(), event, closure));
    Ok(())
}

fn remove_lifecycle(registrations: &mut Vec<LifecycleListener>) {
    for (target, event, closure) in registrations.drain(..) {
        let _ = target.remove_event_listener_with_callback(event, closure.as_ref().unchecked_ref());
    }
}

fn net_error(error: crate::NetError) -> JsValue {
    js_error("p2p_net", &error.to_string())
}
fn js_error(kind: &str, message: &str) -> JsValue {
    let object = Object::new();
    let _ = Reflect::set(
        &object,
        &JsValue::from_str("kind"),
        &JsValue::from_str(kind),
    );
    let _ = Reflect::set(
        &object,
        &JsValue::from_str("message"),
        &JsValue::from_str(message),
    );
    object.into()
}
