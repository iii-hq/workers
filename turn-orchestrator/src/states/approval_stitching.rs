//! Pure helpers for stitching resolved approval records into LLM turn messages.

use serde_json::Value;

/// Maximum characters of args/result JSON included verbatim in a stitched
/// system message. Anything longer is truncated with a `… (truncated)` marker.
pub const STITCH_MAX_CHARS: usize = 512;

/// Truncate `s` to at most `max` characters, appending `… (truncated)` if
/// truncation occurred. Operates on chars (not bytes) to stay safe with
/// multi-byte UTF-8.
pub fn truncate_for_message(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let head: String = chars[..max].iter().collect();
    format!("{head} … (truncated)")
}

/// Build one system-message string per resolved approval entry. Format
/// matches the spec at
/// `docs/superpowers/specs/2026-05-14-approval-gate-trigger-model-design.md`.
pub fn stitch_entries(entries: &[Value]) -> Vec<String> {
    entries.iter().map(stitch_one).collect()
}

fn stitch_one(entry: &Value) -> String {
    let call_id = entry.get("call_id").and_then(Value::as_str).unwrap_or("?");
    let fn_id = entry.get("function_id").and_then(Value::as_str).unwrap_or("?");
    let status = entry.get("status").and_then(Value::as_str).unwrap_or("?");
    let decision = match status {
        "executed" | "failed" => "allow",
        "denied" => "deny",
        "timed_out" => "timeout",
        _ => "?",
    };
    let args_json = entry.get("args")
        .map(|v| serde_json::to_string(v).unwrap_or_default())
        .unwrap_or_default();
    let args = truncate_for_message(&args_json, STITCH_MAX_CHARS);

    let mut lines = vec![
        format!("[approval-gate] Earlier call_id {call_id} (function_id={fn_id}, args={args}):"),
        format!("  decision: {decision}"),
        format!("  status: {status}"),
    ];
    if status == "executed" {
        if let Some(r) = entry.get("result") {
            let r_json = serde_json::to_string(r).unwrap_or_default();
            lines.push(format!("  result: {}", truncate_for_message(&r_json, STITCH_MAX_CHARS)));
        }
    }
    if status == "failed" {
        if let Some(e) = entry.get("error").and_then(Value::as_str) {
            lines.push(format!("  error: {e}"));
        }
    }
    if matches!(status, "denied" | "timed_out") {
        if let Some(reason) = entry.get("decision_reason").and_then(Value::as_str) {
            lines.push(format!("  reason: {reason}"));
        }
    }
    if entry.get("legacy_migrated").and_then(Value::as_bool) == Some(true) {
        lines.push("  note: legacy record migrated from pre-trigger-model gate; original result was delivered in-band when the call was made.".into());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn truncate_for_message_passthrough_when_under_limit() {
        let s = "{\"path\":\"/tmp/foo\"}";
        assert_eq!(truncate_for_message(s, STITCH_MAX_CHARS), s);
    }

    #[test]
    fn truncate_for_message_marks_truncation_when_over_limit() {
        let s = "a".repeat(STITCH_MAX_CHARS + 100);
        let out = truncate_for_message(&s, STITCH_MAX_CHARS);
        assert!(out.ends_with("… (truncated)"));
        assert!(out.len() <= STITCH_MAX_CHARS + " … (truncated)".len() + 1);
    }

    #[test]
    fn truncate_for_message_truncation_makes_json_visibly_incomplete() {
        let s = format!("{{\"k\":\"{}\"}}", "x".repeat(STITCH_MAX_CHARS));
        let out = truncate_for_message(&s, STITCH_MAX_CHARS);
        assert!(!out.ends_with("}"), "truncated JSON must not look complete");
        assert!(out.contains("… (truncated)"));
    }

    fn make_entry(call_id: &str, fn_id: &str, status: &str, extras: Value) -> Value {
        let mut v = json!({
            "call_id": call_id,
            "function_id": fn_id,
            "args": {"path": "/tmp/x"},
            "status": status,
        });
        if let Value::Object(extras) = extras {
            for (k, val) in extras {
                v[k] = val;
            }
        }
        v
    }

    #[test]
    fn stitch_entries_emits_one_message_per_entry() {
        let entries = vec![
            make_entry("c1", "shell::fs::write", "executed", json!({"result": {"ok": true}})),
            make_entry("c2", "shell::fs::mkdir", "denied",   json!({"decision_reason": "no"})),
        ];
        let out = stitch_entries(&entries);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn stitch_entries_executed_includes_result_line() {
        let entries = vec![make_entry("c1", "shell::fs::write", "executed",
            json!({"result": {"ok": true}}))];
        let msg = &stitch_entries(&entries)[0];
        assert!(msg.contains("decision: allow"));
        assert!(msg.contains("status: executed"));
        assert!(msg.contains("result:"));
        assert!(msg.contains("c1"));
        assert!(!msg.contains("error:"));
    }

    #[test]
    fn stitch_entries_failed_includes_error_line_no_result() {
        let entries = vec![make_entry("c1", "shell::fs::write", "failed",
            json!({"error": "EACCES"}))];
        let msg = &stitch_entries(&entries)[0];
        assert!(msg.contains("decision: allow"));
        assert!(msg.contains("status: failed"));
        assert!(msg.contains("error: EACCES"));
        assert!(!msg.contains("result:"));
    }

    #[test]
    fn stitch_entries_denied_omits_result_and_error() {
        let entries = vec![make_entry("c1", "shell::fs::write", "denied",
            json!({"decision_reason": "no"}))];
        let msg = &stitch_entries(&entries)[0];
        assert!(msg.contains("decision: deny"));
        assert!(msg.contains("status: denied"));
        assert!(msg.contains("reason: no"));
        assert!(!msg.contains("result:"));
        assert!(!msg.contains("error:"));
    }

    #[test]
    fn stitch_entries_timed_out_uses_timeout_decision() {
        let entries = vec![make_entry("c1", "shell::fs::write", "timed_out",
            json!({"decision_reason": "timeout"}))];
        let msg = &stitch_entries(&entries)[0];
        assert!(msg.contains("decision: timeout"));
        assert!(msg.contains("status: timed_out"));
    }

    #[test]
    fn stitch_entries_empty_input_returns_empty() {
        assert!(stitch_entries(&[]).is_empty());
    }

    #[test]
    fn stitch_entries_is_deterministic_for_same_input() {
        let entries = vec![make_entry("c1", "shell::fs::write", "executed",
            json!({"result": {"ok": true}}))];
        let a = stitch_entries(&entries);
        let b = stitch_entries(&entries);
        assert_eq!(a, b);
    }

    #[test]
    fn stitch_entries_truncates_args_over_512_chars() {
        let big = "x".repeat(600);
        let entries = vec![make_entry("c1", "shell::fs::write", "executed",
            json!({"args": {"blob": big}, "result": {"ok": true}}))];
        let msg = &stitch_entries(&entries)[0];
        assert!(msg.contains("… (truncated)"));
    }

    #[test]
    fn stitch_entries_truncates_result_over_512_chars() {
        let big = "y".repeat(600);
        let entries = vec![make_entry("c1", "shell::fs::write", "executed",
            json!({"result": {"blob": big}}))];
        let msg = &stitch_entries(&entries)[0];
        assert!(msg.contains("result:"));
        assert!(msg.contains("… (truncated)"));
    }
}
