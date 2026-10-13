//! `console::mentions::trace::{search,get}` — the `@trace(id="<trace_id>")`
//! chat mention provider.
//!
//! Traces belong to the engine (`engine::traces::*`), which registers no
//! function metadata, so the console — the worker that owns the traces page
//! — declares the provider and wraps `engine::traces::list`. Search feeds
//! the composer's `@trace:` menu; get turns one trace id into the pill, the
//! preview card and the agent's one-line summary. Agents read the span tree
//! through `engine::traces::tree` (the provider's declared details).

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterFunction};
use mention_contract::{
    color, tone, MentionField, MentionGetRequest, MentionItem, MentionOpen, MentionProvider,
    MentionSearchRequest, MentionSearchResponse, MentionView,
};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

pub const SEARCH_ID: &str = "console::mentions::trace::search";
pub const GET_ID: &str = "console::mentions::trace::get";
pub const MENTION_NAME: &str = "trace";
pub const ICON: &str = "trace";
/// The console screen a trace opens in.
pub const TRACES_SCREEN: &str = "traces";

const TRACES_LIST_ID: &str = "engine::traces::list";
const TRACES_TIMEOUT_MS: u64 = 5_000;
/// `iii.session.name` baggage the harness stamps on every turn's trace.
const SESSION_NAME_TAG: &str = "iii.session.name";

pub fn provider() -> MentionProvider {
    MentionProvider::new(MENTION_NAME, "Traces", SEARCH_ID)
        .description(
            "An engine trace, by trace id; its span tree reads through engine::traces::tree",
        )
        .icon(ICON)
        .color(color::TEAL)
        .details("engine::traces::tree", "trace_id")
}

pub fn register(iii: &Arc<IIIClient>) {
    let client = iii.clone();
    iii.register_function(
        SEARCH_ID,
        RegisterFunction::new_async(move |request: MentionSearchRequest| {
            let client = client.clone();
            async move { search(&client, request).await }
        })
        .description(
            "Search recent engine traces for the chat @trace mention menu, by span name or trace id; newest first.",
        )
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
    let client = iii.clone();
    iii.register_function(
        GET_ID,
        RegisterFunction::new_async(move |request: MentionGetRequest| {
            let client = client.clone();
            async move { get(&client, request).await }
        })
        .description(
            "Resolve a @trace(id=…) chat mention to its pill, preview card and agent summary; null for an unknown trace. Declares the trace mention provider.",
        )
        .metadata(provider().metadata()),
    );
}

async fn search(
    iii: &IIIClient,
    request: MentionSearchRequest,
) -> Result<MentionSearchResponse, Error> {
    let limit = request.effective_limit();
    let rows = list_traces(iii, search_params(request.trimmed_query(), limit)).await?;
    Ok(MentionSearchResponse {
        items: rows.iter().take(limit).map(item).collect(),
    })
}

async fn get(iii: &IIIClient, request: MentionGetRequest) -> Result<Option<MentionView>, Error> {
    let trace_id = request.id.trim();
    if trace_id.is_empty() {
        return Ok(None);
    }
    let rows = list_traces(iii, get_params(trace_id)).await?;
    Ok(rows.iter().find(|row| row.trace_id == trace_id).map(view))
}

/// `engine::traces::list` rows; an engine without the memory exporter has
/// no traces to mention, so that error reads as an empty list.
async fn list_traces(iii: &IIIClient, payload: Value) -> Result<Vec<TraceRow>, Error> {
    let response = match iii
        .trigger(TriggerRequest {
            function_id: TRACES_LIST_ID.into(),
            payload,
            action: None,
            timeout_ms: Some(TRACES_TIMEOUT_MS),
        })
        .await
    {
        Ok(response) => response,
        Err(error) if is_exporter_disabled(&error.to_string()) => return Ok(Vec::new()),
        Err(error) => {
            return Err(Error::Handler(format!(
                "could not list engine traces: {error}"
            )))
        }
    };
    Ok(parse_rows(&response))
}

pub fn is_exporter_disabled(message: &str) -> bool {
    let message = message.to_lowercase();
    message.contains("memory exporter not enabled")
        || message.contains("memory exporter is not enabled")
}

/// Whether a query reads as a (possibly partial) trace id.
fn looks_like_trace_id(query: &str) -> bool {
    query.len() >= 16 && query.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The `engine::traces::list` payload for a search: newest first, the
/// console's own trace search (`name` across every span) for text, the
/// exact trace for a full id. Internal traces stay out, as on the traces
/// page by default.
pub fn search_params(query: &str, limit: usize) -> Value {
    let mut params = json!({
        "limit": limit,
        "offset": 0,
        "sort_by": "start_time",
        "sort_order": "desc",
        "include_internal": false,
    });
    if looks_like_trace_id(query) {
        params["trace_id"] = json!(query.to_lowercase());
        params["include_internal"] = json!(true);
    } else if !query.is_empty() {
        params["name"] = json!(query);
        params["search_all_spans"] = json!(true);
    }
    params
}

/// The `engine::traces::list` payload that reads one trace.
pub fn get_params(trace_id: &str) -> Value {
    json!({ "trace_id": trace_id, "limit": 1, "offset": 0, "include_internal": true })
}

/// One `engine::traces::list` row (`TraceSummary`), tolerant of fields an
/// older engine leaves out.
#[derive(Debug, Clone, Deserialize)]
pub struct TraceRow {
    pub trace_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "lenient_nanos")]
    pub start_time_unix_nano: Option<u128>,
    #[serde(default, deserialize_with = "lenient_nanos")]
    pub end_time_unix_nano: Option<u128>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub service_name: Option<String>,
    #[serde(default)]
    pub function_id: Option<String>,
    #[serde(default)]
    pub trace_tags: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    pub span_count: u64,
    #[serde(default)]
    pub error_count: u64,
    /// The row as the engine sent it, for a worker's own preview renderer.
    #[serde(skip)]
    pub raw: Value,
}

/// Nanosecond timestamps arrive as JSON numbers or, from engines that
/// guard against JS precision loss, as strings.
fn lenient_nanos<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u128>, D::Error> {
    Ok(match Option::<Value>::deserialize(deserializer)? {
        Some(Value::Number(number)) => number
            .as_u64()
            .map(u128::from)
            .or_else(|| number.as_f64().filter(|f| *f >= 0.0).map(|f| f as u128)),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    })
}

pub fn parse_rows(response: &Value) -> Vec<TraceRow> {
    response
        .get("traces")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|raw| {
                    let mut row: TraceRow = serde_json::from_value(raw.clone()).ok()?;
                    row.raw = raw.clone();
                    Some(row)
                })
                .collect()
        })
        .unwrap_or_default()
}

impl TraceRow {
    fn label(&self) -> String {
        if !self.name.trim().is_empty() {
            return self.name.clone();
        }
        self.function_id
            .clone()
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| "trace".to_string())
    }

    fn short_id(&self) -> String {
        self.trace_id.chars().take(8).collect()
    }

    fn status_word(&self) -> &str {
        match self.status.as_str() {
            "" => "unset",
            other => other,
        }
    }

    fn duration_ms(&self) -> Option<u128> {
        let start = self.start_time_unix_nano?;
        let end = self.end_time_unix_nano?;
        Some(end.saturating_sub(start) / 1_000_000)
    }

    fn duration_label(&self) -> String {
        match self.duration_ms() {
            Some(ms) => format_duration(ms),
            None if self.status == "pending" => "running".to_string(),
            None => "unknown duration".to_string(),
        }
    }

    fn session_name(&self) -> Option<&str> {
        self.trace_tags
            .as_ref()?
            .get(SESSION_NAME_TAG)?
            .as_str()
            .filter(|name| !name.is_empty())
    }

    fn spans_label(&self) -> String {
        match self.span_count {
            1 => "1 span".to_string(),
            n => format!("{n} spans"),
        }
    }
}

pub fn format_duration(ms: u128) -> String {
    if ms < 1_000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1_000.0)
    } else {
        let secs = ms / 1_000;
        format!("{}m {:02}s", secs / 60, secs % 60)
    }
}

fn rfc3339_from_nanos(nanos: u128) -> Option<String> {
    let secs = i64::try_from(nanos / 1_000_000_000).ok()?;
    let sub = u32::try_from(nanos % 1_000_000_000).ok()?;
    chrono::DateTime::from_timestamp(secs, sub)
        .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn status_color(row: &TraceRow) -> &'static str {
    match row.status.as_str() {
        "error" => color::ROSE,
        "pending" => color::AMBER,
        _ if row.error_count > 0 => color::ROSE,
        _ => color::TEAL,
    }
}

fn status_tone(row: &TraceRow) -> &'static str {
    match row.status.as_str() {
        "error" => tone::DANGER,
        "pending" => tone::INFO,
        "ok" => tone::SUCCESS,
        _ => tone::NEUTRAL,
    }
}

fn description(row: &TraceRow) -> String {
    let mut line = format!(
        "{} · {} · {}",
        row.status_word(),
        row.duration_label(),
        row.spans_label()
    );
    if let Some(service) = row.service_name.as_deref().filter(|s| !s.is_empty()) {
        line.push_str(" · ");
        line.push_str(service);
    }
    line
}

pub fn item(row: &TraceRow) -> MentionItem {
    MentionItem {
        id: row.trace_id.clone(),
        label: row.label(),
        hint: Some(row.short_id()),
        description: Some(description(row)),
        icon: None,
        color: Some(status_color(row).to_string()),
    }
}

pub fn view(row: &TraceRow) -> MentionView {
    let mut fields = vec![
        MentionField::new("Status", row.status_word()).tone(status_tone(row)),
        MentionField::new("Duration", row.duration_label()),
        MentionField::new("Spans", row.span_count.to_string()),
    ];
    if row.error_count > 0 {
        fields.push(MentionField::new("Errors", row.error_count.to_string()).tone(tone::DANGER));
    }
    if let Some(service) = row.service_name.as_deref().filter(|s| !s.is_empty()) {
        fields.push(MentionField::new("Service", service));
    }
    if let Some(function) = row.function_id.as_deref().filter(|f| !f.is_empty()) {
        fields.push(MentionField::new("Function", function));
    }
    if let Some(session) = row.session_name() {
        fields.push(MentionField::new("Session", session));
    }
    let started = row.start_time_unix_nano.and_then(rfc3339_from_nanos);
    if let Some(started) = &started {
        fields.push(MentionField::new("Started", started.clone()));
    }
    MentionView {
        id: row.trace_id.clone(),
        label: row.label(),
        hint: Some(row.short_id()),
        description: Some(description(row)),
        icon: Some(ICON.to_string()),
        color: Some(status_color(row).to_string()),
        fields,
        open: Some(MentionOpen::Page {
            page: TRACES_SCREEN.to_string(),
            context: Some(json!({ "trace_id": row.trace_id })),
        }),
        summary: Some(summary(row, started.as_deref())),
        data: Some(row.raw.clone()).filter(|raw| !raw.is_null()),
        updated_at: row
            .end_time_unix_nano
            .or(row.start_time_unix_nano)
            .and_then(rfc3339_from_nanos),
    }
}

fn summary(row: &TraceRow, started: Option<&str>) -> String {
    let label = serde_json::to_string(&row.label()).unwrap_or_default();
    let mut line = format!(
        "Trace {} {label} · status: {} · {} · {}",
        row.trace_id,
        row.status_word(),
        row.duration_label(),
        row.spans_label(),
    );
    if row.error_count > 0 {
        line.push_str(&format!(" ({} with errors)", row.error_count));
    }
    if let Some(service) = row.service_name.as_deref().filter(|s| !s.is_empty()) {
        line.push_str(&format!(" · service: {service}"));
    }
    if let Some(session) = row.session_name() {
        let session = serde_json::to_string(session).unwrap_or_default();
        line.push_str(&format!(" · session: {session}"));
    }
    if let Some(started) = started {
        line.push_str(&format!(" · started {started}"));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<TraceRow> {
        parse_rows(&json!({
            "traces": [
                {
                    "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
                    "name": "harness::send",
                    "start_time_unix_nano": 1_791_244_800_000_000_000u64,
                    "end_time_unix_nano": "1791244801250000000",
                    "status": "error",
                    "service_name": "harness",
                    "function_id": "harness::send",
                    "trace_tags": { "iii.session.name": "fix login" },
                    "span_count": 14,
                    "error_count": 2
                },
                { "trace_id": "00f067aa0ba902b7", "status": "pending", "span_count": 1 },
                { "name": "no id — dropped" }
            ],
            "total": 3
        }))
    }

    #[test]
    fn search_params_pick_name_search_or_exact_id() {
        let text = search_params("send", 5);
        assert_eq!(text["name"], json!("send"));
        assert_eq!(text["search_all_spans"], json!(true));
        assert_eq!(text["include_internal"], json!(false));
        assert_eq!(text["limit"], json!(5));
        let id = search_params("4BF92F3577B34DA6A3CE929D0E0E4736", 5);
        assert_eq!(id["trace_id"], json!("4bf92f3577b34da6a3ce929d0e0e4736"));
        assert!(id.get("name").is_none());
        let recent = search_params("", 5);
        assert!(recent.get("name").is_none() && recent.get("trace_id").is_none());
        assert_eq!(recent["sort_order"], json!("desc"));
    }

    #[test]
    fn rows_parse_leniently_and_drop_rows_without_an_id() {
        let rows = rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].duration_ms(), Some(1_250));
        assert_eq!(rows[1].label(), "trace");
        assert!(parse_rows(&json!({ "nope": [] })).is_empty());
    }

    #[test]
    fn item_and_view_describe_the_trace() {
        let rows = rows();
        let item = item(&rows[0]);
        assert_eq!(item.hint.as_deref(), Some("4bf92f35"));
        assert_eq!(
            item.description.as_deref(),
            Some("error · 1.2 s · 14 spans · harness")
        );
        assert_eq!(item.color.as_deref(), Some(color::ROSE));

        let view = view(&rows[0]);
        assert_eq!(
            view.open,
            Some(MentionOpen::Page {
                page: "traces".into(),
                context: Some(json!({ "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736" })),
            })
        );
        assert!(view
            .fields
            .iter()
            .any(|f| f.label == "Session" && f.value == "fix login"));
        assert_eq!(
            view.summary.as_deref(),
            Some(
                r#"Trace 4bf92f3577b34da6a3ce929d0e0e4736 "harness::send" · status: error · 1.2 s · 14 spans (2 with errors) · service: harness · session: "fix login" · started 2026-10-06T00:00:00.000Z"#
            )
        );
        assert_eq!(view.updated_at.as_deref(), Some("2026-10-06T00:00:01.250Z"));
        assert_eq!(view.data.as_ref().unwrap()["span_count"], json!(14));

        let pending = super::view(&rows[1]);
        assert_eq!(
            pending.description.as_deref(),
            Some("pending · running · 1 span")
        );
        assert_eq!(pending.color.as_deref(), Some(color::AMBER));
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(format_duration(12), "12 ms");
        assert_eq!(format_duration(1_250), "1.2 s");
        assert_eq!(format_duration(125_000), "2m 05s");
    }

    #[test]
    fn exporter_off_is_recognised() {
        assert!(is_exporter_disabled("Handler: memory exporter not enabled"));
        assert!(is_exporter_disabled("Memory exporter is not enabled"));
        assert!(!is_exporter_disabled("timeout"));
    }

    #[test]
    fn the_get_function_declares_the_provider() {
        let provider =
            MentionProvider::from_metadata(&provider().metadata()).expect("descriptor validates");
        assert_eq!(provider.name, "trace");
        assert_eq!(provider.search, SEARCH_ID);
        let details = provider.details.expect("details");
        assert_eq!(details.function_id, "engine::traces::tree");
        assert_eq!(details.payload("t"), json!({ "trace_id": "t" }));
    }
}
