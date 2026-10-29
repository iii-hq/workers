#![allow(dead_code)]
//! An [`Invoker`] for adapter-direct e2e tests: each subscriber id names the
//! function its messages go to, called through the engine. The adapters are
//! driven without `queue::on-message` in between, so a test exercises the
//! transport alone.

use std::sync::Arc;

use async_trait::async_trait;
use iii_queue::subscribers::Invoker;
use iii_sdk::{CallRequest, IIIClient};
use serde_json::Value;

pub struct FunctionInvoker {
    iii: Arc<IIIClient>,
}

impl FunctionInvoker {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl Invoker for FunctionInvoker {
    /// The default [`Invoker::deliver`] calls this with the subscriber id.
    async fn call(&self, function_id: &str, payload: Value) -> Result<Option<Value>, String> {
        self.iii
            .call(CallRequest::new(function_id, payload))
            .await
            .map(Some)
            .map_err(|error| error.to_string())
    }
}
