//! `harness::comm` — one event per inter-agent edge (spawn / result / notify /
//! trigger fire), fanned out live to bound subscribers and appended to a
//! durable per-root-session log in iii-state (scope `harness::comm_log`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::HarnessError;

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

/// Binding config for `harness::comm` triggers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommBindingConfig {
    /// Only deliver events for this session family.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_session_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct CommBindingFilter {
    root_session_id: Option<String>,
}

impl CommBindingFilter {
    fn parse(raw: &Value) -> Result<Self, String> {
        let raw = if raw.is_null() {
            Value::Object(Default::default())
        } else {
            raw.clone()
        };
        let cfg: CommBindingConfig =
            serde_json::from_value(raw).map_err(|e| format!("invalid comm config: {e}"))?;
        Ok(Self {
            root_session_id: cfg.root_session_id,
        })
    }

    fn matches(&self, root_session_id: &str) -> bool {
        match &self.root_session_id {
            Some(r) => r == root_session_id,
            None => true,
        }
    }
}

#[derive(Debug, Clone)]
struct CommBinding {
    function_id: String,
    filter: CommBindingFilter,
}

#[derive(Clone, Default)]
struct CommSubscriberSet {
    inner: Arc<Mutex<HashMap<String, CommBinding>>>,
}

impl CommSubscriberSet {
    fn add(&self, config: TriggerConfig) -> Result<(), String> {
        let filter = CommBindingFilter::parse(&config.config)?;
        self.lock().insert(
            config.id.clone(),
            CommBinding {
                function_id: config.function_id,
                filter,
            },
        );
        Ok(())
    }

    fn remove(&self, id: &str) {
        self.lock().remove(id);
    }

    fn snapshot(&self) -> Vec<CommBinding> {
        self.lock().values().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, CommBinding>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

struct CommTriggerHandler {
    set: CommSubscriberSet,
}

#[async_trait]
impl TriggerHandler for CommTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let id = config.id.clone();
        let function_id = config.function_id.clone();
        self.set.add(config).map_err(Error::Handler)?;
        tracing::info!(trigger_type = COMM, %id, %function_id, "comm subscription registered");
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.set.remove(&config.id);
        Ok(())
    }
}

/// The comm-event emitter. Cloned into [`crate::deps::Deps`].
#[derive(Clone)]
pub struct CommEvents {
    iii: Arc<IIIClient>,
    subscribers: CommSubscriberSet,
}

impl CommEvents {
    /// Register the `harness::comm` trigger type and return the emitter. Must
    /// run before function registration (same ordering rule as `TurnEvents`).
    pub fn register(iii: &Arc<IIIClient>) -> Self {
        let subscribers = CommSubscriberSet::default();
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                COMM,
                "An inter-agent communication edge (spawn / result / notify / trigger fire).",
                CommTriggerHandler {
                    set: subscribers.clone(),
                },
            )
            .trigger_request_format::<CommBindingConfig>(),
        );
        tracing::info!("registered harness::comm trigger type");
        Self {
            iii: iii.clone(),
            subscribers,
        }
    }

    /// Append to the family log (assigning `seq`) and fan out live. Never
    /// fails the caller — comm visibility must not break the turn loop.
    pub async fn emit(&self, mut event: CommEvent, timeout_ms: u64) {
        match self.append(&mut event, timeout_ms).await {
            Ok(()) => {}
            Err(e) => {
                tracing::warn!(error = %e, root = %event.root_session_id, "comm log append failed");
                // Still fan out live (seq stays 0): live viewers beat no viewers.
            }
        }
        let payload = match serde_json::to_value(&event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "comm event serialize failed");
                return;
            }
        };
        for binding in self.subscribers.snapshot() {
            if !binding.filter.matches(&event.root_session_id) {
                continue;
            }
            let res = self
                .iii
                .trigger(TriggerRequest {
                    function_id: binding.function_id.clone(),
                    payload: payload.clone(),
                    action: Some(TriggerAction::Void),
                    timeout_ms: None,
                })
                .await;
            if let Err(e) = res {
                tracing::warn!(function_id = %binding.function_id, error = %e, "comm fan-out failed");
            }
        }
    }

    /// Two-step durable append: atomically increment `seq`, then merge the
    /// event under its unique seq key (unique keys make concurrent appends
    /// from sibling sessions race-free). Prune every 128 appends.
    async fn append(&self, event: &mut CommEvent, timeout_ms: u64) -> Result<(), HarnessError> {
        let key = event.root_session_id.clone();
        let rec = crate::state::state_update(
            &self.iii,
            COMM_LOG_SCOPE,
            &key,
            vec![json!({ "type": "increment", "path": "seq", "by": 1 })],
            timeout_ms,
        )
        .await?;
        let seq = rec.get("seq").and_then(Value::as_u64).unwrap_or(0);
        event.seq = seq;
        let ev = serde_json::to_value(&*event)
            .map_err(|e| HarnessError::State(format!("comm event serialize: {e}")))?;
        let rec = crate::state::state_update(
            &self.iii,
            COMM_LOG_SCOPE,
            &key,
            vec![json!({ "type": "merge", "path": "events", "value": { seq.to_string(): ev } })],
            timeout_ms,
        )
        .await?;
        // ponytail: prune via whole-record rewrite; a concurrent append during
        // the rare prune window can be lost. Move to engine-side list ops if
        // that ever matters.
        if seq % 128 == 0 {
            let pruned = pruned_record(&rec, COMM_LOG_CAP);
            crate::state::state_set(&self.iii, COMM_LOG_SCOPE, &key, pruned, timeout_ms).await?;
        }
        Ok(())
    }
}

/// Rebuild the log record keeping only the newest `cap` events.
fn pruned_record(record: &Value, cap: usize) -> Value {
    let seq = record.get("seq").cloned().unwrap_or(json!(0));
    let mut entries: Vec<(u64, Value)> = record
        .get("events")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| k.parse::<u64>().ok().map(|n| (n, v.clone())))
                .collect()
        })
        .unwrap_or_default();
    entries.sort_by_key(|(n, _)| *n);
    if entries.len() > cap {
        entries.drain(..entries.len() - cap);
    }
    let events: serde_json::Map<String, Value> = entries
        .into_iter()
        .map(|(n, v)| (n.to_string(), v))
        .collect();
    json!({ "seq": seq, "events": events })
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

    #[test]
    fn binding_filter_matches_root() {
        let f = CommBindingFilter::parse(&json!({ "root_session_id": "s_root" })).unwrap();
        assert!(f.matches("s_root"));
        assert!(!f.matches("s_other"));
        let all = CommBindingFilter::parse(&Value::Null).unwrap();
        assert!(all.matches("s_anything"));
        assert!(CommBindingFilter::parse(&json!({ "bogus": 1 })).is_err());
    }

    #[test]
    fn prune_keeps_newest_cap() {
        let mut map = serde_json::Map::new();
        for i in 1..=600u64 {
            map.insert(i.to_string(), ev(i));
        }
        let record = json!({ "seq": 600, "events": map });
        let pruned = pruned_record(&record, 500);
        let events = pruned.get("events").and_then(Value::as_object).unwrap();
        assert_eq!(events.len(), 500);
        assert!(events.contains_key("600"));
        assert!(!events.contains_key("100"));
        assert_eq!(pruned.get("seq").and_then(Value::as_u64), Some(600));
    }
}
