//! The harness and session-manager side, over real iii calls.
//!
//! Everything here is best-effort in a specific sense: the investigation's
//! record lives in this worker's own store, so a harness that is slow, down
//! or has forgotten a session degrades the conversation, never the data. The
//! one call whose failure changes state is `harness::send`, because an
//! investigation whose first pass never started is a failed investigation and
//! saying otherwise would leave a row running forever.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::Runtime;
use crate::investigation::proxies::EngineWindow;
use crate::investigation::{Harness, SendOutcome, TurnMetrics, TurnStatus};
use crate::SentinelError;

pub struct IiiHarness {
    runtime: Runtime,
}

impl IiiHarness {
    pub fn new(runtime: Runtime) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl Harness for IiiHarness {
    async fn grant_filesystem(&self, session_id: &str, root: &str) {
        if root.is_empty() {
            return;
        }
        if let Err(error) = self
            .runtime
            .call(
                "harness::filesystem::grant",
                json!({ "session_id": session_id, "root": root }),
            )
            .await
        {
            // The agent loses `coder::*` on this checkout and says so in
            // `missing_evidence`; the investigation still runs.
            tracing::warn!(
                session_id,
                root,
                %error,
                "could not grant the investigation its checkout"
            );
        }
    }

    async fn send(&self, request: Value) -> Result<SendOutcome, SentinelError> {
        let response = self.runtime.call("harness::send", request).await?;
        if response.get("accepted").and_then(Value::as_bool) == Some(false) {
            return Err(SentinelError::HarnessUnavailable(
                "harness::send refused the investigation turn".into(),
            ));
        }
        let session_id = response
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let turn_id = response
            .get("turn_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                SentinelError::HarnessUnavailable("harness::send answered without a turn id".into())
            })?
            .to_string();
        Ok(SendOutcome {
            session_id,
            turn_id,
        })
    }

    async fn status(&self, session_id: &str) -> Result<Option<TurnStatus>, SentinelError> {
        let response = self
            .runtime
            .call(
                "harness::status",
                json!({ "session_id": session_id, "verbose": true }),
            )
            .await?;
        if response.is_null() {
            return Ok(None);
        }
        Ok(Some(TurnStatus {
            turn_id: response
                .get("turn_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            status: response
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            expects_wake: response
                .get("expects_wake")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            error: response
                .get("result_error")
                .and_then(Value::as_str)
                .map(str::to_string),
        }))
    }

    async fn stop(&self, session_id: &str, turn_id: &str) -> Result<(), SentinelError> {
        self.runtime
            .call(
                "harness::stop",
                json!({ "session_id": session_id, "turn_id": turn_id }),
            )
            .await?;
        Ok(())
    }

    async fn metrics(&self, root_session_id: &str) -> TurnMetrics {
        let Ok(response) = self
            .runtime
            .call(
                "harness::metrics",
                json!({ "root_session_id": root_session_id }),
            )
            .await
        else {
            // Cost is a nice-to-have on a record that is already written.
            return TurnMetrics::default();
        };
        TurnMetrics {
            turns: pick(&response, &["turns", "turn_count"]).and_then(Value::as_u64),
            duration_ms: pick(&response, &["duration_ms", "wall_ms"]).and_then(Value::as_i64),
            cost_usd: pick(&response, &["cost_usd", "total_cost_usd"]).and_then(Value::as_f64),
        }
    }

    async fn ensure_session(
        &self,
        session_id: &str,
        title: &str,
        metadata: Value,
    ) -> Result<(), SentinelError> {
        self.runtime
            .call(
                "session::ensure",
                json!({
                    "session_id": session_id,
                    "title": title,
                    // An investigation is never the person's own session: the
                    // sidebar filters on this.
                    "kind": "automation",
                    "metadata": metadata,
                }),
            )
            .await?;
        Ok(())
    }

    async fn append_message(
        &self,
        session_id: &str,
        entry_id: &str,
        text: &str,
        origin: Value,
    ) -> Result<(), SentinelError> {
        self.runtime
            .call(
                "session::append",
                json!({
                    "session_id": session_id,
                    // A `user` message rather than a `custom` entry: custom
                    // content never reaches the model by harness contract,
                    // and the evidence is the whole point of the session.
                    "message": {
                        "role": "user",
                        "content": [{ "type": "text", "text": text }],
                        "timestamp": crate::ids::now_ms(),
                    },
                    "entry_id": entry_id,
                    "origin": origin,
                }),
            )
            .await?;
        Ok(())
    }
}

/// The live engine, as the investigation's proxies see it.
pub struct IiiEngineWindow {
    runtime: Runtime,
}

/// Flat spans an agent may ask for alongside the tree — enough to read, never
/// a frame the SDK refuses.
const FLAT_SPANS_MAX: u64 = 100;

impl IiiEngineWindow {
    pub fn new(runtime: Runtime) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl EngineWindow for IiiEngineWindow {
    async fn trace(&self, trace_id: &str, include_spans: bool) -> Result<Value, SentinelError> {
        // The same bounded read the ingest uses: an agent asking for a long
        // harness turn must not take the worker's connection down with it.
        let roots = super::bounded_tree(&self.runtime, trace_id).await?;
        let mut tree = json!({ "trace_id": trace_id, "roots": roots });
        if include_spans {
            let spans = self
                .runtime
                .call(
                    "engine::traces::spans",
                    json!({ "trace_id": trace_id, "search_all_spans": true, "limit": FLAT_SPANS_MAX }),
                )
                .await
                .ok()
                .and_then(|response| response.get("spans").cloned());
            if let (Some(map), Some(spans)) = (tree.as_object_mut(), spans) {
                map.insert("spans".into(), spans);
            }
        }
        Ok(tree)
    }

    async fn logs(&self, query: Value) -> Result<Value, SentinelError> {
        self.runtime.call("engine::logs::list", query).await
    }
}

/// First key present, so a harness that renamed a metric still reports one.
fn pick<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| value.get(*key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_metric_is_read_under_either_name() {
        let response = json!({ "turn_count": 3, "total_cost_usd": 0.25 });
        assert_eq!(
            pick(&response, &["turns", "turn_count"]).and_then(Value::as_u64),
            Some(3)
        );
        assert_eq!(
            pick(&response, &["cost_usd", "total_cost_usd"]).and_then(Value::as_f64),
            Some(0.25)
        );
        assert!(pick(&response, &["duration_ms", "wall_ms"]).is_none());
    }
}
