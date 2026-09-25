//! Browser platform adapter: synchronous core storage backed by an IndexedDB
//! journal that is loaded before swarm startup and flushed asynchronously.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures::channel::oneshot;
use serde::{Deserialize, Serialize};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Event, IdbDatabase, IdbOpenDbRequest, IdbRequest, IdbTransactionMode};

use super::{NodeStorage, PlatformRuntime};
use crate::common::error::NetError;
use crate::{EnvironmentConfig, PlatformKind};

const STORE: &str = "state";
const STATE_KEY: &str = "journal";
const INDEXED_DB_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedJournal {
    values: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug)]
pub struct BrowserNodeStorage {
    namespace: String,
    values: RwLock<BTreeMap<String, Vec<u8>>>,
    revision: AtomicU64,
    flushed_revision: AtomicU64,
    flush_guard: tokio::sync::Mutex<()>,
}

impl BrowserNodeStorage {
    pub async fn open(namespace: impl Into<String>) -> Result<Arc<Self>, NetError> {
        let namespace = namespace.into();
        if namespace.trim().is_empty() {
            return Err(NetError::Build(
                "browser storage namespace must not be empty".into(),
            ));
        }
        let db = open_database(&format!("p2p-net:{namespace}")).await?;
        let values = load_journal(&db).await;
        db.close();
        let values = values?;
        Ok(Arc::new(Self {
            namespace,
            values: RwLock::new(values),
            revision: AtomicU64::new(0),
            flushed_revision: AtomicU64::new(0),
            flush_guard: tokio::sync::Mutex::new(()),
        }))
    }

    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.revision.load(Ordering::Acquire) != self.flushed_revision.load(Ordering::Acquire)
    }

    pub async fn flush(&self) -> Result<(), NetError> {
        let _flush_guard = self.flush_guard.lock().await;
        if !self.is_dirty() {
            return Ok(());
        }
        let revision = self.revision.load(Ordering::Acquire);
        let snapshot = self
            .values
            .read()
            .expect("browser storage values lock")
            .clone();
        let encoded = serde_json::to_string(&PersistedJournal { values: snapshot })
            .map_err(|e| NetError::Build(format!("serialize browser storage: {e}")))?;
        let db = open_database(&format!("p2p-net:{}", self.namespace)).await?;
        let tx = db
            .transaction_with_str_and_mode(STORE, IdbTransactionMode::Readwrite)
            .map_err(js_build)?;
        let store = tx.object_store(STORE).map_err(js_build)?;
        let request = store
            .put_with_key(&JsValue::from_str(&encoded), &JsValue::from_str(STATE_KEY))
            .map_err(js_build)?;
        let persisted = futures::try_join!(await_request(&request), await_transaction(&tx));
        db.close();
        let _ = persisted?;
        // Never mark writes that raced this transaction as durable. A later
        // flush observes revision > flushed_revision and persists them.
        self.flushed_revision.fetch_max(revision, Ordering::Release);
        Ok(())
    }
}

impl NodeStorage for BrowserNodeStorage {
    fn storage_kind(&self) -> &'static str {
        "indexeddb-journal"
    }

    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, NetError> {
        Ok(self
            .values
            .read()
            .expect("browser storage values lock")
            .get(key)
            .cloned())
    }
    fn write_secret(&self, key: &str, value: &[u8]) -> Result<(), NetError> {
        self.write_public(key, value)
    }
    fn write_secret_if_absent(&self, key: &str, value: &[u8]) -> Result<bool, NetError> {
        let mut values = self.values.write().expect("browser storage values lock");
        if values.contains_key(key) {
            return Ok(false);
        }
        values.insert(key.to_string(), value.to_vec());
        self.revision.fetch_add(1, Ordering::AcqRel);
        Ok(true)
    }
    fn write_public(&self, key: &str, value: &[u8]) -> Result<(), NetError> {
        self.values
            .write()
            .expect("browser storage values lock")
            .insert(key.to_string(), value.to_vec());
        self.revision.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), NetError> {
        if self
            .values
            .write()
            .expect("browser storage values lock")
            .remove(key)
            .is_some()
        {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BrowserPlatformRuntime;

impl PlatformRuntime for BrowserPlatformRuntime {
    fn runtime_name(&self) -> &'static str {
        "wasm-browser"
    }
    fn platform_kind(&self) -> PlatformKind {
        PlatformKind::Wasm
    }
    fn default_data_dir(&self) -> Option<PathBuf> {
        None
    }
    fn can_listen_tcp(&self) -> bool {
        false
    }
    fn can_listen_quic(&self) -> bool {
        false
    }
    fn can_accept_inbound(&self) -> Option<bool> {
        Some(false)
    }
    fn is_battery_sensitive(&self) -> bool {
        false
    }
    fn is_background_restricted(&self) -> bool {
        true
    }
    fn environment_config(&self) -> EnvironmentConfig {
        EnvironmentConfig {
            platform_hint: Some(PlatformKind::Wasm),
            can_listen_tcp: Some(false),
            can_listen_quic: Some(false),
            can_accept_inbound: Some(false),
            background_restricted: Some(true),
            ..EnvironmentConfig::default()
        }
    }
}

async fn open_database(name: &str) -> Result<IdbDatabase, NetError> {
    let factory = web_sys::window()
        .ok_or_else(|| NetError::Build("browser window unavailable".into()))?
        .indexed_db()
        .map_err(js_build)?
        .ok_or_else(|| NetError::Build("IndexedDB unavailable".into()))?;
    let open = factory.open_with_u32(name, 1).map_err(js_build)?;
    let upgrade_request = open.clone();
    let on_upgrade = Closure::<dyn FnMut(Event)>::new(move |_| {
        if let Ok(value) = upgrade_request.result() {
            if let Ok(db) = value.dyn_into::<IdbDatabase>() {
                let _ = db.create_object_store(STORE);
            }
        }
    });
    open.set_onupgradeneeded(Some(on_upgrade.as_ref().unchecked_ref()));
    let value = await_open(&open).await?;
    open.set_onupgradeneeded(None);
    drop(on_upgrade);
    value
        .dyn_into::<IdbDatabase>()
        .map_err(|_| NetError::Build("IndexedDB open returned non-database".into()))
}

async fn load_journal(db: &IdbDatabase) -> Result<BTreeMap<String, Vec<u8>>, NetError> {
    let tx = db
        .transaction_with_str_and_mode(STORE, IdbTransactionMode::Readonly)
        .map_err(js_build)?;
    let store = tx.object_store(STORE).map_err(js_build)?;
    let req = store.get(&JsValue::from_str(STATE_KEY)).map_err(js_build)?;
    let (value, ()) = futures::try_join!(await_request(&req), await_transaction(&tx))?;
    if value.is_null() || value.is_undefined() {
        return Ok(BTreeMap::new());
    }
    let Some(raw) = value.as_string() else {
        return Err(NetError::Build("IndexedDB journal is not a string".into()));
    };
    let journal: PersistedJournal = serde_json::from_str(&raw)
        .map_err(|e| NetError::Build(format!("decode IndexedDB journal: {e}")))?;
    Ok(journal.values)
}

async fn await_open(req: &IdbOpenDbRequest) -> Result<JsValue, NetError> {
    let base: IdbRequest = req.clone().unchecked_into();
    await_request(&base).await
}

async fn await_request(req: &IdbRequest) -> Result<JsValue, NetError> {
    let (tx, rx) = oneshot::channel::<Result<JsValue, String>>();
    let tx = std::rc::Rc::new(std::cell::RefCell::new(Some(tx)));
    let success_req = req.clone();
    let success_tx = tx.clone();
    let success = Closure::<dyn FnMut(Event)>::new(move |_| {
        let result = success_req.result().map_err(js_string);
        if let Some(tx) = success_tx.borrow_mut().take() {
            let _ = tx.send(result);
        }
    });
    let error_req = req.clone();
    let error_tx = tx;
    let error = Closure::<dyn FnMut(Event)>::new(move |_| {
        let reason = error_req
            .error()
            .ok()
            .flatten()
            .map(|e| e.message())
            .unwrap_or_else(|| "IndexedDB request failed".into());
        if let Some(tx) = error_tx.borrow_mut().take() {
            let _ = tx.send(Err(reason));
        }
    });
    req.set_onsuccess(Some(success.as_ref().unchecked_ref()));
    req.set_onerror(Some(error.as_ref().unchecked_ref()));
    let out = crate::runtime::timeout(INDEXED_DB_OPERATION_TIMEOUT, rx)
        .await
        .map_err(|_| NetError::Build("IndexedDB request timed out after 30s".into()))?
        .map_err(|_| NetError::Build("IndexedDB request callback dropped".into()))?;
    req.set_onsuccess(None);
    req.set_onerror(None);
    drop((success, error));
    out.map_err(NetError::Build)
}

async fn await_transaction(tx: &web_sys::IdbTransaction) -> Result<(), NetError> {
    let (send, recv) = oneshot::channel::<Result<(), String>>();
    let send = std::rc::Rc::new(std::cell::RefCell::new(Some(send)));
    let ok_send = send.clone();
    let complete = Closure::<dyn FnMut(Event)>::new(move |_| {
        if let Some(tx) = ok_send.borrow_mut().take() {
            let _ = tx.send(Ok(()));
        }
    });
    let err_send = send;
    let error = Closure::<dyn FnMut(Event)>::new(move |_| {
        if let Some(tx) = err_send.borrow_mut().take() {
            let _ = tx.send(Err("IndexedDB transaction failed".into()));
        }
    });
    tx.set_oncomplete(Some(complete.as_ref().unchecked_ref()));
    tx.set_onabort(Some(error.as_ref().unchecked_ref()));
    tx.set_onerror(Some(error.as_ref().unchecked_ref()));
    let out = crate::runtime::timeout(INDEXED_DB_OPERATION_TIMEOUT, recv)
        .await
        .map_err(|_| NetError::Build("IndexedDB transaction timed out after 30s".into()))?
        .map_err(|_| NetError::Build("IndexedDB transaction callback dropped".into()))?;
    tx.set_oncomplete(None);
    tx.set_onabort(None);
    tx.set_onerror(None);
    drop((complete, error));
    out.map_err(NetError::Build)
}

fn js_string(value: JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{value:?}"))
}
fn js_build(value: JsValue) -> NetError {
    NetError::Build(js_string(value))
}
