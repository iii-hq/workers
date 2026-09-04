//! `harness::status` — read the current turn status for a session
//! (harness.md § `harness::status`). Read-only; safe to expose to agents.

use schemars::JsonSchema;
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::types::turn::TurnStatus;

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct StatusRequest {
    pub session_id: String,
    /// Include the full runtime report and unmodified result.
    #[serde(default)]
    pub verbose: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChildRef {
    pub function_call_id: String,
    pub session_id: String,
    pub turn_id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StatusReport {
    pub session_id: String,
    pub turn_id: Option<String>,
    pub status: TurnStatus,
    pub step: u64,
    pub turn_count: u32,
    pub max_turns: Option<u32>,
    pub validation_retries: Option<u32>,
    pub max_validation_retries: Option<u32>,
    pub transient_resumes: Option<u32>,
    pub max_transient_resumes: Option<u32>,
    pub partial_result_available: Option<bool>,
    pub depth: Option<u32>,
    pub pending_function_calls: Option<Vec<String>>,
    pub children: Vec<ChildRef>,
    /// The session owns an armed wake (a one-shot notify subscription): a
    /// completed turn here is NOT the run's outcome — a later turn in this
    /// session carries it. Mirrors the `terminal` flag on
    /// `harness::turn-completed` (`expects_wake == !terminal`). Pollers
    /// (e.g. workflow reconcile) must treat `completed && expects_wake` as
    /// still running.
    #[serde(default)]
    pub expects_wake: bool,
    /// WHAT the session is parked on, when `expects_wake`: each armed wake's
    /// watch and deadline, so "parked 12m on state operation_meta/status —
    /// never written" is readable from the outside instead of the session
    /// just looking quietly done.
    pub armed_wakes: Option<Vec<crate::bindings::ArmedWake>>,
    /// Messages queued while a step streams, in arrival order; they land in
    /// the transcript when the stream ends.
    pub queued: Option<Vec<crate::state::QueuedMessage>>,
    pub result: Option<Value>,
    pub result_error: Option<String>,
}

const LEAN_RESULT_CHAR_LIMIT: usize = 600;
const LEAN_RESULT_TRUNCATION_SUFFIX: &str = " …(truncated; use verbose: true for the full result)";

impl StatusReport {
    fn is_verbose(&self) -> bool {
        // Verbose reports always populate this field, including with `[]`.
        self.pending_function_calls.is_some()
    }
}

impl Serialize for StatusReport {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let verbose = self.is_verbose();
        let mut report = serializer.serialize_map(None)?;
        report.serialize_entry("session_id", &self.session_id)?;
        report.serialize_entry("turn_id", &self.turn_id)?;
        report.serialize_entry("status", &self.status)?;
        report.serialize_entry("step", &self.step)?;
        report.serialize_entry("turn_count", &self.turn_count)?;

        if verbose {
            serialize_optional(&mut report, "max_turns", &self.max_turns)?;
            serialize_optional(&mut report, "validation_retries", &self.validation_retries)?;
            serialize_optional(
                &mut report,
                "max_validation_retries",
                &self.max_validation_retries,
            )?;
            serialize_optional(&mut report, "transient_resumes", &self.transient_resumes)?;
            serialize_optional(
                &mut report,
                "max_transient_resumes",
                &self.max_transient_resumes,
            )?;
            serialize_optional(
                &mut report,
                "partial_result_available",
                &self.partial_result_available,
            )?;
            serialize_optional(&mut report, "depth", &self.depth)?;
            serialize_optional(
                &mut report,
                "pending_function_calls",
                &self.pending_function_calls,
            )?;
        }

        report.serialize_entry("children", &self.children)?;
        report.serialize_entry("expects_wake", &self.expects_wake)?;

        if verbose {
            serialize_non_empty(&mut report, "armed_wakes", &self.armed_wakes)?;
            serialize_non_empty(&mut report, "queued", &self.queued)?;
        }

        if let Some(result) = &self.result {
            if verbose {
                report.serialize_entry("result", result)?;
            } else {
                let result = lean_result(result).map_err(serde::ser::Error::custom)?;
                report.serialize_entry("result", &result)?;
            }
        }

        if verbose {
            serialize_optional(&mut report, "result_error", &self.result_error)?;
        } else {
            report.serialize_entry("result_error", &self.result_error)?;
        }

        report.end()
    }
}

fn serialize_optional<M, T>(
    map: &mut M,
    key: &'static str,
    value: &Option<T>,
) -> Result<(), M::Error>
where
    M: SerializeMap,
    T: Serialize,
{
    if let Some(value) = value {
        map.serialize_entry(key, value)?;
    }
    Ok(())
}

fn serialize_non_empty<M, T>(
    map: &mut M,
    key: &'static str,
    value: &Option<Vec<T>>,
) -> Result<(), M::Error>
where
    M: SerializeMap,
    T: Serialize,
{
    if let Some(value) = value.as_ref().filter(|value| !value.is_empty()) {
        map.serialize_entry(key, value)?;
    }
    Ok(())
}

fn lean_result(result: &Value) -> Result<Value, serde_json::Error> {
    let compact = serde_json::to_string(result)?;
    if compact.chars().count() <= LEAN_RESULT_CHAR_LIMIT {
        return Ok(result.clone());
    }

    let truncated = compact
        .chars()
        .take(LEAN_RESULT_CHAR_LIMIT)
        .collect::<String>();
    Ok(Value::String(format!(
        "{truncated}{LEAN_RESULT_TRUNCATION_SUFFIX}"
    )))
}

/// `null` for unknown sessions.
pub async fn handle(deps: &Deps, req: StatusRequest) -> Result<Option<StatusReport>, HarnessError> {
    let cfg = deps.cfg().await;
    let Some(record) =
        crate::state::get_turn(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?
    else {
        return Ok(None);
    };
    let queued = if req.verbose {
        Some(crate::state::list_queued(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?)
    } else {
        None
    };
    let children = record
        .spawned_children()
        .into_iter()
        .map(|c| ChildRef {
            function_call_id: c.function_call_id,
            session_id: c.session_id,
            turn_id: c.turn_id,
        })
        .collect();
    // One store read answers both: the flag (fail-closed on an unreadable
    // store, same as `session_expects_wake`) and the detail rows behind it.
    let armed = crate::bindings::armed_wakes(deps, &req.session_id).await;
    let expects_wake = armed.as_ref().is_none_or(|wakes| !wakes.is_empty());
    let armed_wakes = req
        .verbose
        .then(|| armed.unwrap_or_default())
        .filter(|wakes| !wakes.is_empty());
    let queued = queued.filter(|messages| !messages.is_empty());
    Ok(Some(StatusReport {
        session_id: record.session_id.clone(),
        turn_id: Some(record.turn_id.clone()),
        status: record.status,
        step: record.step,
        turn_count: record.turn_count,
        max_turns: req.verbose.then_some(record.options.max_turns),
        validation_retries: req.verbose.then_some(record.validation_retries),
        max_validation_retries: req.verbose.then_some(record.options.max_validation_retries),
        transient_resumes: req.verbose.then_some(record.transient_resumes),
        max_transient_resumes: req.verbose.then_some(record.options.max_transient_resumes),
        partial_result_available: req.verbose.then_some(record.result.is_some()),
        depth: req.verbose.then_some(record.depth),
        pending_function_calls: req.verbose.then(|| record.pending_call_ids()),
        children,
        queued,
        result: record.result.clone(),
        result_error: record.result_error.clone(),
        expects_wake,
        armed_wakes,
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::*;

    const TRUNCATION_SUFFIX: &str = " …(truncated; use verbose: true for the full result)";

    fn lean_report(result: Option<Value>) -> StatusReport {
        let mut value = json!({
            "session_id": "s_parent",
            "turn_id": "t_parent",
            "status": "completed",
            "step": 4,
            "turn_count": 3,
            "expects_wake": false,
            "result_error": null,
            "children": [{
                "function_call_id": "call_child",
                "session_id": "s_child",
                "turn_id": "t_child"
            }]
        });
        if let Some(result) = result {
            value["result"] = result;
        }
        serde_json::from_value(value).expect("lean status fixture deserializes")
    }

    fn keys(value: &Value) -> BTreeSet<&str> {
        value
            .as_object()
            .expect("status serializes as an object")
            .keys()
            .map(String::as_str)
            .collect()
    }

    #[test]
    fn omitted_verbose_request_defaults_to_false() {
        let request: StatusRequest =
            serde_json::from_value(json!({ "session_id": "s_parent" })).unwrap();

        let serialized = serde_json::to_value(request).unwrap();

        assert_eq!(serialized["verbose"], false);
    }

    #[test]
    fn lean_status_serializes_only_the_model_facing_keys() {
        let serialized = serde_json::to_value(lean_report(None)).unwrap();

        assert_eq!(
            keys(&serialized),
            BTreeSet::from([
                "children",
                "expects_wake",
                "result_error",
                "session_id",
                "status",
                "step",
                "turn_count",
                "turn_id",
            ])
        );
        assert!(serialized["result_error"].is_null());
        assert_eq!(
            serialized["children"],
            json!([{
                "function_call_id": "call_child",
                "session_id": "s_child",
                "turn_id": "t_child"
            }])
        );
    }

    #[test]
    fn lean_status_preserves_a_short_result_json_value() {
        let result = json!({ "answer": [1, true, "done"] });

        let serialized = serde_json::to_value(lean_report(Some(result.clone()))).unwrap();

        assert_eq!(serialized["result"], result);
        assert!(serialized["result"].is_object());
    }

    #[test]
    fn lean_status_preserves_a_result_at_the_600_character_boundary() {
        let result = Value::String("x".repeat(598));
        assert_eq!(serde_json::to_string(&result).unwrap().chars().count(), 600);

        let serialized = serde_json::to_value(lean_report(Some(result.clone()))).unwrap();

        assert_eq!(serialized["result"], result);
    }

    #[test]
    fn lean_status_truncates_a_long_result_by_unicode_scalar_count() {
        let result = json!({ "answer": "猫".repeat(700) });
        let compact_json = serde_json::to_string(&result).unwrap();
        let expected = format!(
            "{}{}",
            compact_json.chars().take(600).collect::<String>(),
            TRUNCATION_SUFFIX
        );

        let serialized = serde_json::to_value(lean_report(Some(result))).unwrap();

        assert_eq!(serialized["result"], Value::String(expected));
    }

    #[test]
    fn verbose_status_preserves_the_complete_legacy_response() {
        let full_result = json!({ "answer": "猫".repeat(700) });
        let fixture = json!({
            "session_id": "s_parent",
            "turn_id": "t_parent",
            "status": "awaiting_functions",
            "step": 4,
            "turn_count": 3,
            "max_turns": 12,
            "validation_retries": 1,
            "max_validation_retries": 2,
            "transient_resumes": 1,
            "max_transient_resumes": 3,
            "partial_result_available": true,
            "depth": 1,
            "pending_function_calls": ["call_pending"],
            "children": [{
                "function_call_id": "call_child",
                "session_id": "s_child",
                "turn_id": "t_child"
            }],
            "expects_wake": true,
            "armed_wakes": [{
                "subscription_id": "sub_1",
                "trigger_type": "state:change",
                "config": { "scope": "work" },
                "created_at": 10,
                "expires_at": 20
            }],
            "queued": [{
                "id": "q_1",
                "session_id": "s_parent",
                "message": {
                    "role": "user",
                    "content": [],
                    "timestamp": 11
                },
                "entry_id": "e_1",
                "queued_at": 12
            }],
            "result": full_result,
            "result_error": "contract failed"
        });
        let report: StatusReport = serde_json::from_value(fixture.clone()).unwrap();

        let serialized = serde_json::to_value(report).unwrap();

        assert_eq!(serialized, fixture);
    }

    #[test]
    fn verbose_status_keeps_legacy_empty_collection_omissions() {
        let mut fixture = json!({
            "session_id": "s_parent",
            "turn_id": "t_parent",
            "status": "running",
            "step": 1,
            "turn_count": 1,
            "max_turns": 12,
            "validation_retries": 0,
            "max_validation_retries": 2,
            "transient_resumes": 0,
            "max_transient_resumes": 3,
            "partial_result_available": false,
            "depth": 0,
            "pending_function_calls": [],
            "children": [],
            "expects_wake": false,
            "armed_wakes": [],
            "queued": []
        });
        let report: StatusReport = serde_json::from_value(fixture.clone()).unwrap();
        let fixture = fixture.as_object_mut().unwrap();
        fixture.remove("armed_wakes");
        fixture.remove("queued");
        let expected = Value::Object(std::mem::take(fixture));

        let serialized = serde_json::to_value(report).unwrap();

        assert_eq!(serialized, expected);
        assert_eq!(serialized["pending_function_calls"], json!([]));
        assert!(!serialized.as_object().unwrap().contains_key("result_error"));
    }
}
