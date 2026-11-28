//! Through a real engine (`III_ENGINE_BIN`; skipped when unset): inside
//! `iii compose`, which gives the worker's process its name and namespace
//! (`III_WORKER_NAME`, `III_NAMESPACE`), the socket to iii cloud still calls the
//! cloud's functions in the namespace they are in, `default`. A test binary of
//! its own, since it sets the process's environment.

mod support;

use std::sync::Arc;
use std::time::Duration;

use iii_cloud::cloud::{FunctionsAt, Req};
use iii_cloud::ctx::Credential;
use iii_cloud::types::*;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{Error, InitOptions, RegisterFunction, WorkerIdentityMode, register_worker};
use serde_json::{Value, json};
use support::real_engine::RealEngine;
use support::{Fake, harness};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn the_cloud_s_functions_are_called_in_its_namespace_whatever_the_process_s() {
    if std::env::var_os("III_ENGINE_BIN").is_none() {
        return;
    }
    // As `iii compose` sets them for the worker it starts, before any thread of
    // this binary reads the environment.
    unsafe {
        std::env::set_var("III_NAMESPACE", "my-project");
        std::env::set_var("III_WORKER_NAME", "iii-cloud");
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let engine = RealEngine::start().await.expect("III_ENGINE_BIN is set");
        // iii cloud's control plane registers its functions in `default`.
        let cloud = register_worker(
            &engine.url,
            InitOptions {
                metadata: Some(WorkerMetadata {
                    name: "iii-cloud-control-plane".into(),
                    ..Default::default()
                }),
                identity: WorkerIdentityMode::Explicit,
                namespace: Some("default".into()),
                ..Default::default()
            },
        );
        cloud.register_function(
            "cloud::identity::me",
            RegisterFunction::new_async(|_: Value| async {
                Ok::<Value, Error>(json!({ "over": "function" }))
            })
            .metadata(json!({ "api": { "method": "GET", "path": "/api/v1/me", "fields": [] } })),
        );
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "over": "http" })))
            .mount(&server)
            .await;
        let h = harness(&server.uri(), Arc::new(Fake::default()));
        *h.ctx.functions_at.lock().unwrap() = FunctionsAt::Engine(engine.url.clone());
        *h.ctx.cred.lock().unwrap() = Some(Credential {
            method: Method::ApiKey,
            token: "iii_namespace_test".into(),
        });
        for _ in 0..150 {
            let answer = h.ctx.call(&Req::get("/me")).await.unwrap();
            if answer["over"] == "function" {
                cloud.shutdown_async().await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the call never reached the cloud's function in `default`");
    });
}
