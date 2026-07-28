//! The `github::called` trigger type this worker emits, the subscriber
//! registry behind it, and the summarizers that turn a call's argv + result
//! into the short human strings the console activity feed renders.
//!
//! After every `github::*` function runs, [`run_and_emit`] fires a
//! fire-and-forget `github::called` event (`TriggerAction::Void`) carrying the
//! call and a bounded result summary. Subscribers (the console page) bind a
//! trigger instance of the type with the standard two-step pattern; the engine
//! routes each registration through [`CalledTriggerHandler`], which stashes the
//! subscriber in [`SubscriberSet`]. Emission is best-effort: a slow or absent
//! subscriber never blocks or fails the real call — the mirror of the
//! `iii-directory` / `browser` worker fan-out.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use serde::Serialize;
use serde_json::{json, Map, Value};

/// The trigger type the console subscribes to for the activity feed.
pub const CALLED: &str = "github::called";

/// Result/args summaries are bounded so a huge diff or body can never bloat the
/// event past what the feed row needs.
const MAX_SUMMARY: usize = 512;
const MAX_ARGS: usize = 256;

/// The result PREVIEW carried alongside the one-line summary is budgeted so the
/// event stays small no matter how big the underlying gh output was.
/// List previews keep at most this many items (with the true total recorded);
/// object/text/diff/outcome previews are byte-capped.
const PREVIEW_MAX_ITEMS: usize = 12;
const PREVIEW_MAX_BYTES: usize = 6144;
/// Per-item value cap for a projected list entry (an `author`/`repository`
/// object stays whole; a rogue long field is trimmed).
const PREVIEW_ITEM_VALUE_BYTES: usize = 512;
/// Long top-level strings on a projected object (a PR body, release notes) are
/// trimmed to this before the object is re-measured against the byte budget.
const PREVIEW_STRING_BYTES: usize = 1024;
/// Each stream of an outcome preview is capped independently so a chatty
/// stdout AND stderr together still fit the budget.
const PREVIEW_STREAM_BYTES: usize = PREVIEW_MAX_BYTES / 2;

/// One `github::called` event: the call that ran plus a short, human result
/// summary. Serialized as the trigger payload; the console renders it directly.
#[derive(Debug, Clone, Serialize)]
pub struct CalledEvent {
    /// The `github::*` function id that ran.
    pub function_id: String,
    /// One-line echo of the salient input (repo, number, query), bodies and the
    /// `--json` field list stripped.
    pub args_summary: String,
    /// `owner/name` when the call named one, else null.
    pub repo: Option<String>,
    /// True when the function returned Ok.
    pub ok: bool,
    /// Wall-clock duration of the wrapped handler, in ms.
    pub duration_ms: u64,
    /// Short human string derived from the result (e.g. "3 pull requests") —
    /// the collapsed row header.
    pub result_summary: String,
    /// Which renderer the UI should use for [`Self::result_preview`], derived
    /// from the response envelope + function id:
    /// `"list" | "object" | "text" | "diff" | "outcome"`.
    pub kind: String,
    /// The ACTUAL result, budgeted small (see the `PREVIEW_*` caps): a
    /// projected + truncated slice of what the call returned so the feed shows
    /// the substance, not just a count. `null` when there is nothing useful.
    pub result_preview: Value,
    /// RFC 3339 UTC timestamp of completion.
    pub timestamp: String,
}

/// Thread-safe subscriber registry keyed by trigger-instance id. Cloned into
/// the [`CalledTriggerHandler`] (mutates on register/unregister) and the
/// [`CalledEmitter`] fan-out (iterates read-only).
#[derive(Clone, Default)]
pub struct SubscriberSet {
    inner: Arc<Mutex<HashMap<String, String>>>,
}

impl SubscriberSet {
    pub fn new() -> Self {
        Self::default()
    }

    fn insert(&self, id: String, function_id: String) {
        self.lock().insert(id, function_id);
    }

    fn remove(&self, id: &str) {
        self.lock().remove(id);
    }

    /// Snapshot of the current subscriber `function_id`s so the mutex is never
    /// held across an await.
    pub fn function_ids(&self) -> Vec<String> {
        self.lock().values().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

struct CalledTriggerHandler {
    subscribers: SubscriberSet,
}

#[async_trait]
impl TriggerHandler for CalledTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(
            trigger_type = CALLED,
            id = %config.id,
            function_id = %config.function_id,
            "trigger subscription registered"
        );
        self.subscribers.insert(config.id, config.function_id);
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(trigger_type = CALLED, id = %config.id, "trigger subscription unregistered");
        self.subscribers.remove(&config.id);
        Ok(())
    }
}

/// Fires `github::called` events to every current subscriber. Holds the client
/// and the shared subscriber set; cheap to clone into each function handler.
#[derive(Clone)]
pub struct CalledEmitter {
    iii: Arc<IIIClient>,
    subscribers: SubscriberSet,
}

impl CalledEmitter {
    pub fn new(iii: Arc<IIIClient>, subscribers: SubscriberSet) -> Self {
        Self { iii, subscribers }
    }

    /// Fan `event` out to every subscriber with `TriggerAction::Void`
    /// (fire-and-forget). No subscribers ⇒ no work. Delivery failures are
    /// logged and swallowed — the real call already returned.
    pub async fn emit(&self, event: CalledEvent) {
        let targets = self.subscribers.function_ids();
        if targets.is_empty() {
            return;
        }
        let payload = match serde_json::to_value(&event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "github::called payload failed to serialize");
                return;
            }
        };
        for function_id in targets {
            let res = self
                .iii
                .trigger(TriggerRequest {
                    function_id: function_id.clone(),
                    payload: payload.clone(),
                    action: Some(TriggerAction::Void),
                    timeout_ms: None,
                })
                .await;
            if let Err(e) = res {
                tracing::warn!(function_id = %function_id, error = %e, "github::called fan-out failed");
            }
        }
    }
}

/// Register the `github::called` trigger type with the engine and return the
/// emitter wired to its subscriber set. Call before registering the functions
/// so the emitter is ready to thread through them.
pub fn register_called_trigger(iii: &Arc<IIIClient>) -> CalledEmitter {
    let subscribers = SubscriberSet::new();
    let _ = iii.register_trigger_type(RegisterTriggerType::new(
        CALLED,
        "Fires after each github function runs; carries the call + a short result summary.",
        CalledTriggerHandler {
            subscribers: subscribers.clone(),
        },
    ));
    tracing::info!(trigger_type = CALLED, "registered trigger type");
    CalledEmitter::new(iii.clone(), subscribers)
}

/// Run `fut` (the real handler), measure it, then emit a `github::called`
/// event. The wrapped handler's result is returned unchanged; the emit never
/// affects it (fire-and-forget, log-and-ignore on failure).
pub async fn run_and_emit<Resp, Fut>(
    emitter: &CalledEmitter,
    function_id: &'static str,
    args_summary: String,
    repo: Option<String>,
    fut: Fut,
) -> Result<Resp, Error>
where
    Resp: Serialize,
    Fut: Future<Output = Result<Resp, Error>>,
{
    let started = std::time::Instant::now();
    let result = fut.await;
    let duration_ms = started.elapsed().as_millis() as u64;
    let (ok, result_summary, kind, result_preview) = match &result {
        Ok(resp) => {
            let value = serde_json::to_value(resp).unwrap_or(Value::Null);
            let (kind, preview) = preview(function_id, &value);
            (true, summarize(function_id, &value), kind, preview)
        }
        Err(e) => {
            let msg = e.to_string();
            let (text, truncated) = truncate_bytes(&msg, PREVIEW_MAX_BYTES);
            (
                false,
                truncate(&msg, MAX_SUMMARY),
                "text".to_string(),
                json!({ "text": text, "truncated": truncated }),
            )
        }
    };
    emitter
        .emit(CalledEvent {
            function_id: function_id.to_string(),
            args_summary,
            repo,
            ok,
            duration_ms,
            result_summary,
            kind,
            result_preview,
            timestamp: now_rfc3339(),
        })
        .await;
    result
}

/// A SHORT human summary of a function's serialized result. Dispatches on the
/// response envelope (`{ value }` / `{ output }` / `{ diff }` / a `GhOutcome`)
/// and falls back to a bounded first-line/bytes description.
pub fn summarize(function_id: &str, result: &Value) -> String {
    let raw = match result {
        Value::Object(m) if m.contains_key("output") => {
            let out = m.get("output").and_then(Value::as_str).unwrap_or("");
            let line = first_line(out);
            if line.is_empty() {
                "ok".to_string()
            } else {
                line
            }
        }
        Value::Object(m) if m.contains_key("diff") => {
            let diff = m.get("diff").and_then(Value::as_str).unwrap_or("");
            let truncated = m.get("truncated").and_then(Value::as_bool).unwrap_or(false);
            let mut s = format!("{} diff", human_bytes(diff.len()));
            if truncated {
                s.push_str(", truncated");
            }
            s
        }
        Value::Object(m) if m.contains_key("value") => {
            summarize_value(function_id, m.get("value").unwrap_or(&Value::Null))
        }
        Value::Object(m) if m.contains_key("exit_code") || m.contains_key("stdout") => {
            summarize_outcome(m)
        }
        _ => first_line(&result.to_string()),
    };
    truncate(&raw, MAX_SUMMARY)
}

/// The `kind` discriminator + the budgeted `result_preview` for an event.
/// Dispatches on the SAME response envelope as [`summarize`] so the UI can pick
/// a renderer without re-parsing:
///
/// - `{ output }`  → `"text"`,   `{ text, truncated }` (byte-capped)
/// - `{ diff }`    → `"diff"`,   `{ diff, truncated }` (first ~6 KB kept)
/// - `{ value: [] }`→ `"list"`,  `{ items: [projected…], total }` (first N kept)
/// - `{ value: {} }`→ `"object"`, the object whole (under budget) or top-level
///   keys projected
/// - `{ exit_code|stdout }` → `"outcome"`, `{ exit_code, stdout, stderr, … }`
/// - anything else → `"object"`, the value projected to fit the byte budget
pub fn preview(function_id: &str, result: &Value) -> (String, Value) {
    match result {
        Value::Object(m) if m.contains_key("output") => {
            let out = m.get("output").and_then(Value::as_str).unwrap_or("");
            let (text, truncated) = truncate_bytes(out, PREVIEW_MAX_BYTES);
            (
                "text".to_string(),
                json!({ "text": text, "truncated": truncated }),
            )
        }
        Value::Object(m) if m.contains_key("diff") => {
            let diff = m.get("diff").and_then(Value::as_str).unwrap_or("");
            let already = m.get("truncated").and_then(Value::as_bool).unwrap_or(false);
            let (text, cut) = truncate_bytes(diff, PREVIEW_MAX_BYTES);
            (
                "diff".to_string(),
                json!({ "diff": text, "truncated": already || cut }),
            )
        }
        Value::Object(m) if m.contains_key("value") => match m.get("value").unwrap_or(&Value::Null)
        {
            Value::Array(items) => ("list".to_string(), preview_list(function_id, items)),
            Value::Null => ("object".to_string(), Value::Null),
            other => ("object".to_string(), project_object(other)),
        },
        Value::Object(m) if m.contains_key("exit_code") || m.contains_key("stdout") => {
            ("outcome".to_string(), preview_outcome(m))
        }
        Value::Null => ("object".to_string(), Value::Null),
        other => ("object".to_string(), project_object(other)),
    }
}

/// `{ items: [first N projected], total }` — keep the true length so the UI can
/// show "showing 12 of 70", and project each kept item down to its salient
/// display keys (per [`keys_for`]) so the payload stays small.
fn preview_list(function_id: &str, items: &[Value]) -> Value {
    let allow = keys_for(function_id);
    let kept: Vec<Value> = items
        .iter()
        .take(PREVIEW_MAX_ITEMS)
        .map(|item| project_item(item, allow))
        .collect();
    json!({ "items": kept, "total": items.len() })
}

/// Project one list item to the salient keys. With an allowlist, keep exactly
/// those keys (each bounded); without one, keep top-level scalar keys and drop
/// the blobs (nested arrays/objects, long strings).
fn project_item(item: &Value, allow: &[&str]) -> Value {
    let Value::Object(map) = item else {
        return item.clone();
    };
    let mut out = Map::new();
    if allow.is_empty() {
        for (k, v) in map {
            let keep = match v {
                Value::String(s) => s.len() <= PREVIEW_ITEM_VALUE_BYTES,
                Value::Array(_) | Value::Object(_) => false,
                _ => true,
            };
            if keep {
                out.insert(k.clone(), v.clone());
            }
        }
    } else {
        for &k in allow {
            if let Some(v) = map.get(k) {
                out.insert(k.to_string(), bound_value(v));
            }
        }
    }
    Value::Object(out)
}

/// Keep a projected value, trimming a rogue long string / oversized nested
/// value so a single item can't blow the budget. Small objects (an `author`)
/// pass through whole.
fn bound_value(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(truncate_bytes(s, PREVIEW_ITEM_VALUE_BYTES).0),
        other if value_bytes(other) > PREVIEW_ITEM_VALUE_BYTES => {
            Value::String(format!("… ({} bytes elided)", value_bytes(other)))
        }
        other => other.clone(),
    }
}

/// An object result: keep it whole when it fits the byte budget, else project
/// top-level keys — scalars kept, long strings trimmed, nested blobs dropped.
fn project_object(value: &Value) -> Value {
    if value_bytes(value) <= PREVIEW_MAX_BYTES {
        return value.clone();
    }
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                match v {
                    Value::String(s) => {
                        out.insert(
                            k.clone(),
                            Value::String(truncate_bytes(s, PREVIEW_STRING_BYTES).0),
                        );
                    }
                    Value::Array(_) | Value::Object(_) => {}
                    other => {
                        out.insert(k.clone(), other.clone());
                    }
                }
            }
            Value::Object(out)
        }
        Value::String(s) => Value::String(truncate_bytes(s, PREVIEW_MAX_BYTES).0),
        other => other.clone(),
    }
}

/// A `GhOutcome` preview: exit code / kill state plus byte-capped stdout+stderr.
fn preview_outcome(m: &Map<String, Value>) -> Value {
    let (stdout, out_cut) = truncate_bytes(
        m.get("stdout").and_then(Value::as_str).unwrap_or(""),
        PREVIEW_STREAM_BYTES,
    );
    let (stderr, err_cut) = truncate_bytes(
        m.get("stderr").and_then(Value::as_str).unwrap_or(""),
        PREVIEW_STREAM_BYTES,
    );
    let already = m
        .get("stdout_truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || m.get("stderr_truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    json!({
        "exit_code": m.get("exit_code").cloned().unwrap_or(Value::Null),
        "timed_out": m.get("timed_out").and_then(Value::as_bool).unwrap_or(false),
        "stdout": stdout,
        "stderr": stderr,
        "truncated": already || out_cut || err_cut,
    })
}

/// The salient display keys to keep for each list-returning function, dropping
/// the noise (labels, timestamps, text-match blobs). An empty slice means
/// "no allowlist" — [`project_item`] falls back to keeping scalar keys.
fn keys_for(function_id: &str) -> &'static [&'static str] {
    match function_id {
        "github::pr::list" => &["number", "title", "state", "url", "author", "isDraft"],
        "github::search::prs" => &[
            "number",
            "title",
            "state",
            "url",
            "repository",
            "author",
            "isDraft",
        ],
        "github::issue::list" => &["number", "title", "state", "url", "author"],
        "github::search::issues" => &["number", "title", "state", "url", "repository", "author"],
        "github::pr::checks" => &["name", "state", "bucket", "workflow", "link"],
        "github::repo::list" => &[
            "nameWithOwner",
            "name",
            "description",
            "url",
            "stargazerCount",
            "primaryLanguage",
            "visibility",
        ],
        "github::search::repos" => &[
            "fullName",
            "description",
            "url",
            "stargazersCount",
            "language",
            "visibility",
        ],
        "github::run::list" => &[
            "databaseId",
            "displayTitle",
            "workflowName",
            "name",
            "status",
            "conclusion",
            "headBranch",
            "event",
            "url",
        ],
        "github::workflow::list" => &["id", "name", "path", "state"],
        "github::release::list" => &["tagName", "name", "isLatest", "isPrerelease", "isDraft"],
        "github::search::code" => &["path", "repository", "sha", "url"],
        _ => &[],
    }
}

/// Byte length of a value's compact JSON encoding.
fn value_bytes(v: &Value) -> usize {
    serde_json::to_string(v).map(|s| s.len()).unwrap_or(0)
}

/// Truncate `s` to at most `max` bytes on a char boundary. Returns the bounded
/// string and whether anything was dropped.
fn truncate_bytes(s: &str, max: usize) -> (String, bool) {
    if s.len() <= max {
        return (s.to_string(), false);
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}

/// Count arrays (with a function-derived noun), name a single object, else a
/// short description of the `{ value }` payload.
fn summarize_value(function_id: &str, value: &Value) -> String {
    match value {
        Value::Array(items) => {
            format!(
                "{} {}",
                items.len(),
                pluralize(noun_for(function_id), items.len())
            )
        }
        Value::Null => "no data".to_string(),
        Value::Object(_) => format!("1 {}", noun_for(function_id)),
        other => first_line(&other.to_string()),
    }
}

/// `github::exec`'s `GhOutcome`: exit code + captured stdout size, or the
/// timeout/kill state.
fn summarize_outcome(m: &Map<String, Value>) -> String {
    let timed_out = m.get("timed_out").and_then(Value::as_bool).unwrap_or(false);
    if timed_out {
        return "timed out".to_string();
    }
    let bytes = m.get("stdout").and_then(Value::as_str).map_or(0, str::len);
    match m.get("exit_code").and_then(Value::as_i64) {
        Some(code) => format!("exit {code}, {} out", human_bytes(bytes)),
        None => "killed".to_string(),
    }
}

/// The singular noun a list/search function returns, for count summaries.
fn noun_for(function_id: &str) -> &'static str {
    if function_id.contains("::pr::") || function_id.ends_with("::prs") {
        "pull request"
    } else if function_id.contains("issue") {
        "issue"
    } else if function_id.contains("run") {
        "run"
    } else if function_id.contains("release") {
        "release"
    } else if function_id.contains("repo") {
        "repository"
    } else if function_id.contains("workflow") {
        "workflow"
    } else if function_id.ends_with("::code") {
        "code result"
    } else {
        "result"
    }
}

fn pluralize(noun: &str, n: usize) -> String {
    if n == 1 {
        return noun.to_string();
    }
    match noun.strip_suffix('y') {
        Some(stem) => format!("{stem}ies"),
        None => format!("{noun}s"),
    }
}

/// A one-line echo of the salient gh input: the subcommand words, `-R` repo,
/// number, and query flags. The `--json` field list is dropped (plumbing) and
/// body/field values are redacted — gh keeps the auth token in the env, never
/// in argv, so there is nothing secret in the words themselves.
pub fn summarize_args(argv: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut iter = argv.iter();
    while let Some(tok) = iter.next() {
        match tok.as_str() {
            // Plumbing noise: drop the flag and its (long) field list.
            "--json" => {
                iter.next();
            }
            // Keep the field key, redact its value (may carry user secrets).
            "-f" | "-F" | "--field" | "--raw-field" => {
                out.push(tok.clone());
                if let Some(v) = iter.next() {
                    let key = v.split('=').next().unwrap_or("");
                    out.push(format!("{key}=…"));
                }
            }
            // Bodies/inputs are long free text — echo the flag, redact the value.
            "--body" | "--body-file" | "--input" => {
                out.push(tok.clone());
                if iter.next().is_some() {
                    out.push("…".to_string());
                }
            }
            _ => out.push(tok.clone()),
        }
    }
    truncate(&out.join(" "), MAX_ARGS)
}

/// `owner/name` from a gh argv (`-R` / `--repo`), if present.
pub fn repo_from_args(argv: &[String]) -> Option<String> {
    let mut iter = argv.iter();
    while let Some(tok) = iter.next() {
        if tok == "-R" || tok == "--repo" {
            return iter.next().cloned();
        }
    }
    None
}

fn first_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

fn human_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

/// Char-bounded truncation with an ellipsis; keeps the result ≤ `max` chars.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// RFC 3339 UTC timestamp for "now" — no chrono dependency; the calendar math
/// is Howard Hinnant's `civil_from_days`.
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, mo, d, h, mi, s) = civil_from_epoch(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

fn civil_from_epoch(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let h = (rem / 3600) as u32;
    let mi = ((rem % 3600) / 60) as u32;
    let s = (rem % 60) as u32;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summarize_counts_list_results_with_a_noun() {
        assert_eq!(
            summarize("github::pr::list", &json!({ "value": [1, 2, 3] })),
            "3 pull requests"
        );
        assert_eq!(
            summarize("github::repo::list", &json!({ "value": [{}] })),
            "1 repository"
        );
        assert_eq!(
            summarize("github::search::issues", &json!({ "value": [] })),
            "0 issues"
        );
    }

    #[test]
    fn summarize_names_a_single_object() {
        assert_eq!(
            summarize("github::pr::view", &json!({ "value": { "number": 7 } })),
            "1 pull request"
        );
    }

    #[test]
    fn summarize_uses_first_line_for_text_output() {
        assert_eq!(
            summarize(
                "github::issue::create",
                &json!({ "output": "https://github.com/o/r/issues/42\n" })
            ),
            "https://github.com/o/r/issues/42"
        );
        assert_eq!(
            summarize("github::pr::edit", &json!({ "output": "" })),
            "ok"
        );
    }

    #[test]
    fn summarize_describes_diff_and_outcome() {
        assert_eq!(
            summarize(
                "github::pr::diff",
                &json!({ "diff": "abcd", "truncated": true })
            ),
            "4 B diff, truncated"
        );
        assert_eq!(
            summarize(
                "github::exec",
                &json!({ "stdout": "hi", "exit_code": 0, "timed_out": false })
            ),
            "exit 0, 2 B out"
        );
    }

    #[test]
    fn summarize_is_bounded() {
        let big = "x".repeat(5000);
        let s = summarize("github::pr::edit", &json!({ "output": big }));
        assert!(s.chars().count() <= MAX_SUMMARY);
    }

    #[test]
    fn preview_list_truncates_to_n_and_records_the_true_total() {
        let items: Vec<Value> = (0..70)
            .map(|n| json!({ "number": n, "title": format!("pr {n}") }))
            .collect();
        let (kind, pv) = preview("github::pr::list", &json!({ "value": items }));
        assert_eq!(kind, "list");
        assert_eq!(pv["total"], json!(70));
        assert_eq!(pv["items"].as_array().unwrap().len(), PREVIEW_MAX_ITEMS);
        // First kept item survived intact.
        assert_eq!(pv["items"][0]["number"], json!(0));
    }

    #[test]
    fn preview_list_projects_to_the_namespace_allowlist() {
        // A PR list item carries labels + timestamps the feed does not need;
        // the allowlist keeps the salient keys (incl. the author object) and
        // drops the rest.
        let item = json!({
            "number": 7,
            "title": "fix",
            "state": "OPEN",
            "url": "https://x/7",
            "author": { "login": "octocat", "id": "abc" },
            "isDraft": false,
            "labels": [{ "name": "bug" }, { "name": "p1" }],
            "createdAt": "2026-01-01T00:00:00Z",
            "updatedAt": "2026-01-02T00:00:00Z",
        });
        let (_, pv) = preview("github::pr::list", &json!({ "value": [item] }));
        let projected = &pv["items"][0];
        assert_eq!(projected["number"], json!(7));
        assert_eq!(projected["title"], json!("fix"));
        assert_eq!(projected["author"]["login"], json!("octocat"));
        assert!(projected.get("labels").is_none(), "labels dropped");
        assert!(projected.get("createdAt").is_none(), "timestamp dropped");
    }

    #[test]
    fn preview_list_default_projection_keeps_scalars_drops_blobs() {
        // github::api (contents) has no allowlist: keep scalar keys, drop the
        // nested `_links` blob.
        let item = json!({
            "name": "main.rs",
            "path": "src/main.rs",
            "type": "file",
            "size": 1234,
            "_links": { "self": "https://x", "git": "https://y" },
        });
        let (kind, pv) = preview("github::api", &json!({ "value": [item] }));
        assert_eq!(kind, "list");
        let projected = &pv["items"][0];
        assert_eq!(projected["name"], json!("main.rs"));
        assert_eq!(projected["type"], json!("file"));
        assert_eq!(projected["size"], json!(1234));
        assert!(projected.get("_links").is_none(), "nested blob dropped");
    }

    #[test]
    fn preview_object_kept_whole_when_small_projected_when_big() {
        let small = json!({ "value": { "number": 7, "title": "ok" } });
        let (kind, pv) = preview("github::pr::view", &small);
        assert_eq!(kind, "object");
        assert_eq!(pv["title"], json!("ok"));

        let big = json!({
            "value": {
                "number": 7,
                "body": "x".repeat(20_000),
                "comments": vec![json!({ "b": "y".repeat(4000) }); 5],
            }
        });
        let (_, pv) = preview("github::pr::view", &big);
        assert!(value_bytes(&pv) <= PREVIEW_MAX_BYTES);
        assert_eq!(pv["number"], json!(7));
        assert!(pv.get("comments").is_none(), "nested blob dropped");
        assert!(
            pv["body"].as_str().unwrap().len() <= PREVIEW_STRING_BYTES,
            "long string trimmed"
        );
    }

    #[test]
    fn preview_text_and_diff_are_byte_capped() {
        let (kind, pv) = preview("github::pr::edit", &json!({ "output": "x".repeat(20_000) }));
        assert_eq!(kind, "text");
        assert!(pv["text"].as_str().unwrap().len() <= PREVIEW_MAX_BYTES);
        assert_eq!(pv["truncated"], json!(true));

        let (kind, pv) = preview(
            "github::pr::diff",
            &json!({ "diff": "+".repeat(20_000), "truncated": false }),
        );
        assert_eq!(kind, "diff");
        assert!(pv["diff"].as_str().unwrap().len() <= PREVIEW_MAX_BYTES);
        assert_eq!(pv["truncated"], json!(true));

        // The wrapper's own truncation flag is preserved even when the preview
        // itself fit.
        let (_, pv) = preview(
            "github::pr::diff",
            &json!({ "diff": "small", "truncated": true }),
        );
        assert_eq!(pv["truncated"], json!(true));
    }

    #[test]
    fn preview_outcome_caps_each_stream_and_carries_exit() {
        let (kind, pv) = preview(
            "github::exec",
            &json!({
                "stdout": "a".repeat(20_000),
                "stderr": "boom",
                "exit_code": 3,
                "timed_out": false,
            }),
        );
        assert_eq!(kind, "outcome");
        assert_eq!(pv["exit_code"], json!(3));
        assert_eq!(pv["stderr"], json!("boom"));
        assert!(pv["stdout"].as_str().unwrap().len() <= PREVIEW_STREAM_BYTES);
        assert_eq!(pv["truncated"], json!(true));
    }

    #[test]
    fn preview_null_value_is_object_null() {
        let (kind, pv) = preview("github::pr::edit", &json!({ "value": Value::Null }));
        assert_eq!(kind, "object");
        assert_eq!(pv, Value::Null);
    }

    #[test]
    fn args_summary_redacts_bodies_and_drops_json_fields() {
        let argv: Vec<String> = [
            "pr",
            "list",
            "-R",
            "o/r",
            "--json",
            "number,title",
            "--state",
            "open",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(summarize_args(&argv), "pr list -R o/r --state open");

        let create: Vec<String> = [
            "pr",
            "create",
            "-R",
            "o/r",
            "--title",
            "t",
            "--body",
            "secret body",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            summarize_args(&create),
            "pr create -R o/r --title t --body …"
        );
    }

    #[test]
    fn repo_extracted_from_argv() {
        let argv: Vec<String> = ["pr", "list", "-R", "iii-hq/workers"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(repo_from_args(&argv), Some("iii-hq/workers".to_string()));
        assert_eq!(repo_from_args(&["api".into(), "rate_limit".into()]), None);
    }

    #[test]
    fn rfc3339_matches_known_epochs() {
        assert_eq!(civil_from_epoch(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil_from_epoch(1_700_000_000), (2023, 11, 14, 22, 13, 20));
    }

    #[tokio::test]
    async fn emitting_does_not_change_the_wrapped_return() {
        #[derive(Serialize, PartialEq, Debug)]
        struct Out {
            output: String,
        }
        // No subscribers ⇒ emit is a no-op and never touches the client.
        let emitter = CalledEmitter::new(
            Arc::new(IIIClient::new("ws://127.0.0.1:1")),
            SubscriberSet::new(),
        );

        let ok = run_and_emit(
            &emitter,
            "github::pr::create",
            "pr create".into(),
            None,
            async {
                Ok::<_, Error>(Out {
                    output: "hello".to_string(),
                })
            },
        )
        .await;
        assert_eq!(
            ok.unwrap(),
            Out {
                output: "hello".to_string()
            }
        );

        let err =
            run_and_emit::<Out, _>(&emitter, "github::pr::create", String::new(), None, async {
                Err(Error::Handler("boom".to_string()))
            })
            .await;
        assert!(matches!(err, Err(Error::Handler(m)) if m == "boom"));
    }
}
