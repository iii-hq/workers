//! A streamed response and the call of `http::on-request` behind it, run
//! in-process: the fake engine confirms local dispatch, like the engine. The
//! call's `execute http::on-request` span ends when the call does, so its
//! length tells whether the call ran to its end.
mod common;

use std::time::Duration;

use common::fake_engine::FakeEngine;
use iii_http::boot;
use iii_http::config::RestApiConfig;
use iii_http::on_request::ON_REQUEST_FN;
use iii_http::types::HttpRequest;
use iii_sdk::RegisterFunction;
use iii_sdk::channel::ChannelWriter;
use iii_sdk::errors::Error;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use serde_json::{Value, json};

#[tokio::test]
async fn a_stream_that_ends_normally_lets_its_call_finish() {
    let spans = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(spans.clone())
        .build();
    let engine = FakeEngine::start_with_local_dispatch().await;
    let config = RestApiConfig {
        host: "127.0.0.1".into(),
        port: 0,
        ..Default::default()
    };
    let boot = boot::start(engine.iii.clone(), config).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !boot.on_request.local_dispatch_active() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("http::on-request runs in-process");
    // After the SDK's own telemetry setup, which installs its provider.
    opentelemetry::global::set_tracer_provider(provider.clone());
    let url = engine.url.clone();
    engine.iii.register_function(
        "test::sized".to_string(),
        RegisterFunction::new_async(move |req: HttpRequest| {
            let url = url.clone();
            async move {
                // A body of a stated length: the server is done with it at
                // its last byte, before the channel closes.
                let writer = ChannelWriter::new(&url, &req.response);
                let headers = json!({"type": "set_headers", "headers": {"content-length": "5"}});
                writer.send_message(&headers.to_string()).await?;
                writer.write(b"hello").await?;
                writer.close().await?;
                tokio::time::sleep(Duration::from_millis(400)).await;
                Ok::<Value, Error>(Value::Null)
            }
        }),
    );
    let observer = json!({
        "id": "obs-sized",
        "target": "test::sized",
        "args": {"method": "GET", "path": "/new/sized"},
        "filters": {},
    });
    assert!(engine.add_observer(ON_REQUEST_FN, observer).await.is_none());

    let resp = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("http://{}/new/sized", boot.local_addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "hello");

    // The target answers 400 ms after its last byte; the call waits for it.
    tokio::time::sleep(Duration::from_millis(800)).await;
    let name = format!("execute {ON_REQUEST_FN}");
    let execute = spans
        .get_finished_spans()
        .unwrap()
        .into_iter()
        .find(|span| span.name == name)
        .expect("the call's span");
    let lasted = execute.end_time.duration_since(execute.start_time).unwrap();
    assert!(
        lasted >= Duration::from_millis(350),
        "the call ran {lasted:?}"
    );

    boot.shutdown().await;
    engine.shutdown().await;
    let _ = provider.shutdown();
}
