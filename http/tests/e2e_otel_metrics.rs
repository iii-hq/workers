//! Phase 3 e2e: the per-request OTEL counter `iii.http.requests` increments once
//! per handled request, carrying method/route/status attributes that mirror the
//! HTTP span tags.
//!
//! Determinism note: the handler creates its counter lazily from the *global*
//! meter provider on the first request in the process (a `OnceLock`), and
//! `opentelemetry::global::meter` binds the instrument to whatever provider is
//! installed at that moment. This test therefore lives in its own integration
//! binary (separate process) so no other test issues a request first, and it
//! installs an in-memory `SdkMeterProvider` as the global provider *before* any
//! request -- overriding whatever the SDK's `init_otel` set on connect. A single
//! `#[serial]` test keeps the counter binding stable, so the assertion is
//! deterministic.

mod common;

use common::{backend, engine, worker};
use opentelemetry::global;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};
use serial_test::serial;

#[tokio::test]
#[serial]
async fn http_request_counter_increments_per_request() {
    // In-memory meter provider. Installed as the global provider BEFORE the
    // worker handles any request so the handler's lazily-built counter binds to
    // it (see module docs). The periodic reader only exports on `force_flush`
    // here (its interval dwarfs the test), so the export is a single snapshot.
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter.clone())
        .build();

    let iii = engine::get_or_init().await;
    // Override any provider the SDK's `init_otel` installed on connect. Safe to
    // do after `get_or_init` returns: `init_otel` runs (and sets its provider)
    // before the connection is ready, so there is no late race that could clobber
    // this one.
    global::set_meter_provider(provider.clone());

    let boot = worker::start_http_worker(iii.clone()).await;
    backend::register_echo_backend(&iii, "/otel/metric/:id", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/otel/metric/:id").await;

    // Drive N identical requests. Identical attributes collapse into one Sum
    // datapoint whose value is the count.
    const N: u64 = 3;
    let client = reqwest::Client::new();
    for i in 0..N {
        let url = format!("http://{}/otel/metric/{i}", boot.local_addr);
        let resp = client.get(&url).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let _ = resp.bytes().await.unwrap();
    }

    // Export the accumulated metrics into the in-memory exporter.
    provider.force_flush().expect("force_flush");

    let metrics = exporter.get_finished_metrics().expect("finished metrics");
    let mut total: u64 = 0;
    let mut saw_attrs = false;
    for rm in &metrics {
        for sm in rm.scope_metrics() {
            for m in sm.metrics() {
                if m.name() != "iii.http.requests" {
                    continue;
                }
                let AggregatedMetrics::U64(MetricData::Sum(sum)) = m.data() else {
                    panic!("iii.http.requests should be a u64 Sum aggregation");
                };
                for dp in sum.data_points() {
                    total += dp.value();
                    let has = |k: &str, v: &str| {
                        dp.attributes()
                            .any(|kv| kv.key.as_str() == k && kv.value.as_str() == v)
                    };
                    if has("http.request.method", "GET")
                        && has("http.route", "/otel/metric/:id")
                        && has("http.response.status_code", "200")
                    {
                        saw_attrs = true;
                    }
                }
            }
        }
    }

    assert_eq!(total, N, "counter should increment exactly once per request");
    assert!(
        saw_attrs,
        "counter datapoint should carry method/route/status attributes mirroring the span"
    );

    boot.shutdown().await;
}
