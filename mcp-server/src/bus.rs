//! The slice of the engine this bridge calls, as a trait, so the gateway,
//! session and directory logic is testable without an engine.

use std::future::Future;

use iii_sdk::{errors::Error, CallRequest, IIIClient};
use serde_json::Value;

/// An engine error as `{ code, message }`.
#[derive(Debug, Clone, PartialEq)]
pub struct BusError {
    pub code: String,
    pub message: String,
}

impl BusError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// The function is not registered (the delegate worker is absent).
    pub fn is_missing_function(&self) -> bool {
        let code = self.code.to_uppercase();
        if code == "NOT_FOUND" || code == "FUNCTION_NOT_FOUND" {
            return true;
        }
        let m = self.message.to_lowercase();
        m.contains("function_not_found")
            || m.contains("unknown function")
            || m.contains("no such function")
            || (m.contains("function") && m.contains("not found"))
    }
}

impl std::fmt::Display for BusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl From<Error> for BusError {
    fn from(e: Error) -> Self {
        match e {
            Error::Remote { code, message, .. } => BusError::new(code, message),
            other => BusError::new("ERROR", other.to_string()),
        }
    }
}

pub trait Bus: Send + Sync {
    fn call(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: Option<u64>,
    ) -> impl Future<Output = Result<Value, BusError>> + Send;

    /// Fire and forget: the engine accepts it and nobody waits for a result.
    fn call_void(
        &self,
        function_id: &str,
        payload: Value,
    ) -> impl Future<Output = Result<(), BusError>> + Send;
}

impl Bus for IIIClient {
    async fn call(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value, BusError> {
        IIIClient::call(
            self,
            CallRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms,
            },
        )
        .await
        .map_err(BusError::from)
    }

    async fn call_void(&self, function_id: &str, payload: Value) -> Result<(), BusError> {
        IIIClient::call(
            self,
            CallRequest {
                function_id: function_id.to_string(),
                payload,
                action: Some(iii_sdk::protocol::CallAction::Void),
                timeout_ms: None,
            },
        )
        .await
        .map(|_| ())
        .map_err(BusError::from)
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    type Handler = Arc<dyn Fn(Value) -> Result<Value, BusError> + Send + Sync>;

    /// Answers registered function ids; anything else is `FUNCTION_NOT_FOUND`.
    #[derive(Clone, Default)]
    pub struct FakeBus {
        handlers: HashMap<String, Handler>,
        calls: Arc<Mutex<Vec<(String, Value)>>>,
    }

    impl FakeBus {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn on(
            mut self,
            id: &str,
            f: impl Fn(Value) -> Result<Value, BusError> + Send + Sync + 'static,
        ) -> Self {
            self.handlers.insert(id.to_string(), Arc::new(f));
            self
        }
        pub fn calls(&self) -> Vec<(String, Value)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Bus for FakeBus {
        async fn call(
            &self,
            function_id: &str,
            payload: Value,
            _t: Option<u64>,
        ) -> Result<Value, BusError> {
            self.calls
                .lock()
                .unwrap()
                .push((function_id.to_string(), payload.clone()));
            match self.handlers.get(function_id) {
                Some(h) => h(payload),
                None => Err(BusError::new(
                    "function_not_found",
                    format!("Function {function_id} not found in namespace default."),
                )),
            }
        }

        async fn call_void(&self, function_id: &str, payload: Value) -> Result<(), BusError> {
            self.calls
                .lock()
                .unwrap()
                .push((format!("void:{function_id}"), payload));
            Ok(())
        }
    }
}
