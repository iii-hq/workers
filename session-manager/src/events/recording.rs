//! Test support: a deliverer that records the event-function calls it gets.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use super::EventDeliverer;

/// Observes the listed event functions (every one when `observed` is
/// `None`) and records the (function_id, payload) of every call.
pub(crate) struct RecordingDeliverer {
    observed: Option<Vec<&'static str>>,
    pub(crate) seen: Mutex<Vec<(String, Value)>>,
}

impl RecordingDeliverer {
    pub(crate) fn observing_all() -> Arc<Self> {
        Arc::new(Self {
            observed: None,
            seen: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn observing(observed: &[&'static str]) -> Arc<Self> {
        Arc::new(Self {
            observed: Some(observed.to_vec()),
            seen: Mutex::new(Vec::new()),
        })
    }

    /// The functions called so far, in call order.
    pub(crate) fn functions(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|(f, _)| f.clone())
            .collect()
    }
}

#[async_trait]
impl EventDeliverer for RecordingDeliverer {
    fn is_observed(&self, function_id: &str) -> bool {
        self.observed
            .as_ref()
            .is_none_or(|ids| ids.contains(&function_id))
    }

    async fn call(&self, function_id: &str, payload: Value) {
        self.seen
            .lock()
            .unwrap()
            .push((function_id.to_string(), payload));
    }
}
