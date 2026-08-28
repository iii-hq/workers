//! Calling other workers, and being able to test that we did.
//!
//! Every function in this worker except `document::ocr` is pure CPU work over
//! a buffer. OCR is the exception: it needs pixels it cannot produce and a
//! model it does not host, so it talks to `browser` and `llm-router` over the
//! bus.
//!
//! Those calls go through this trait rather than an `IIIClient` directly, for
//! two reasons. A test can drive the whole handler — render, transcribe, cache
//! — against recorded responses with no engine, no Chromium and no model bill.
//! And the dependency stays SOFT: nothing here is declared in
//! `iii.worker.yaml`, so a worker that is not installed surfaces as a failed
//! call this module turns into an instruction, not as a boot-time refusal that
//! would cost a `.docx` reader a browser install.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::Value;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One bus call: a function id, a payload, a JSON answer or a message.
pub trait Bus: Send + Sync {
    fn trigger<'a>(
        &'a self,
        function_id: &'a str,
        payload: Value,
        timeout_ms: u64,
    ) -> BoxFuture<'a, Result<Value, String>>;
}

/// The live bus.
pub struct EngineBus {
    iii: Arc<IIIClient>,
}

impl EngineBus {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

impl Bus for EngineBus {
    fn trigger<'a>(
        &'a self,
        function_id: &'a str,
        payload: Value,
        timeout_ms: u64,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            self.iii
                .trigger(TriggerRequest {
                    function_id: function_id.to_string(),
                    payload,
                    action: None,
                    timeout_ms: Some(timeout_ms),
                })
                .await
                .map_err(|e| e.to_string())
        })
    }
}

/// The worker a failed call was trying to reach, for the message a caller acts
/// on.
///
/// "remote error (NOT_FOUND)" tells someone nothing. "the browser worker is not
/// installed" tells them the one thing they can do about it, which matters more
/// here than anywhere else in this worker: OCR is the only surface whose
/// dependencies are not shipped with it.
pub fn describe_bus_failure(function_id: &str, err: &str) -> String {
    let worker = function_id.split("::").next().unwrap_or(function_id);
    let missing = err.to_ascii_uppercase().contains("NOT_FOUND")
        || err.contains("not registered")
        || err.contains("not found");
    if !missing {
        return format!("{function_id} failed: {err}");
    }
    match worker {
        "browser" => "reading a scanned PDF needs the browser worker to render its pages; \
                      install it with `iii trigger compose::add worker=browser`"
            .to_string(),
        "router" => "transcribing needs a model through llm-router; install it with \
                     `iii trigger compose::add worker=llm-router` and configure a provider"
            .to_string(),
        "state" => format!("{function_id} is unavailable: {err}"),
        _ => format!("{function_id} is not available: {err}"),
    }
}

#[cfg(test)]
pub mod test_bus {
    //! A recorded bus: each function id answers with a queued value or an
    //! error, and every call is logged so a test can assert the order things
    //! happened in — that a page was rendered before it was transcribed, or
    //! that a cached page was never rendered at all.

    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub struct RecordedBus {
        responses: Mutex<HashMap<String, Vec<Result<Value, String>>>>,
        pub calls: Mutex<Vec<(String, Value)>>,
    }

    impl RecordedBus {
        pub fn new() -> Self {
            Self::default()
        }

        /// Queue one answer for `function_id`. Repeated pushes answer repeated
        /// calls in order; the last answer repeats once the queue is empty.
        pub fn on(self, function_id: &str, value: Value) -> Self {
            self.responses
                .lock()
                .expect("lock")
                .entry(function_id.to_string())
                .or_default()
                .push(Ok(value));
            self
        }

        pub fn failing(self, function_id: &str, error: &str) -> Self {
            self.responses
                .lock()
                .expect("lock")
                .entry(function_id.to_string())
                .or_default()
                .push(Err(error.to_string()));
            self
        }

        pub fn called(&self) -> Vec<String> {
            self.calls
                .lock()
                .expect("lock")
                .iter()
                .map(|(id, _)| id.clone())
                .collect()
        }

        pub fn payloads(&self, function_id: &str) -> Vec<Value> {
            self.calls
                .lock()
                .expect("lock")
                .iter()
                .filter(|(id, _)| id == function_id)
                .map(|(_, payload)| payload.clone())
                .collect()
        }
    }

    impl Bus for RecordedBus {
        fn trigger<'a>(
            &'a self,
            function_id: &'a str,
            payload: Value,
            _timeout_ms: u64,
        ) -> BoxFuture<'a, Result<Value, String>> {
            self.calls
                .lock()
                .expect("lock")
                .push((function_id.to_string(), payload));
            let mut responses = self.responses.lock().expect("lock");
            let queued = responses.get_mut(function_id);
            let answer = match queued {
                Some(queue) if queue.len() > 1 => queue.remove(0),
                Some(queue) if queue.len() == 1 => queue[0].clone(),
                _ => Err(format!(
                    "remote error (NOT_FOUND): {function_id} not registered"
                )),
            };
            Box::pin(async move { answer })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The point of the whole module: a missing worker has to arrive as an
    /// instruction, because OCR is the one surface here whose dependencies are
    /// not shipped with the binary.
    #[test]
    fn a_missing_worker_becomes_something_to_do() {
        let browser = describe_bus_failure(
            "browser::screenshot",
            "remote error (NOT_FOUND): browser::screenshot not registered",
        );
        assert!(
            browser.contains("iii trigger compose::add worker=browser"),
            "{browser}"
        );

        let router = describe_bus_failure(
            "router::complete",
            "remote error (NOT_FOUND): router::complete not registered",
        );
        assert!(router.contains("llm-router"), "{router}");
    }

    /// A real failure from a worker that IS there passes through: the caller
    /// needs the reason, not advice to install something already installed.
    #[test]
    fn a_live_worker_failure_is_reported_as_it_came() {
        let described = describe_bus_failure("browser::navigate", "scheme `file` is not allowed");
        assert!(
            described.contains("scheme `file` is not allowed"),
            "{described}"
        );
        assert!(
            !described.contains("iii trigger compose::add"),
            "{described}"
        );
    }
}
