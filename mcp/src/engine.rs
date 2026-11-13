//! The real engine behind the [`Bus`] and [`Observe`] traits: an iii-sdk
//! `IIIClient`, its `observe_with`, and its `ObserverHandle`.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::{Capture, IIIClient, ObserverHandle, ObserverOptions, Source};
use serde_json::Value;

use crate::bus::{Bus, BusError};
use crate::sessions::{BoxFuture, Observe, ObserveRequest, ObserverLike};

const CAPTURE_NAMES: &str = "input, output, error, caller, metadata";

/// `capture` strings as SDK values; an unknown one is refused before the
/// engine is asked.
pub fn parse_capture(names: &[String]) -> Result<Vec<Capture>, String> {
    names
        .iter()
        .map(|n| match Capture::from(n.as_str()) {
            Capture::Unknown(_) => Err(format!("unknown capture `{n}`; use {CAPTURE_NAMES}")),
            c => Ok(c),
        })
        .collect()
}

/// The engine connection this worker holds.
#[derive(Clone)]
pub struct SdkEngine {
    pub client: Arc<IIIClient>,
}

impl Bus for SdkEngine {
    async fn call(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value, BusError> {
        Bus::call(self.client.as_ref(), function_id, payload, timeout_ms).await
    }
    async fn call_void(&self, function_id: &str, payload: Value) -> Result<(), BusError> {
        Bus::call_void(self.client.as_ref(), function_id, payload).await
    }
}

/// An observer that could not even be sent (bad arguments): `ready` fails.
struct Invalid {
    message: String,
}

impl ObserverLike for Invalid {
    fn id(&self) -> String {
        String::new()
    }
    fn state(&self) -> String {
        "error".into()
    }
    fn ready(&self) -> BoxFuture<'_, Result<(), BusError>> {
        let m = self.message.clone();
        Box::pin(async move { Err(BusError::new("INVALID_ARGS", m)) })
    }
    fn dispose(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

struct Handle(ObserverHandle);

impl ObserverLike for Handle {
    fn id(&self) -> String {
        self.0.id().to_string()
    }
    fn state(&self) -> String {
        serde_json::to_value(self.0.state())
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".into())
    }
    fn ready(&self) -> BoxFuture<'_, Result<(), BusError>> {
        Box::pin(async move {
            self.0.ready().await.map_err(|e| match self.0.reason() {
                Some(r) => BusError::new(r.code, r.message),
                None => BusError::from(e),
            })
        })
    }
    fn dispose(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if let Err(e) = self.0.dispose().await {
                tracing::debug!(error = %e, observer_id = %self.0.id(), "dispose failed");
            }
        })
    }
}

impl Observe for SdkEngine {
    fn observe(&self, req: &ObserveRequest, target: &str) -> Box<dyn ObserverLike> {
        let mut source = Source::function(req.function_id.clone());
        if let Some(ns) = req.namespace.as_ref().filter(|n| !n.is_empty()) {
            source = source.namespace(ns.clone());
        }
        if let Some(args) = req.args.clone().filter(|a| !a.is_null()) {
            source = source.args(args);
        }
        if let Some(filters) = req.filters.clone().filter(|f| !f.is_null()) {
            source = source.filters(filters);
        }
        if let Some(names) = &req.capture {
            match parse_capture(names) {
                Ok(capture) => source = source.capture(capture),
                Err(message) => return Box::new(Invalid { message }),
            }
        }
        let mut options = ObserverOptions::default();
        if let Some(n) = req.max_observations.filter(|n| *n > 0) {
            options = options.max_fires(n);
        }
        if let Some(ms) = req.expires_in_ms.filter(|ms| *ms > 0) {
            options = options.expires_in(Duration::from_millis(ms));
        }
        Box::new(Handle(self.client.observe_with(source, target, options)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_names_parse_and_unknown_ones_are_refused() {
        assert_eq!(
            parse_capture(&["input".into(), "error".into(), "caller".into()])
                .unwrap()
                .len(),
            3
        );
        assert!(parse_capture(&[]).unwrap().is_empty());
        let e = parse_capture(&["input".into(), "everything".into()]).unwrap_err();
        assert!(
            e.contains("everything") && e.contains("input, output, error, caller, metadata"),
            "{e}"
        );
    }
}
