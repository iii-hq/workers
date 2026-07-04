//! `harness::comm` — one event per inter-agent edge (spawn / result / notify /
//! trigger fire), fanned out live to bound subscribers and appended to a
//! durable per-root-session log in iii-state (scope `harness::comm_log`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const COMM: &str = "harness::comm";
pub const COMM_LOG_SCOPE: &str = "harness::comm_log";
/// ponytail: fixed cap; make configurable if real sessions overflow.
pub const COMM_LOG_CAP: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommKind {
    Spawn,
    Result,
    Notify,
    TriggerFire,
}

/// One side of a communication edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommEndpoint {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

/// Trigger identity on `Notify` / `TriggerFire` events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommTrigger {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registered_trigger_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// `"notify"` or `"react"`.
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_id: Option<String>,
}

/// One inter-agent communication event. Appended to the family log AND fanned
/// out live to `harness::comm` bindings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommEvent {
    /// Monotonic per family log; assigned on append (0 until then).
    #[serde(default)]
    pub seq: u64,
    pub at: i64,
    pub root_session_id: String,
    pub kind: CommKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<CommEndpoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<CommEndpoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<CommTrigger>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<CommRef>,
}

/// Parse the log record (`{ "seq": N, "events": { "<seq>": {...} } }`) into
/// events sorted by seq, capped to the newest `cap`. Second value = truncated.
pub fn collect_events(record: &Value, cap: usize) -> (Vec<CommEvent>, bool) {
    let mut events: Vec<CommEvent> = record
        .get("events")
        .and_then(Value::as_object)
        .map(|m| {
            m.values()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    events.sort_by_key(|e: &CommEvent| e.seq);
    let truncated = events.len() > cap;
    if truncated {
        events.drain(..events.len() - cap);
    }
    (events, truncated)
}

/// Human-readable snippet of a JSON value for `CommEvent.summary` (160 chars).
pub fn snippet(v: &Value) -> String {
    const MAX: usize = 160;
    let rendered = match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    if rendered.chars().count() > MAX {
        let mut s: String = rendered.chars().take(MAX).collect();
        s.push_str(" …(truncated)");
        s
    } else {
        rendered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(seq: u64) -> Value {
        json!({
            "seq": seq, "at": 1000 + seq, "root_session_id": "s_root",
            "kind": "spawn",
            "from": { "session_id": "s_root", "turn_id": "t_1" },
            "to": { "session_id": format!("s_child_{seq}") },
        })
    }

    #[test]
    fn kind_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(CommKind::TriggerFire).unwrap(),
            json!("trigger_fire")
        );
        assert_eq!(
            serde_json::to_value(CommKind::Spawn).unwrap(),
            json!("spawn")
        );
    }

    #[test]
    fn ref_field_serializes_as_ref() {
        let e: CommEvent = serde_json::from_value(json!({
            "at": 1, "root_session_id": "s", "kind": "result",
            "ref": { "function_call_id": "fc_1" }
        }))
        .unwrap();
        assert_eq!(e.r#ref.unwrap().function_call_id.as_deref(), Some("fc_1"));
    }

    #[test]
    fn collect_events_sorts_by_seq() {
        let record = json!({ "seq": 3, "events": { "3": ev(3), "1": ev(1), "2": ev(2) } });
        let (events, truncated) = collect_events(&record, 500);
        assert_eq!(
            events.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(!truncated);
    }

    #[test]
    fn collect_events_caps_to_newest() {
        let mut map = serde_json::Map::new();
        for i in 1..=10u64 {
            map.insert(i.to_string(), ev(i));
        }
        let record = json!({ "seq": 10, "events": map });
        let (events, truncated) = collect_events(&record, 4);
        assert_eq!(
            events.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![7, 8, 9, 10]
        );
        assert!(truncated);
    }

    #[test]
    fn collect_events_tolerates_garbage() {
        assert_eq!(collect_events(&Value::Null, 500).0.len(), 0);
        let record = json!({ "seq": 2, "events": { "1": ev(1), "2": "not an event" } });
        let (events, _) = collect_events(&record, 500);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn snippet_truncates() {
        assert_eq!(snippet(&json!("short")), "short");
        let long = "x".repeat(300);
        let s = snippet(&json!(long));
        assert!(s.chars().count() <= 160 + " …(truncated)".chars().count());
        assert!(s.ends_with("…(truncated)"));
        assert_eq!(snippet(&Value::Null), "");
    }
}
