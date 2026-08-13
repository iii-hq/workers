//! The production [`GuestBridge`]: what guest Python's `iii` global can reach.
//!
//! The mirror of [`crate::node_bus`] for the other engine. `iii-python-core`
//! is bus-free, so this is the only thing in the program that turns a guest
//! request into a real engine call.
//!
//! Deliberately one method. The guest gets `trigger` and nothing else — no
//! `register_function`, no triggers, no shutdown. Registration is a WORKER
//! function taking `source` (`code-runner::register_function`), so a
//! guest-callable version would duplicate a capability the API already
//! exposes while dragging in the whole id-ownership apparatus; and `shutdown`
//! is a guest-callable self-DoS that a `return` already covers.

use std::sync::Arc;

use futures::future::BoxFuture;
use iii_python_core::runner::GuestBridge;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::Value;

pub struct IIIBridge {
    iii: Arc<IIIClient>,
}

impl IIIBridge {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

impl GuestBridge for IIIBridge {
    fn call(
        &self,
        fn_id: String,
        payload: Value,
        timeout_ms: u64,
    ) -> BoxFuture<'static, Result<Value, String>> {
        let iii = self.iii.clone();
        Box::pin(async move {
            iii.trigger(TriggerRequest {
                function_id: fn_id,
                payload,
                action: None,
                timeout_ms: Some(timeout_ms),
            })
            .await
            // The guest sees the engine's own message. It is already the
            // caller-facing text a bus error carries, and reshaping it here
            // would only make a tenant's error less like every other worker's.
            .map_err(|e| e.to_string())
        })
    }
}
