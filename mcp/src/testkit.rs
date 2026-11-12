//! Test doubles shared by unit tests: an engine that answers calls
//! ([`FakeBus`](crate::bus::fake::FakeBus)) and creates observers.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::Value;

use crate::bus::{fake::FakeBus, Bus, BusError};
use crate::sessions::{BoxFuture, Observe, ObserveRequest, ObserverLike};

pub struct Obs {
    id: String,
    disposed: Arc<AtomicUsize>,
}

impl ObserverLike for Obs {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn state(&self) -> String {
        "active".into()
    }
    fn ready(&self) -> BoxFuture<'_, Result<(), BusError>> {
        Box::pin(async { Ok(()) })
    }
    fn dispose(&self) -> BoxFuture<'_, ()> {
        self.disposed.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {})
    }
}

/// `FakeBus` for calls; observers `obs-1`, `obs-2`, ... that count disposals.
pub struct Engine {
    pub bus: FakeBus,
    next: AtomicUsize,
    pub disposed: Arc<AtomicUsize>,
    pub observed: std::sync::Mutex<Vec<(ObserveRequest, String)>>,
}

impl Engine {
    pub fn new(bus: FakeBus) -> Self {
        Self {
            bus,
            next: AtomicUsize::new(0),
            disposed: Arc::default(),
            observed: Default::default(),
        }
    }
}

impl Bus for Engine {
    async fn call(&self, id: &str, payload: Value, t: Option<u64>) -> Result<Value, BusError> {
        self.bus.call(id, payload, t).await
    }
    async fn call_void(&self, id: &str, payload: Value) -> Result<(), BusError> {
        self.bus.call_void(id, payload).await
    }
}

impl Observe for Engine {
    fn observe(&self, req: &ObserveRequest, target: &str) -> Box<dyn ObserverLike> {
        self.observed
            .lock()
            .unwrap()
            .push((req.clone(), target.to_string()));
        let n = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        Box::new(Obs {
            id: format!("obs-{n}"),
            disposed: self.disposed.clone(),
        })
    }
}
