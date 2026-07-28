use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

pub(super) type Totals = BTreeMap<String, (i64, f64)>;

#[derive(Debug, Default)]
pub(super) struct Evidence {
    pub(super) existing_tables: BTreeSet<String>,
    pub(super) writer_spawns: WriterSpawnEvidence,
    pub(super) watch: WatchEvidence,
    pub(super) order_summary: Option<OrderSummary>,
    pub(super) writers: Vec<WriterStatus>,
    pub(super) direct_totals: Option<Totals>,
    pub(super) stored_totals: Option<Totals>,
    pub(super) reports: Vec<FinalReport>,
    pub(super) reactor_spawned_by_trigger: bool,
    pub(super) finalizer_spawned_by_trigger: bool,
    pub(super) active_run_triggers: usize,
}

#[derive(Debug, Default)]
pub(super) struct WriterSpawnEvidence {
    pub(super) call_count: usize,
    pub(super) session_ids: BTreeSet<String>,
    pub(super) max_parallel_calls: usize,
    pub(super) sessions_in_tree: BTreeSet<String>,
}

#[derive(Debug, Default)]
pub(super) struct WatchEvidence {
    pub(super) first_writer_spawn: Option<usize>,
    pub(super) trigger_catalog: Option<usize>,
    pub(super) row_change_probe: Option<usize>,
    pub(super) state_reaction: Option<usize>,
}

#[derive(Debug, Default)]
pub(super) struct OrderSummary {
    pub(super) rows_written: Option<i64>,
    pub(super) distinct_ids: Option<i64>,
    pub(super) writer_count: Option<i64>,
    pub(super) missing_created_at: Option<i64>,
}

#[derive(Debug, Default)]
pub(super) struct WriterStatus {
    pub(super) writer: Option<String>,
    pub(super) status: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct FinalReport {
    pub(super) run_id: Option<String>,
    pub(super) watch_mechanism: Option<String>,
    pub(super) fallback_reason: Option<String>,
    pub(super) events_received: Option<i64>,
    pub(super) rows_written: Option<i64>,
    pub(super) elapsed_ms: Option<i64>,
    pub(super) totals_match: Option<bool>,
    pub(super) no_notification_loss: Option<bool>,
    pub(super) no_double_counting: Option<bool>,
    pub(super) trigger_spawned_reactor: Option<bool>,
    pub(super) no_inline_waiting: Option<bool>,
    pub(super) reactor_session_id: Option<String>,
    pub(super) spawning_event: Option<String>,
    pub(super) finalizer_session_id: Option<String>,
}

impl OrderSummary {
    pub(super) fn from_value(value: &Value) -> Self {
        Self {
            rows_written: integer_field(value, "rows_written"),
            distinct_ids: integer_field(value, "distinct_ids"),
            writer_count: integer_field(value, "writer_count"),
            missing_created_at: integer_field(value, "missing_created_at"),
        }
    }
}

impl WriterStatus {
    pub(super) fn from_value(value: &Value) -> Self {
        Self {
            writer: string_field(value, "writer"),
            status: string_field(value, "status"),
        }
    }
}

impl FinalReport {
    pub(super) fn from_value(value: &Value) -> Self {
        Self {
            run_id: string_field(value, "run_id"),
            watch_mechanism: string_field(value, "watch_mechanism"),
            fallback_reason: string_field(value, "fallback_reason"),
            events_received: integer_field(value, "events_received"),
            rows_written: integer_field(value, "rows_written"),
            elapsed_ms: integer_field(value, "elapsed_ms"),
            totals_match: boolean_field(value, "totals_match"),
            no_notification_loss: boolean_field(value, "no_notification_loss"),
            no_double_counting: boolean_field(value, "no_double_counting"),
            trigger_spawned_reactor: boolean_field(value, "trigger_spawned_reactor"),
            no_inline_waiting: boolean_field(value, "no_inline_waiting"),
            reactor_session_id: string_field(value, "reactor_session_id"),
            spawning_event: string_field(value, "spawning_event"),
            finalizer_session_id: string_field(value, "finalizer_session_id"),
        }
    }
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_string)
}

pub(super) fn integer_field(value: &Value, field: &str) -> Option<i64> {
    value
        .get(field)
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
}

fn boolean_field(value: &Value, field: &str) -> Option<bool> {
    value.get(field).and_then(|value| match value {
        Value::Bool(boolean) => Some(*boolean),
        Value::Number(number) => number.as_i64().map(|number| number != 0),
        Value::String(string) => match string.to_ascii_lowercase().as_str() {
            "true" | "pass" | "passed" | "1" => Some(true),
            "false" | "fail" | "failed" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn final_report_accepts_database_encoded_booleans() {
        let report = FinalReport::from_value(&json!({
            "totals_match": true,
            "no_notification_loss": 1,
            "no_double_counting": "passed",
            "trigger_spawned_reactor": "true",
            "no_inline_waiting": "1"
        }));
        assert_eq!(report.totals_match, Some(true));
        assert_eq!(report.no_notification_loss, Some(true));
        assert_eq!(report.no_double_counting, Some(true));
        assert_eq!(report.trigger_spawned_reactor, Some(true));
        assert_eq!(report.no_inline_waiting, Some(true));
    }
}
