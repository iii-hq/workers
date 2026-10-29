//! The observe half of a mock engine, for tests that drive the harness over a
//! raw socket: the hello that advertises `observe.v2`, the
//! `functionregistered` answer to each `registerfunction` (with local
//! dispatch, so the harness calls its own functions in-process, and with the
//! observers already added), the `observeradded` frames that hand the harness
//! an observer, and the activations an `invocationsobserved` report asks for.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

/// The engine's `workerregistered`, advertising the observe features.
pub fn hello() -> Value {
    json!({
        "type": "workerregistered",
        "worker_id": "w-harness-test",
        "features": ["observe.v1", "observe.v2", "local-dispatch.v1"],
    })
}

/// An `observeradded` frame for an observer of `function_id` (hosted by the
/// harness) with `args` and `filters`, calling `target`. The mock engine
/// stamps its revision when it announces it ([`Host::announce`]).
pub fn observer_added(
    id: &str,
    function_id: &str,
    target: &str,
    args: Value,
    filters: Value,
) -> Value {
    json!({
        "type": "observeradded",
        "function_id": function_id,
        "namespace": "default",
        "observer": {
            "id": id,
            "args": args,
            "filters": filters,
            "target": target,
            "capture": ["input", "output", "caller"],
            "on": "success",
        },
    })
}

/// One connection's engine-side view of the functions the harness hosts and
/// their observers. Like the engine, it announces an observer only once its
/// function is registered, and lists the current observers in every
/// `functionregistered`.
#[derive(Default)]
pub struct Host {
    registered: HashSet<String>,
    observers: HashMap<String, Vec<Value>>,
    revision: u64,
}

impl Host {
    fn next_revision(&mut self) -> u64 {
        self.revision += 1;
        self.revision
    }

    /// The engine's answer to a frame the harness sent, when it needs one:
    /// `functionregistered` for each `registerfunction`.
    pub fn answer(&mut self, message: &Value) -> Option<Value> {
        if message["type"] != "registerfunction" {
            return None;
        }
        let id = message["id"].as_str()?.to_string();
        self.registered.insert(id.clone());
        let observers = self.observers.get(&id).cloned().unwrap_or_default();
        Some(json!({
            "type": "functionregistered",
            "id": id,
            "namespace": "default",
            "revision": self.next_revision(),
            "local_dispatch": true,
            "observers": observers,
        }))
    }

    /// Record an [`observer_added`] frame the test pushed, and return it with
    /// its revision when its function is registered already; otherwise the
    /// next `functionregistered` carries it.
    pub fn announce(&mut self, mut frame: Value) -> Option<Value> {
        if frame["type"] != "observeradded" {
            return Some(frame);
        }
        let function_id = frame["function_id"].as_str()?.to_string();
        self.observers
            .entry(function_id.clone())
            .or_default()
            .push(frame["observer"].clone());
        if !self.registered.contains(&function_id) {
            return None;
        }
        frame["revision"] = json!(self.next_revision());
        Some(frame)
    }
}

/// What an `invocationsobserved` report asks the engine to deliver: for each
/// matched observer id, the value it observes (the call's output, which an
/// event source reports once as its input).
pub fn activations(message: &Value) -> Vec<(String, Value)> {
    if message["type"] != "invocationsobserved" {
        return Vec::new();
    }
    let mut out = Vec::new();
    for item in message["items"].as_array().into_iter().flatten() {
        let observed = match item.get("output") {
            Some(output) if !output.is_null() => output.clone(),
            _ => item["input"].clone(),
        };
        for id in item["matched"].as_array().into_iter().flatten() {
            if let Some(id) = id.as_str() {
                out.push((id.to_string(), observed.clone()));
            }
        }
    }
    out
}
