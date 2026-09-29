//! The agent's only window onto the live engine.
//!
//! `engine::traces::*` and `engine::logs::*` are on the investigation's deny
//! list, and these two functions are what replace them. The difference is not
//! cosmetic: everything that comes back goes through the same redactor the
//! ingest uses before it leaves this worker, so a token sitting in an
//! unrelated span of the same trace never reaches a model.
//!
//! A trace that has already left the engine's ring answers `null` rather than
//! an error — "it is gone" is an answer the agent can act on by working from
//! the frozen bundle instead.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{
    ConfigCell, LogsListRequestV1, LogsListResponseV1, Redactor, SentinelError, TraceGetRequestV1,
    TraceGetResponseV1,
};

/// Raw reads of the engine's telemetry, as a seam.
#[async_trait]
pub trait EngineWindow: Send + Sync {
    /// The span tree of one trace, and optionally its flat span list.
    async fn trace(&self, trace_id: &str, include_spans: bool) -> Result<Value, SentinelError>;
    async fn logs(&self, query: Value) -> Result<Value, SentinelError>;
}

/// Ceiling on a proxied log read, whatever the agent asked for.
const LOG_LIMIT: u32 = 500;

pub struct Proxies {
    engine: Arc<dyn EngineWindow>,
    config: ConfigCell,
}

impl Proxies {
    pub fn new(engine: Arc<dyn EngineWindow>, config: ConfigCell) -> Self {
        Self { engine, config }
    }

    pub async fn trace(
        &self,
        request: TraceGetRequestV1,
    ) -> Result<TraceGetResponseV1, SentinelError> {
        if request.trace_id.trim().is_empty() {
            return Err(SentinelError::invalid("trace_id is required"));
        }
        let mut trace = self
            .engine
            .trace(&request.trace_id, request.include_spans)
            .await?;
        if is_empty_trace(&trace) {
            return Ok(TraceGetResponseV1 {
                trace: None,
                redactions: 0,
            });
        }
        let redactions = self.redactor().await?.redact_json(&mut trace);
        Ok(TraceGetResponseV1 {
            trace: Some(trace),
            redactions,
        })
    }

    pub async fn logs(
        &self,
        request: LogsListRequestV1,
    ) -> Result<LogsListResponseV1, SentinelError> {
        let mut query = json!({
            "limit": request.limit.unwrap_or(200).min(LOG_LIMIT),
        });
        if let Some(trace_id) = request.trace_id.as_deref().filter(|id| !id.is_empty()) {
            query["trace_id"] = json!(trace_id);
        }
        if let Some(service_name) = request.service_name.as_deref().filter(|s| !s.is_empty()) {
            query["service_name"] = json!(service_name);
        }
        if let Some(level) = request.level.as_deref().filter(|s| !s.is_empty()) {
            query["level"] = json!(level);
        }
        if let Some(since_ms) = request.since_ms {
            query["since_ms"] = json!(since_ms);
        }
        let response = self.engine.logs(query).await?;
        let mut logs = response
            .get("logs")
            .and_then(Value::as_array)
            .cloned()
            .map(Value::Array)
            .unwrap_or_else(|| Value::Array(Vec::new()));
        let redactions = self.redactor().await?.redact_json(&mut logs);
        Ok(LogsListResponseV1 {
            logs: match logs {
                Value::Array(logs) => logs,
                _ => Vec::new(),
            },
            redactions,
        })
    }

    /// Compiled from the live configuration on each call: the extra patterns
    /// are a setting somebody can change while an investigation is open.
    async fn redactor(&self) -> Result<Redactor, SentinelError> {
        let config = self.config.read().await.clone();
        Redactor::new(&config.redaction.patterns).map_err(SentinelError::dependency)
    }
}

/// Whether the engine answered with a trace at all. It reports a missing
/// trace as an empty tree rather than an error.
fn is_empty_trace(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map
            .get("roots")
            .and_then(Value::as_array)
            .map(|roots| roots.is_empty())
            .unwrap_or_else(|| map.is_empty()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::RwLock;

    struct Canned {
        trace: Value,
        logs: Value,
    }

    #[async_trait]
    impl EngineWindow for Canned {
        async fn trace(
            &self,
            _trace_id: &str,
            _include_spans: bool,
        ) -> Result<Value, SentinelError> {
            Ok(self.trace.clone())
        }
        async fn logs(&self, _query: Value) -> Result<Value, SentinelError> {
            Ok(self.logs.clone())
        }
    }

    fn config() -> ConfigCell {
        Arc::new(RwLock::new(Arc::new(crate::WorkerConfig::default())))
    }

    #[tokio::test]
    async fn a_trace_that_left_the_ring_reads_as_gone_rather_than_as_an_error() {
        let proxies = Proxies::new(
            Arc::new(Canned {
                trace: json!({ "roots": [] }),
                logs: json!({ "logs": [] }),
            }),
            config(),
        );
        let response = proxies
            .trace(TraceGetRequestV1 {
                trace_id: "abc".into(),
                ..TraceGetRequestV1::default()
            })
            .await
            .expect("an absent trace is an answer");
        assert_eq!(response.trace, None);
    }

    #[tokio::test]
    async fn a_secret_in_a_neighbouring_span_never_reaches_the_agent() {
        let proxies = Proxies::new(
            Arc::new(Canned {
                trace: json!({
                    "roots": [{
                        "name": "execute worker::call",
                        "attributes": [["http.url", "https://user:hunter2@example.test/x"]],
                    }],
                }),
                logs: json!({ "logs": [{ "body": "Authorization: Bearer abc.def.ghi" }] }),
            }),
            config(),
        );
        let trace = proxies
            .trace(TraceGetRequestV1 {
                trace_id: "abc".into(),
                include_spans: false,
                _caller_worker_id: None,
            })
            .await
            .expect("the trace is served");
        let rendered = serde_json::to_string(&trace.trace).expect("serializes");
        assert!(trace.redactions > 0, "{rendered}");
        assert!(!rendered.contains("hunter2"), "{rendered}");

        let logs = proxies
            .logs(LogsListRequestV1 {
                trace_id: Some("abc".into()),
                ..LogsListRequestV1::default()
            })
            .await
            .expect("the logs are served");
        let rendered = serde_json::to_string(&logs.logs).expect("serializes");
        assert!(logs.redactions > 0, "{rendered}");
        assert!(!rendered.contains("abc.def.ghi"), "{rendered}");
    }
}
