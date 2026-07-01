//! End-to-end coverage for the per-request OTEL/tracing HTTP span emitted by
//! `dynamic_handler` (`handler.rs`).
//!
//! The worker installs no OTEL export layer in tests (`main.rs` uses a plain
//! `fmt` subscriber, and `boot::start` installs none), so this binary installs
//! a process-wide `tracing_subscriber` capture layer that records the fields of
//! every `HTTP` span. The worker's server task runs on the same process, so the
//! global subscriber sees the spans it emits. Each test drives a real HTTP
//! request through the booted worker and asserts the captured span carries the
//! expected tags (`otel.name`, `otel.kind`, `http.request.method`,
//! `http.route`, `url.path`) plus the status recorded at the end
//! (`otel.status_code`, `http.response.status_code`).
//!
//! Distinct URL paths per test let the assertions pick out the right span from
//! the shared, cumulative capture buffer.

mod common;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use common::{backend, engine, worker};
use serial_test::serial;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

/// Fields captured for a single span, keyed by field name and stringified.
type SpanFields = HashMap<String, String>;

/// Newtype so the per-span field map is uniquely typed in span extensions.
struct CapturedFields(SpanFields);

/// Process-wide buffer of every closed `HTTP` span's fields.
fn captured() -> &'static Mutex<Vec<SpanFields>> {
    static CAPTURED: OnceLock<Mutex<Vec<SpanFields>>> = OnceLock::new();
    CAPTURED.get_or_init(|| Mutex::new(Vec::new()))
}

/// Install the global capture subscriber exactly once for this test binary.
fn install_capture() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        tracing_subscriber::registry().with(CaptureLayer).init();
    });
}

/// Visitor that stringifies field values. `%`-formatted fields (Display) arrive
/// as `record_debug(format_args)`, whose `{:?}` renders the plain string with no
/// quotes; `otel.kind`/`otel.status_code` arrive as `record_str`; the numeric
/// status code as `record_u64`.
struct FieldVisitor(SpanFields);

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_string(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
}

/// Layer capturing the `HTTP` span's initial fields (`on_new_span`) and any
/// later `record()` calls (`on_record`), flushing the merged map to the global
/// buffer on span close.
struct CaptureLayer;

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("span exists on new_span");
        if span.name() != "HTTP" {
            return;
        }
        let mut visitor = FieldVisitor(SpanFields::new());
        attrs.record(&mut visitor);
        span.extensions_mut().insert(CapturedFields(visitor.0));
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("span exists on record");
        if span.name() != "HTTP" {
            return;
        }
        let mut ext = span.extensions_mut();
        if let Some(existing) = ext.get_mut::<CapturedFields>() {
            let mut visitor = FieldVisitor(std::mem::take(&mut existing.0));
            values.record(&mut visitor);
            existing.0 = visitor.0;
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let span = ctx.span(&id).expect("span exists on close");
        if span.name() != "HTTP" {
            return;
        }
        let ext = span.extensions();
        if let Some(fields) = ext.get::<CapturedFields>() {
            captured().lock().unwrap().push(fields.0.clone());
        }
    }
}

/// Poll the capture buffer (spans close slightly after the response returns)
/// for the `HTTP` span whose `url.path` matches, up to ~2s.
async fn wait_for_span(url_path: &str) -> SpanFields {
    for _ in 0..40 {
        if let Some(found) = captured()
            .lock()
            .unwrap()
            .iter()
            .find(|f| f.get("url.path").map(String::as_str) == Some(url_path))
            .cloned()
        {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no HTTP span captured for url.path {url_path}");
}

#[tokio::test]
#[serial]
async fn otel_span_tags_for_200() {
    install_capture();
    let iii = engine::get_or_init().await;
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_echo_backend(&iii, "/otel/ping/:id", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/otel/ping/:id").await;

    let url = format!("http://{}/otel/ping/42", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let _ = resp.bytes().await.unwrap();

    let span = wait_for_span("/otel/ping/42").await;
    assert_eq!(span.get("otel.name").map(String::as_str), Some("GET /otel/ping/:id"));
    assert_eq!(span.get("otel.kind").map(String::as_str), Some("server"));
    assert_eq!(span.get("http.request.method").map(String::as_str), Some("GET"));
    assert_eq!(span.get("http.route").map(String::as_str), Some("/otel/ping/:id"));
    assert_eq!(span.get("url.path").map(String::as_str), Some("/otel/ping/42"));
    assert_eq!(span.get("otel.status_code").map(String::as_str), Some("OK"));
    assert_eq!(span.get("http.response.status_code").map(String::as_str), Some("200"));

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn otel_span_tags_for_404() {
    install_capture();
    let iii = engine::get_or_init().await;
    let boot = worker::start_http_worker(iii.clone()).await;

    // No route registered for this path -> 404. `http.route`/`otel.name` fall
    // back to the actual request path (no matched pattern).
    let url = format!("http://{}/otel/missing", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 404);
    let _ = resp.bytes().await.unwrap();

    let span = wait_for_span("/otel/missing").await;
    assert_eq!(span.get("otel.name").map(String::as_str), Some("GET /otel/missing"));
    assert_eq!(span.get("otel.kind").map(String::as_str), Some("server"));
    assert_eq!(span.get("http.request.method").map(String::as_str), Some("GET"));
    assert_eq!(span.get("http.route").map(String::as_str), Some("/otel/missing"));
    assert_eq!(span.get("otel.status_code").map(String::as_str), Some("ERROR"));
    assert_eq!(span.get("http.response.status_code").map(String::as_str), Some("404"));

    boot.shutdown().await;
}
