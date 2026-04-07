use std::collections::HashMap;
use std::sync::Arc;

use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Value};

const DEFAULT_LIMIT: u64 = 100;
const HIGH_ERROR_RATE_THRESHOLD: f64 = 0.10;
const SLOW_FUNCTION_MS: f64 = 1000.0;

fn extract_function_id(span: &Value) -> Option<String> {
    span.get("function_id")
        .and_then(|v| v.as_str())
        .or_else(|| span.get("name").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
}

fn extract_duration_ms(span: &Value) -> Option<f64> {
    if let Some(ms) = span.get("duration_ms").and_then(|v| v.as_f64()) {
        return Some(ms);
    }
    if let Some(ms) = span.get("duration").and_then(|v| v.as_f64()) {
        return Some(ms);
    }
    if let Some(ns) = span.get("duration_ns").and_then(|v| v.as_f64()) {
        return Some(ns / 1_000_000.0);
    }
    None
}

fn extract_is_error(span: &Value) -> bool {
    if let Some(status) = span.get("status").and_then(|v| v.as_str()) {
        let lower = status.to_lowercase();
        if lower == "error" || lower == "failed" || lower == "err" {
            return true;
        }
    }
    if let Some(success) = span.get("success").and_then(|v| v.as_bool()) {
        return !success;
    }
    if span.get("error").and_then(|v| v.as_str()).is_some() {
        return true;
    }
    false
}

fn extract_error_message(span: &Value) -> Option<String> {
    span.get("error")
        .and_then(|v| v.as_str())
        .or_else(|| span.get("error_message").and_then(|v| v.as_str()))
        .or_else(|| span.get("status_message").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
}

fn extract_timestamp_ms(span: &Value) -> Option<i64> {
    if let Some(ts_str) = span.get("timestamp").and_then(|v| v.as_str()) {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts_str) {
            return Some(dt.timestamp_millis());
        }
    }
    if let Some(ts_ms) = span.get("timestamp").and_then(|v| v.as_i64()) {
        return Some(ts_ms);
    }
    if let Some(ts_ms) = span.get("start_time").and_then(|v| v.as_i64()) {
        return Some(ts_ms);
    }
    if let Some(ts_str) = span.get("start_time").and_then(|v| v.as_str()) {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts_str) {
            return Some(dt.timestamp_millis());
        }
    }
    None
}

struct FunctionStats {
    invocations: u64,
    total_duration_ms: f64,
    error_count: u64,
    last_error: Option<String>,
    min_timestamp_ms: Option<i64>,
    max_timestamp_ms: Option<i64>,
}

impl Default for FunctionStats {
    fn default() -> Self {
        FunctionStats {
            invocations: 0,
            total_duration_ms: 0.0,
            error_count: 0,
            last_error: None,
            min_timestamp_ms: None,
            max_timestamp_ms: None,
        }
    }
}

pub async fn handle(iii: &Arc<III>, payload: Value) -> Result<Value, IIIError> {
    let limit = payload
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_LIMIT);

    let function_filter = payload
        .get("function_filter")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let traces_response = iii
        .trigger(TriggerRequest {
            function_id: "engine::traces::list".to_string(),
            payload: json!({ "limit": limit }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
        .map_err(|e| {
            IIIError::Handler(format!("failed to fetch traces from engine: {e}"))
        })?;

    let spans: Vec<&Value> = if let Some(arr) = traces_response.as_array() {
        arr.iter().collect()
    } else if let Some(arr) = traces_response.get("spans").and_then(|v| v.as_array()) {
        arr.iter().collect()
    } else if let Some(arr) = traces_response.get("traces").and_then(|v| v.as_array()) {
        arr.iter().collect()
    } else {
        return Ok(json!({
            "summary": {
                "total_spans": 0,
                "unique_functions": 0,
                "time_range": null,
                "error_rate": 0.0
            },
            "slowest_functions": [],
            "most_active": [],
            "errors": [],
            "insights": ["No trace data available — engine returned an unexpected format"]
        }));
    };

    let mut stats_map: HashMap<String, FunctionStats> = HashMap::new();
    let mut global_min_ts: Option<i64> = None;
    let mut global_max_ts: Option<i64> = None;
    let mut total_errors: u64 = 0;

    for span in &spans {
        let fid = match extract_function_id(span) {
            Some(id) => id,
            None => continue,
        };

        if let Some(ref filter) = function_filter {
            if !fid.contains(filter.as_str()) {
                continue;
            }
        }

        let stats = stats_map.entry(fid).or_default();
        stats.invocations += 1;

        if let Some(dur) = extract_duration_ms(span) {
            stats.total_duration_ms += dur;
        }

        let is_err = extract_is_error(span);
        if is_err {
            stats.error_count += 1;
            total_errors += 1;
            if let Some(msg) = extract_error_message(span) {
                stats.last_error = Some(msg);
            }
        }

        if let Some(ts) = extract_timestamp_ms(span) {
            stats.min_timestamp_ms = Some(
                stats.min_timestamp_ms.map_or(ts, |existing| existing.min(ts)),
            );
            stats.max_timestamp_ms = Some(
                stats.max_timestamp_ms.map_or(ts, |existing| existing.max(ts)),
            );
            global_min_ts = Some(global_min_ts.map_or(ts, |existing| existing.min(ts)));
            global_max_ts = Some(global_max_ts.map_or(ts, |existing| existing.max(ts)));
        }
    }

    let total_counted: u64 = stats_map.values().map(|s| s.invocations).sum();
    let unique_functions = stats_map.len();
    let overall_error_rate = if total_counted > 0 {
        total_errors as f64 / total_counted as f64
    } else {
        0.0
    };

    let time_range = match (global_min_ts, global_max_ts) {
        (Some(min_ts), Some(max_ts)) => {
            let from = chrono::DateTime::from_timestamp_millis(min_ts)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| min_ts.to_string());
            let to = chrono::DateTime::from_timestamp_millis(max_ts)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| max_ts.to_string());
            json!({ "from": from, "to": to })
        }
        _ => json!(null),
    };

    let mut function_entries: Vec<(&String, &FunctionStats)> = stats_map.iter().collect();

    function_entries.sort_by(|a, b| {
        let avg_a = if a.1.invocations > 0 {
            a.1.total_duration_ms / a.1.invocations as f64
        } else {
            0.0
        };
        let avg_b = if b.1.invocations > 0 {
            b.1.total_duration_ms / b.1.invocations as f64
        } else {
            0.0
        };
        avg_b.partial_cmp(&avg_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    let slowest_functions: Vec<Value> = function_entries
        .iter()
        .take(5)
        .filter(|(_, s)| s.invocations > 0)
        .map(|(fid, s)| {
            let avg = s.total_duration_ms / s.invocations as f64;
            json!({
                "function_id": fid,
                "avg_duration_ms": (avg * 100.0).round() / 100.0,
                "invocations": s.invocations
            })
        })
        .collect();

    function_entries.sort_by(|a, b| b.1.invocations.cmp(&a.1.invocations));

    let most_active: Vec<Value> = function_entries
        .iter()
        .take(5)
        .map(|(fid, s)| {
            let avg = if s.invocations > 0 {
                s.total_duration_ms / s.invocations as f64
            } else {
                0.0
            };
            json!({
                "function_id": fid,
                "invocations": s.invocations,
                "avg_duration_ms": (avg * 100.0).round() / 100.0
            })
        })
        .collect();

    let errors: Vec<Value> = stats_map
        .iter()
        .filter(|(_, s)| s.error_count > 0)
        .map(|(fid, s)| {
            let rate = s.error_count as f64 / s.invocations as f64;
            json!({
                "function_id": fid,
                "error_count": s.error_count,
                "last_error": s.last_error,
                "error_rate": (rate * 1000.0).round() / 1000.0
            })
        })
        .collect();

    let mut insights: Vec<String> = Vec::new();

    for (fid, s) in &stats_map {
        let error_rate = if s.invocations > 0 {
            s.error_count as f64 / s.invocations as f64
        } else {
            0.0
        };
        if error_rate >= HIGH_ERROR_RATE_THRESHOLD && s.invocations >= 3 {
            insights.push(format!(
                "{} has {:.0}% error rate — investigate",
                fid,
                error_rate * 100.0
            ));
        }
    }

    if let Some((fid, s)) = function_entries.first() {
        insights.push(format!(
            "{} is the most active function ({} calls)",
            fid, s.invocations
        ));
    }

    for (fid, s) in &stats_map {
        if s.invocations > 0 {
            let avg = s.total_duration_ms / s.invocations as f64;
            if avg >= SLOW_FUNCTION_MS {
                insights.push(format!(
                    "{} averages {:.1}s — consider optimization",
                    fid,
                    avg / 1000.0
                ));
            }
        }
    }

    if insights.is_empty() && !stats_map.is_empty() {
        insights.push("All functions operating within normal parameters".to_string());
    }

    Ok(json!({
        "summary": {
            "total_spans": total_counted,
            "unique_functions": unique_functions,
            "time_range": time_range,
            "error_rate": (overall_error_rate * 1000.0).round() / 1000.0
        },
        "slowest_functions": slowest_functions,
        "most_active": most_active,
        "errors": errors,
        "insights": insights
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_function_id_primary() {
        let span = json!({ "function_id": "eval::ingest", "duration_ms": 50 });
        assert_eq!(extract_function_id(&span), Some("eval::ingest".to_string()));
    }

    #[test]
    fn test_extract_function_id_fallback() {
        let span = json!({ "name": "eval::metrics" });
        assert_eq!(extract_function_id(&span), Some("eval::metrics".to_string()));
    }

    #[test]
    fn test_extract_function_id_none() {
        let span = json!({ "duration_ms": 50 });
        assert_eq!(extract_function_id(&span), None);
    }

    #[test]
    fn test_extract_duration_ms_primary() {
        let span = json!({ "duration_ms": 123 });
        assert_eq!(extract_duration_ms(&span), Some(123.0));
    }

    #[test]
    fn test_extract_duration_ms_ns() {
        let span = json!({ "duration_ns": 5_000_000 });
        assert_eq!(extract_duration_ms(&span), Some(5.0));
    }

    #[test]
    fn test_extract_duration_ms_fallback() {
        let span = json!({ "duration": 42.5 });
        assert_eq!(extract_duration_ms(&span), Some(42.5));
    }

    #[test]
    fn test_extract_duration_ms_none() {
        let span = json!({ "function_id": "x" });
        assert_eq!(extract_duration_ms(&span), None);
    }

    #[test]
    fn test_extract_is_error_status() {
        assert!(extract_is_error(&json!({ "status": "error" })));
        assert!(extract_is_error(&json!({ "status": "ERROR" })));
        assert!(extract_is_error(&json!({ "status": "Failed" })));
        assert!(!extract_is_error(&json!({ "status": "ok" })));
    }

    #[test]
    fn test_extract_is_error_success_field() {
        assert!(extract_is_error(&json!({ "success": false })));
        assert!(!extract_is_error(&json!({ "success": true })));
    }

    #[test]
    fn test_extract_is_error_error_field() {
        assert!(extract_is_error(&json!({ "error": "timeout" })));
        assert!(!extract_is_error(&json!({})));
    }

    #[test]
    fn test_extract_error_message() {
        assert_eq!(
            extract_error_message(&json!({ "error": "timeout" })),
            Some("timeout".to_string())
        );
        assert_eq!(
            extract_error_message(&json!({ "error_message": "not found" })),
            Some("not found".to_string())
        );
        assert_eq!(extract_error_message(&json!({})), None);
    }

    #[test]
    fn test_extract_timestamp_ms_rfc3339() {
        let span = json!({ "timestamp": "2026-01-01T00:00:00Z" });
        let ts = extract_timestamp_ms(&span);
        assert!(ts.is_some());
        assert!(ts.unwrap() > 0);
    }

    #[test]
    fn test_extract_timestamp_ms_integer() {
        let span = json!({ "timestamp": 1700000000000_i64 });
        assert_eq!(extract_timestamp_ms(&span), Some(1700000000000));
    }

    #[test]
    fn test_extract_timestamp_ms_none() {
        let span = json!({ "function_id": "x" });
        assert_eq!(extract_timestamp_ms(&span), None);
    }
}
