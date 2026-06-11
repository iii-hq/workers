//! Recording `EventDeliverer` for @pure scenarios: instead of fanning
//! out over the bus, matched deliveries are captured for assertions.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use session_manager::events::EventDeliverer;

#[derive(Debug, Clone, PartialEq)]
pub struct Delivery {
    pub trigger_type: String,
    pub function_id: String,
    pub payload: Value,
}

#[derive(Clone, Default)]
pub struct Recorder {
    deliveries: Arc<Mutex<Vec<Delivery>>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn all(&self) -> Vec<Delivery> {
        self.lock().clone()
    }

    pub fn to_function(&self, function_id: &str) -> Vec<Delivery> {
        self.lock()
            .iter()
            .filter(|d| d.function_id == function_id)
            .cloned()
            .collect()
    }

    pub fn clear(&self) {
        self.lock().clear();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Delivery>> {
        self.deliveries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

pub struct RecorderDeliverer(pub Recorder);

#[async_trait]
impl EventDeliverer for RecorderDeliverer {
    async fn deliver(&self, trigger_type: &str, function_id: &str, payload: Value) {
        self.0.lock().push(Delivery {
            trigger_type: trigger_type.to_string(),
            function_id: function_id.to_string(),
            payload,
        });
    }
}
