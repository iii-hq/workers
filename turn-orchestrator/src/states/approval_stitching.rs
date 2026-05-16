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

/// Build a one-line user-message string warning the model that
/// `approval::consume` capped the response and left N entries behind.
/// Returns `None` when `omitted == 0`. After T14 the recovery path is
/// the natural next-turn consume (no flush_delivered RPC anymore).
pub fn omission_summary_message(omitted: u64) -> Option<String> {
    if omitted == 0 {
        return None;
    }
    Some(format!(
        "[approval-gate] {omitted} more resolved approvals waiting to stitch — \
         they'll surface on the next turn."
    ))
}

/// Render an approval-gate `Denial` (tagged `{ kind, detail }`) as one or
/// more indented lines for the stitched LLM message. Each kind gets a
/// shape it can act on:
/// - `policy`   → "policy denied by rule <perm>:<pat>" (rule-based deny;
///   names the matched rule so the model knows what to avoid)
/// - `user_rejected`  → "user rejected this call" (no feedback)
/// - `user_corrected` → "user rejected with feedback: <text>"
///   (the high-value variant — model gets actionable correction)
/// - `state_error`    → "gate state-write failure (phase=<p>): <e>"
///   (fail-closed signal; operator-facing)
fn render_denial_lines(denial: &Value) -> Vec<String> {
    let kind = denial.get("kind").and_then(Value::as_str).unwrap_or("");
    let detail = denial.get("detail").cloned().unwrap_or(Value::Null);
    match kind {
        "policy" => {
            let perm = detail
                .get("rule_permission")
                .and_then(Value::as_str)
                .unwrap_or("");
            let pat = detail
                .get("rule_pattern")
                .and_then(Value::as_str)
                .unwrap_or("");
            vec![format!("  policy denied by rule {perm} : {pat}")]
        }
        "user_rejected" => vec!["  user rejected this call".to_string()],
        "user_corrected" => {
            let feedback = detail.get("feedback").and_then(Value::as_str).unwrap_or("");
            vec![format!("  user rejected with feedback: {feedback}")]
        }
        "state_error" => {
            let phase = detail.get("phase").and_then(Value::as_str).unwrap_or("");
            let error = detail.get("error").and_then(Value::as_str).unwrap_or("");
            vec![format!(
                "  approval gate state-write failure (phase={phase}): {error}"
            )]
        }
        other => vec![format!("  denial kind: {other}")],
    }
}

/// Render one entry from `approval::consume`. Record shape after T1/T8:
///   `{ function_call_id, function_id, args, session_id, expires_at,
///      status: "done", outcome: { kind, detail }, resolved_at }`
/// `outcome.kind` is `"executed" | "failed" | "denied" | "timed_out"`.
fn stitch_one(entry: &Value) -> String {
    let call_id = entry
        .get("function_call_id")
        .or_else(|| entry.get("call_id"))   // legacy fallback
        .and_then(Value::as_str)
        .unwrap_or("?");
    let fn_id = entry.get("function_id").and_then(Value::as_str).unwrap_or("?");

    let outcome = entry.get("outcome").cloned().unwrap_or(Value::Null);
    let outcome_kind = outcome.get("kind").and_then(Value::as_str).unwrap_or("?");
    let detail = outcome.get("detail").cloned().unwrap_or(Value::Null);

    let decision = match outcome_kind {
        "executed" | "failed" => "allow",
        "denied" => "deny",
        "timed_out" => "timeout",
        _ => "?",
    };
    let args_json = entry
        .get("args")
        .map(|v| serde_json::to_string(v).unwrap_or_default())
        .unwrap_or_default();
    let args = truncate_for_message(&args_json, STITCH_MAX_CHARS);

    let mut lines = vec![
        format!("[approval-gate] Earlier call_id {call_id} (function_id={fn_id}, args={args}):"),
        format!("  decision: {decision}"),
        format!("  outcome: {outcome_kind}"),
    ];
    match outcome_kind {
        "executed" => {
            if let Some(r) = detail.get("result") {
                let r_json = serde_json::to_string(r).unwrap_or_default();
                lines.push(format!(
                    "  result: {}",
                    truncate_for_message(&r_json, STITCH_MAX_CHARS)
                ));
            }
        }
        "failed" => {
            if let Some(e) = detail.get("error").and_then(Value::as_str) {
                lines.push(format!("  error: {e}"));
            }
        }
        "denied" => {
            if let Some(denial) = detail.get("denial") {
                lines.extend(render_denial_lines(denial));
            }
        }
        "timed_out" => { /* self-describing */ }
        _ => {}
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
        assert!(!out.ends_with('}'), "truncated JSON must not look complete");
        assert!(out.contains("… (truncated)"));
    }

    /// Build a wire-shape entry as `approval::consume` returns it: typed
    /// Record with nested `outcome: { kind, detail }`. `old_status` maps to
    /// the matching `outcome.kind` and the top-level extras are folded
    /// under `outcome.detail` where the new shape expects them.
    fn make_entry(call_id: &str, fn_id: &str, old_status: &str, extras: Value) -> Value {
        // Pull args override out of extras if present (top-level on the
        // Record), and fold the remaining keys into outcome.detail.
        let mut extras_obj = match extras {
            Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        let args_override = extras_obj.remove("args");
        let detail: Value = if extras_obj.is_empty() {
            Value::Null
        } else {
            Value::Object(extras_obj)
        };
        let kind = old_status;  // executed | failed | denied | timed_out
        json!({
            "function_call_id": call_id,
            "function_id": fn_id,
            "args": args_override.unwrap_or_else(|| json!({"path": "/tmp/x"})),
            "session_id": "sess_test",
            "expires_at": u64::MAX,
            "status": "done",
            "outcome": { "kind": kind, "detail": detail },
        })
    }

    #[test]
    fn stitch_entries_emits_one_message_per_entry() {
        let entries = vec![
            make_entry("c1", "shell::fs::write", "executed", json!({"result": {"ok": true}})),
            make_entry(
                "c2",
                "shell::fs::mkdir",
                "denied",
                json!({"denial": {"kind": "user_rejected"}}),
            ),
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
        assert!(msg.contains("outcome: executed"));
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
        assert!(msg.contains("outcome: failed"));
        assert!(msg.contains("error: EACCES"));
        assert!(!msg.contains("result:"));
    }

    #[test]
    fn stitch_entries_denied_user_rejected_renders_simple_line() {
        let entries = vec![make_entry(
            "c1",
            "shell::fs::write",
            "denied",
            json!({"denial": {"kind": "user_rejected"}}),
        )];
        let msg = &stitch_entries(&entries)[0];
        assert!(msg.contains("decision: deny"));
        assert!(msg.contains("outcome: denied"));
        assert!(msg.contains("user rejected this call"));
        assert!(!msg.contains("result:"));
        assert!(!msg.contains("error:"));
    }

    #[test]
    fn stitch_entries_denied_user_corrected_surfaces_feedback() {
        let entries = vec![make_entry(
            "c1",
            "shell::fs::write",
            "denied",
            json!({
                "denial": {
                    "kind": "user_corrected",
                    "detail": { "feedback": "use git diff instead" }
                }
            }),
        )];
        let msg = &stitch_entries(&entries)[0];
        assert!(msg.contains("user rejected with feedback: use git diff instead"));
    }

    #[test]
    fn stitch_entries_denied_policy_names_matching_rule() {
        let entries = vec![make_entry(
            "c1",
            "shell::fs::write",
            "denied",
            json!({
                "denial": {
                    "kind": "policy",
                    "detail": {
                        "rule_permission": "shell::exec",
                        "rule_pattern": "rm -rf*"
                    }
                }
            }),
        )];
        let msg = &stitch_entries(&entries)[0];
        assert!(msg.contains("policy denied by rule shell::exec : rm -rf*"));
    }

    #[test]
    fn stitch_entries_denied_state_error_surfaces_phase_and_error() {
        let entries = vec![make_entry(
            "c1",
            "shell::fs::write",
            "denied",
            json!({
                "denial": {
                    "kind": "state_error",
                    "detail": { "phase": "intercept_write_pending", "error": "kv unavailable" }
                }
            }),
        )];
        let msg = &stitch_entries(&entries)[0];
        assert!(msg.contains("approval gate state-write failure"));
        assert!(msg.contains("intercept_write_pending"));
        assert!(msg.contains("kv unavailable"));
    }

    #[test]
    fn stitch_entries_timed_out_omits_denial_block() {
        let entries = vec![make_entry(
            "c1",
            "shell::fs::write",
            "timed_out",
            json!({}),
        )];
        let msg = &stitch_entries(&entries)[0];
        assert!(msg.contains("decision: timeout"));
        assert!(msg.contains("outcome: timed_out"));
        // Timed-out records carry no denial; the status line is enough.
        assert!(!msg.contains("user rejected"));
        assert!(!msg.contains("policy denied"));
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

    #[test]
    fn omission_summary_message_zero_returns_none() {
        assert!(omission_summary_message(0).is_none());
    }

    #[test]
    fn omission_summary_message_positive_mentions_count_and_next_turn() {
        let msg = omission_summary_message(42).expect("expected Some");
        assert!(msg.starts_with("[approval-gate]"));
        assert!(msg.contains("42"));
        assert!(msg.contains("next turn"));
    }
}
