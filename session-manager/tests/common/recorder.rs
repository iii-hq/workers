//! Recording `EventDeliverer` for @pure scenarios: it stands in for iii.
//! Scenarios add observers (an id, the observed event function, filters and
//! a target); every call of an event function is recorded, and delivered, as
//! the function's output, to each observer of that function whose filters
//! match the call. The filters are evaluated with `iii_sdk::Filter`, the
//! language iii evaluates in production, so these scenarios pin what an
//! observer of each function receives.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::{Doc, Filter};
use serde_json::Value;

use session_manager::events::EventDeliverer;

#[derive(Debug, Clone, PartialEq)]
pub struct Delivery {
    /// The observed event function (`session::on-message-added`, ...).
    pub source: String,
    /// The observer's target.
    pub function_id: String,
    pub payload: Value,
}

#[derive(Clone)]
struct TestObserver {
    id: String,
    source: String,
    filter: Filter,
    target: String,
}

#[derive(Clone, Default)]
pub struct Recorder {
    observers: Arc<Mutex<Vec<TestObserver>>>,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    deliveries: Arc<Mutex<Vec<Delivery>>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Observe `source` with `target`. Filters iii would refuse
    /// (`INVALID_FILTER`) are refused here too.
    pub fn observe(
        &self,
        id: &str,
        source: &str,
        filters: &Value,
        target: &str,
    ) -> Result<(), String> {
        let filter = Filter::parse(filters).map_err(|e| format!("{}: {}", e.code, e.message))?;
        lock(&self.observers).push(TestObserver {
            id: id.to_string(),
            source: source.to_string(),
            filter,
            target: target.to_string(),
        });
        Ok(())
    }

    pub fn dispose(&self, id: &str) {
        lock(&self.observers).retain(|o| o.id != id);
    }

    pub fn all(&self) -> Vec<Delivery> {
        lock(&self.deliveries).clone()
    }

    pub fn to_function(&self, function_id: &str) -> Vec<Delivery> {
        lock(&self.deliveries)
            .iter()
            .filter(|d| d.function_id == function_id)
            .cloned()
            .collect()
    }

    /// The payloads `function_id` was called with.
    pub fn calls_to(&self, function_id: &str) -> Vec<Value> {
        lock(&self.calls)
            .iter()
            .filter(|(f, _)| f == function_id)
            .map(|(_, payload)| payload.clone())
            .collect()
    }

    /// Forget recorded calls and deliveries; observers stay.
    pub fn clear(&self) {
        lock(&self.calls).clear();
        lock(&self.deliveries).clear();
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poison| poison.into_inner())
}

pub struct RecorderDeliverer(pub Recorder);

#[async_trait]
impl EventDeliverer for RecorderDeliverer {
    fn is_observed(&self, function_id: &str) -> bool {
        lock(&self.0.observers)
            .iter()
            .any(|o| o.source == function_id)
    }

    async fn call(&self, function_id: &str, payload: Value) {
        lock(&self.0.calls).push((function_id.to_string(), payload.clone()));
        // An event function returns the event it is called with: the call's
        // input and output are the same value.
        let doc = Doc {
            input: Some(&payload),
            output: Some(&payload),
            ..Doc::default()
        };
        let matched: Vec<Delivery> = lock(&self.0.observers)
            .iter()
            .filter(|o| o.source == function_id && o.filter.matches(&doc))
            .map(|o| Delivery {
                source: function_id.to_string(),
                function_id: o.target.clone(),
                payload: payload.clone(),
            })
            .collect();
        lock(&self.0.deliveries).extend(matched);
    }
}
