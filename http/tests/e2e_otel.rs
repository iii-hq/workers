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
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
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

/// Process-wide in-memory OTel span exporter. Populated by the
/// `tracing_opentelemetry` bridge layer installed in [`install_capture`]. The
/// trace-context test reads exported spans (their events + trace ids) from here;
/// the field-based Phase-1 tests ignore it.
fn exported_spans() -> &'static InMemorySpanExporter {
    static EXPORTER: OnceLock<InMemorySpanExporter> = OnceLock::new();
    EXPORTER.get_or_init(InMemorySpanExporter::default)
}

/// Install the global capture subscriber exactly once for this test binary.
///
/// Two layers share the registry: [`CaptureLayer`] records span *fields* for the
/// Phase-1 tag assertions, and a `tracing_opentelemetry` bridge exports each span
/// (with the OTel `set_parent`/`add_event` data) into [`exported_spans`]. The
/// bridge is required for `set_parent` to resolve at all -- without an OTel layer
/// its `WithContext` downcast fails and parent propagation is a no-op.
fn install_capture() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exported_spans().clone())
            .build();
        let tracer = opentelemetry::trace::TracerProvider::tracer(&provider, "e2e-otel-test");
        // Install the same provider as the process-wide OTel tracer, so the
        // receiving SDK's `global::tracer("iii-rust-sdk")` (which builds the
        // `execute <fn>` span for each invocation) produces real, trace-id-
        // bearing spans. Without this the global tracer is a no-op and a
        // backend function's `Context::current()` trace id would be all-zero,
        // making the outbound-propagation assertion meaningless.
        opentelemetry::global::set_tracer_provider(provider.clone());
        // Keep the provider alive for the whole test binary; dropping it would
        // shut down the exporter.
        Box::leak(Box::new(provider));

        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
        tracing_subscriber::registry()
            .with(CaptureLayer)
            .with(otel_layer)
            .init();
    });
}

/// Visitor that stringifies field values. `%`-formatted fields (Display) arrive
/// as `record_debug(format_args)`, whose `{:?}` renders the plain string with no
/// quotes; `otel.kind`/`otel.status_code` arrive as `record_str`; the numeric
/// status code as `record_u64`.
struct FieldVisitor(SpanFields);

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
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
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_echo_backend(&iii, "/otel/ping/:id", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/otel/ping/:id").await;

    let url = format!("http://{}/otel/ping/42", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let _ = resp.bytes().await.unwrap();

    let span = wait_for_span("/otel/ping/42").await;
    assert_eq!(
        span.get("otel.name").map(String::as_str),
        Some("GET /otel/ping/:id")
    );
    assert_eq!(span.get("otel.kind").map(String::as_str), Some("server"));
    assert_eq!(
        span.get("http.request.method").map(String::as_str),
        Some("GET")
    );
    assert_eq!(
        span.get("http.route").map(String::as_str),
        Some("/otel/ping/:id")
    );
    assert_eq!(
        span.get("url.path").map(String::as_str),
        Some("/otel/ping/42")
    );
    assert_eq!(span.get("otel.status_code").map(String::as_str), Some("OK"));
    assert_eq!(
        span.get("http.response.status_code").map(String::as_str),
        Some("200")
    );

    boot.shutdown().await;
}

#[tokio::test]
#[serial]
async fn otel_span_tags_for_404() {
    install_capture();
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let boot = worker::start_http_worker(iii.clone()).await;

    // No route registered for this path -> 404. `http.route`/`otel.name` fall
    // back to the actual request path (no matched pattern).
    let url = format!("http://{}/otel/missing", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 404);
    let _ = resp.bytes().await.unwrap();

    let span = wait_for_span("/otel/missing").await;
    assert_eq!(
        span.get("otel.name").map(String::as_str),
        Some("GET /otel/missing")
    );
    assert_eq!(span.get("otel.kind").map(String::as_str), Some("server"));
    assert_eq!(
        span.get("http.request.method").map(String::as_str),
        Some("GET")
    );
    assert_eq!(
        span.get("http.route").map(String::as_str),
        Some("/otel/missing")
    );
    assert_eq!(
        span.get("otel.status_code").map(String::as_str),
        Some("ERROR")
    );
    assert_eq!(
        span.get("http.response.status_code").map(String::as_str),
        Some("404")
    );

    boot.shutdown().await;
}

/// Poll the exported OTel spans for the closed `HTTP` span whose `url.path`
/// attribute matches, up to ~2s. Returns owned `SpanData` (events + context).
async fn wait_for_exported_span(url_path: &str) -> opentelemetry_sdk::trace::SpanData {
    for _ in 0..40 {
        let found = exported_spans()
            .get_finished_spans()
            .unwrap_or_default()
            .into_iter()
            // The OTel span name is derived from the `otel.name` field
            // (e.g. "GET /otel/trace/:id"), not the tracing metadata name
            // "HTTP", so match on the `url.path` attribute instead.
            .find(|s| {
                s.attributes
                    .iter()
                    .any(|kv| kv.key.as_str() == "url.path" && kv.value.as_str() == url_path)
            });
        if let Some(span) = found {
            return span;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no exported HTTP span for url.path {url_path}");
}

/// Phase 2: an inbound `traceparent` makes the worker's HTTP span a child of the
/// caller's trace. The known trace id is carried by the header; the handler
/// records it back on the span via the `traceparent.propagated` event (matching
/// the engine). We assert both the event's `parent.trace_id` attribute and the
/// exported span's own trace id equal the known value -- proving the header was
/// parsed and set as the span's parent.
#[tokio::test]
#[serial]
async fn otel_span_propagates_inbound_traceparent() {
    install_capture();
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_echo_backend(&iii, "/otel/trace/:id", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/otel/trace/:id").await;

    // W3C example traceparent; the 32-hex trace id is the value we expect to see
    // propagated onto the server span.
    const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    const KNOWN_TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";

    let url = format!("http://{}/otel/trace/7", boot.local_addr);
    let resp = reqwest::Client::new()
        .get(&url)
        .header("traceparent", TRACEPARENT)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let _ = resp.bytes().await.unwrap();

    let span = wait_for_exported_span("/otel/trace/7").await;

    // The span's own trace id is inherited from the parent traceparent.
    assert_eq!(
        format!("{}", span.span_context.trace_id()),
        KNOWN_TRACE_ID,
        "server span should inherit the inbound traceparent trace id"
    );

    // The `traceparent.propagated` event carries `parent.trace_id` (engine parity).
    let event = span
        .events
        .iter()
        .find(|e| e.name == "traceparent.propagated")
        .expect("HTTP span should carry a traceparent.propagated event");
    let parent_trace_id = event
        .attributes
        .iter()
        .find(|kv| kv.key.as_str() == "parent.trace_id")
        .map(|kv| kv.value.as_str().to_string())
        .expect("event should carry parent.trace_id");
    assert_eq!(parent_trace_id, KNOWN_TRACE_ID);

    boot.shutdown().await;
}

/// Phase 3 (outbound): the worker must propagate its HTTP span's trace context
/// onto the trigger it fires at the matched function, so `HTTP GET -> worker ->
/// function` is ONE trace rather than two disjoint ones. We register a backend
/// that records the trace id it runs under, drive a request with NO inbound
/// `traceparent` (so the worker's HTTP span is the trace root), and assert the
/// function ran under the SAME trace id as the worker's exported HTTP span.
///
/// Buggy behaviour (no context attached to the outbound trigger): the worker
/// injects no `traceparent`, the receiving SDK starts a fresh root trace, and
/// the function's trace id differs from the HTTP span's -> assertion fails.
#[tokio::test]
#[serial]
async fn otel_span_propagates_outbound_traceparent_to_function() {
    install_capture();
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let boot = worker::start_http_worker(iii.clone()).await;
    let captured_trace_id =
        backend::register_trace_capturing_backend(&iii, "/otel/outbound/:id", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/otel/outbound/:id").await;

    let url = format!("http://{}/otel/outbound/9", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let _ = resp.bytes().await.unwrap();

    let span = wait_for_exported_span("/otel/outbound/9").await;
    let http_trace_id = format!("{}", span.span_context.trace_id());

    // The function may record its trace id slightly after the response returns.
    let mut function_trace_id = None;
    for _ in 0..40 {
        if let Some(id) = captured_trace_id.lock().unwrap().clone() {
            function_trace_id = Some(id);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let function_trace_id =
        function_trace_id.expect("backend function should have recorded a trace id");

    assert_eq!(
        function_trace_id, http_trace_id,
        "the invoked function must run under the worker's HTTP-span trace \
         (one unified trace), not a disconnected trace"
    );

    boot.shutdown().await;
}
