//! `judge::evaluate` usage is counted per calling worker, named from
//! `engine::workers::list`. Its own binary: the hub's instruments bind to the
//! global meter provider on first use, so the in-memory one is installed first.
#[path = "../../judge-typesafe/tests/support/fake_engine.rs"]
mod fake_engine;
use iii_sdk::{register_worker, InitOptions};
use judge_contract::FUNCTION_ID;
use opentelemetry::global;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::timeout};

#[tokio::test]
async fn evaluate_is_counted_per_named_caller() {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter.clone())
        .build();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let installed = provider.clone();
    let mut call = Some(json!({
        "_caller_worker_id": "conn-7", "timeout_ms": 1000,
        "evaluations": [{"id": "e", "state": {}, "questions": {"q": {"type": "noul"}}}]
    }));
    let engine = fake_engine::start(move |message| match message["type"].as_str() {
        Some("registerfunction") if message["id"] == FUNCTION_ID => {
            // The SDK installed its own provider before it registered; replace
            // it before the hub handles the call.
            global::set_meter_provider(installed.clone());
            vec![json!({"type": "invokefunction", "invocation_id": "00000000-0000-0000-0000-000000000001",
                "function_id": FUNCTION_ID, "data": call.take().expect("registered once")})]
        }
        Some("invokefunction") if !message["invocation_id"].is_null() => {
            let result = match message["function_id"].as_str() {
                Some("engine::workers::list") => {
                    json!({"workers": [{"id": "conn-7", "name": "sentinel"}]})
                }
                _ => json!({"status": "ok", "model": "jev-1.14.0",
                    "results": {"e": {"answers": {"q": {"type": "noul", "noul": 0.5}}}},
                    "stats": {"attempts": 1, "requests": 1, "questions": 1, "input_tokens": 600,
                        "output_tokens": 50, "elapsed_ms": 1, "usage_complete": true}}),
            };
            vec![json!({"type": "invocationresult", "invocation_id": message["invocation_id"],
                "function_id": message["function_id"], "result": result})]
        }
        Some("invocationresult") => {
            tx.send(()).unwrap();
            vec![]
        }
        _ => vec![],
    })
    .await;
    let iii = Arc::new(register_worker(&engine.url, InitOptions::default()));
    judge::register(
        &iii,
        judge::configuration::new_cell(judge::JudgeConfig::default()),
    );
    timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("hub answers the invocation");
    iii.shutdown_async().await;
    provider.force_flush().expect("flush");

    let metrics = exporter.get_finished_metrics().expect("metrics");
    let points = |name: &str| -> Vec<(u64, Vec<(String, String)>)> {
        let mut found = Vec::new();
        for metric in metrics
            .iter()
            .flat_map(|resource| resource.scope_metrics())
            .flat_map(|scope| scope.metrics())
            .filter(|metric| metric.name() == name)
        {
            let AggregatedMetrics::U64(MetricData::Sum(sum)) = metric.data() else {
                panic!("{name} is a u64 sum");
            };
            for point in sum.data_points() {
                let attributes = point
                    .attributes()
                    .map(|kv| (kv.key.to_string(), kv.value.to_string()))
                    .collect();
                found.push((point.value(), attributes));
            }
        }
        found
    };
    let has = |attributes: &[(String, String)], key: &str, value: &str| {
        attributes.iter().any(|(k, v)| k == key && v == value)
    };
    let calls = points("judge.calls");
    assert_eq!(calls.len(), 1, "{calls:?}");
    let (count, attributes) = &calls[0];
    assert_eq!(*count, 1);
    for (key, value) in [
        ("caller", "sentinel"),
        ("provider", "typesafe"),
        ("model", "jev-1.14.0"),
        ("outcome", "ok"),
    ] {
        assert!(
            has(attributes, key, value),
            "{key}={value} in {attributes:?}"
        );
    }
    let tokens = points("judge.tokens");
    let input = tokens.iter().find(|(_, a)| has(a, "direction", "input"));
    assert_eq!(input.map(|(count, _)| *count), Some(600), "{tokens:?}");
}
