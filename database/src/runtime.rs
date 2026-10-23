//! Per-connection runtime state kept beside the pools: the secret cache,
//! why a connection has no pool, and what each pool was built from (so a
//! reload rebuilds only the connections whose entry or resolved secrets
//! changed — saving one connection never drops another's pool, SSH tunnel or
//! open connections).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::connector::{ConnectError, Connector};
use crate::secrets::SecretCache;

pub struct Runtime {
    pub secrets: Arc<SecretCache>,
    state: RwLock<Built>,
    /// Serializes reloads: a config change, a secret change, the
    /// maintenance tick and `saveConnection` all rebuild through one door.
    pub(crate) reload: tokio::sync::Mutex<()>,
    /// Serializes `saveConnection` / `deleteConnection` read-modify-writes.
    pub(crate) saves: tokio::sync::Mutex<()>,
    /// Native-capture listeners, reconciled on every reload.
    listeners: std::sync::OnceLock<Arc<crate::triggers::NativeListeners>>,
}

#[derive(Default)]
struct Built {
    /// Why a configured connection has no pool.
    errors: HashMap<String, ConnectError>,
    /// The fingerprint and connector each live pool was built from.
    connectors: HashMap<String, Arc<Connector>>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new(Arc::new(SecretCache::default()))
    }
}

impl Runtime {
    pub fn new(secrets: Arc<SecretCache>) -> Self {
        Self {
            secrets,
            state: RwLock::default(),
            reload: tokio::sync::Mutex::new(()),
            saves: tokio::sync::Mutex::new(()),
            listeners: std::sync::OnceLock::new(),
        }
    }

    /// Hand the worker's native-capture listeners to every reload path.
    pub fn set_listeners(&self, listeners: Arc<crate::triggers::NativeListeners>) {
        let _ = self.listeners.set(listeners);
    }

    pub fn listeners(&self) -> Option<Arc<crate::triggers::NativeListeners>> {
        self.listeners.get().cloned()
    }

    /// Why `name` has no pool, if it has none because its build failed.
    pub fn error(&self, name: &str) -> Option<ConnectError> {
        self.read().errors.get(name).cloned()
    }

    pub fn errors(&self) -> HashMap<String, ConnectError> {
        self.read().errors.clone()
    }

    pub fn has_errors(&self) -> bool {
        !self.read().errors.is_empty()
    }

    /// The connector `name`'s live pool was built from.
    pub fn connector(&self, name: &str) -> Option<Arc<Connector>> {
        self.read().connectors.get(name).cloned()
    }

    pub(crate) fn replace(
        &self,
        errors: HashMap<String, ConnectError>,
        connectors: HashMap<String, Arc<Connector>>,
    ) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state.errors = errors;
        state.connectors = connectors;
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Built> {
        self.state.read().unwrap_or_else(|e| e.into_inner())
    }
}
