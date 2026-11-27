//! Through a real engine (`III_ENGINE_BIN`; skipped when unset), standing in
//! for iii cloud's: the worker calls the cloud's functions over a socket, reads
//! a deployment's events from the channel its function answers, and goes over
//! HTTP where a function can't answer.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use iii_cloud::cloud::{FunctionsAt, Req};
use iii_cloud::ctx::Credential;
use iii_cloud::types::*;
use iii_sdk::{Error, Pipe, RegisterFunction};
use serde_json::{Value, json};
use support::real_engine::RealEngine;
use support::{Fake, harness};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const EVENTS: &str =
    "/teams/{team_id}/environments/{environment_id}/deployments/{deployment_id}/events";

fn api(method: &str, path: &str, fields: &[&str]) -> Value {
    json!({ "api": { "method": method, "path": format!("/api/v1{path}"), "fields": fields } })
}

fn echo() -> RegisterFunction {
    RegisterFunction::new_async(|input: Value| async move {
        Ok::<Value, Error>(json!({ "over": "function", "input": input }))
    })
}

async fn http_answers(server: &MockServer, route: &str) {
    Mock::given(path(format!("/api/v1{route}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "over": "http" })))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn calls_go_to_the_cloud_s_functions_and_http_takes_what_they_cannot_answer() {
    let Some(engine) = RealEngine::start().await else {
        return;
    };
    let cloud = engine.client("iii-cloud-control-plane").await;
    cloud.register_function(
        "cloud::identity::me",
        echo().metadata(api("GET", "/me", &[])),
    );
    cloud.register_function(
        "cloud::deployments::create",
        echo().metadata(api(
            "POST",
            "/teams/{team_id}/environments/{environment_id}/deployments",
            &["team_id", "environment_id", "idempotency_key", "body"],
        )),
    );
    cloud.register_function(
        "cloud::variables::delete",
        RegisterFunction::new_async(|_: Value| async { Ok::<Value, Error>(Value::Null) }).metadata(
            api(
                "DELETE",
                "/teams/{team_id}/variables/{name}",
                &["team_id", "name"],
            ),
        ),
    );
    cloud.register_function(
        "cloud::projects::create",
        RegisterFunction::new_async(|_: Value| async {
            Err::<Value, Error>(Error::Remote {
                code: "CONFLICT".into(),
                message: "Taken. Pick another.".into(),
                stacktrace: Some(
                    json!({ "status": 409, "body": { "code": "CONFLICT", "message": "Taken.", "fix": "Pick another." } })
                        .to_string(),
                ),
            })
        })
        .metadata(api("POST", "/teams/{team_id}/projects", &["team_id", "idempotency_key", "body"])),
    );
    let gone = cloud.register_function(
        "cloud::gone::read",
        echo().metadata(api("GET", "/gone", &[])),
    );
    engine.function(&cloud, "cloud::gone::read").await;

    let server = MockServer::start().await;
    http_answers(&server, "/me").await;
    http_answers(&server, "/gone").await;
    let h = harness(&server.uri(), Arc::new(Fake::default()));
    *h.ctx.functions_at.lock().unwrap() = FunctionsAt::Engine(engine.url.clone());
    *h.ctx.cred.lock().unwrap() = Some(Credential {
        method: Method::ApiKey,
        token: "iii_functions_test".into(),
    });

    // The first call opens the socket and goes over HTTP; once it has listed the functions, calls go to them.
    assert_eq!(h.ctx.call(&Req::get("/me")).await.unwrap()["over"], "http");
    let mut over = String::new();
    for _ in 0..150 {
        over = h.ctx.call(&Req::get("/me")).await.unwrap()["over"]
            .as_str()
            .unwrap()
            .to_owned();
        if over == "function" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(over, "function");

    // A write: its parameters decoded, its body and its idempotency key, by field.
    let (status, answer) = h
        .ctx
        .raw(&Req::post(
            "/teams/t%201/environments/e1/deployments",
            json!({ "commit": "abc" }),
        ))
        .await
        .unwrap();
    assert_eq!(status, 200);
    let input = &answer["input"];
    assert_eq!(
        (&input["team_id"], &input["environment_id"], &input["body"]),
        (&json!("t 1"), &json!("e1"), &json!({ "commit": "abc" }))
    );
    assert!(
        input["idempotency_key"]
            .as_str()
            .is_some_and(|k| !k.is_empty())
    );

    // An empty answer is a 204, as HTTP says it.
    let delete = Req {
        verb: Some(reqwest::Method::DELETE),
        ..Req::get("/teams/t1/variables/A")
    };
    assert_eq!(h.ctx.raw(&delete).await.unwrap(), (204, Value::Null));

    // The API's refusal comes back as its status and body.
    let create = Req::post("/teams/t1/projects", json!({ "name": "a" }));
    assert_eq!(
        h.ctx.raw(&create).await.unwrap(),
        (
            409,
            json!({ "code": "CONFLICT", "message": "Taken.", "fix": "Pick another." })
        )
    );

    // What the function can't carry (a query it has no field for) goes over HTTP.
    let queried = h
        .ctx
        .call(&Req::get("/me").query("extra", "1"))
        .await
        .unwrap();
    assert_eq!(queried["over"], "http");

    // A function the engine no longer has (the control plane reconnecting): nothing ran, HTTP answers.
    gone.unregister();
    let mut over = String::new();
    for _ in 0..50 {
        over = h.ctx.call(&Req::get("/gone")).await.unwrap()["over"]
            .as_str()
            .unwrap()
            .to_owned();
        if over == "http" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(over, "http");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_deployment_s_events_are_read_from_its_function_s_channel_until_the_first_ping() {
    let Some(engine) = RealEngine::start().await else {
        return;
    };
    let cloud = engine.client("iii-cloud-control-plane").await;
    let left = Arc::new(AtomicBool::new(false));
    let (writer, gone) = (cloud.clone(), left.clone());
    let mut metadata = api(
        "GET",
        EVENTS,
        &[
            "team_id",
            "environment_id",
            "deployment_id",
            "last_event_id",
        ],
    );
    metadata["api"]["events"] = json!(true);
    cloud.register_function(
        "cloud::deployments::events",
        RegisterFunction::new_async(move |input: Value| {
            let (writer, gone) = (writer.clone(), gone.clone());
            async move {
                assert_eq!(input["deployment_id"], "d1");
                let Pipe { input, output } = writer.pipe().await?;
                tokio::spawn(async move {
                    let kept = [
                        json!({ "type": "log", "line": "Building http, api." }),
                        json!({ "type": "status", "status": "ready" }),
                    ];
                    for (i, data) in kept.into_iter().enumerate() {
                        let message =
                            json!({ "event": data["type"], "id": i.to_string(), "data": data });
                        output.send(&message).await.unwrap();
                    }
                    // The stream goes on (a ping every while) until its reader leaves.
                    for _ in 0..100 {
                        let ping =
                            json!({ "event": "ping", "id": null, "data": { "type": "ping" } });
                        if output.send(&ping).await.is_err() {
                            gone.store(true, Ordering::SeqCst);
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                });
                Ok::<Value, Error>(json!({ "events": input }))
            }
        })
        .metadata(metadata),
    );
    engine.function(&cloud, "cloud::deployments::events").await;

    let server = MockServer::start().await;
    let h = harness(&server.uri(), Arc::new(Fake::default()));
    *h.ctx.functions_at.lock().unwrap() = FunctionsAt::Engine(engine.url.clone());
    *h.ctx.cred.lock().unwrap() = Some(Credential {
        method: Method::ApiKey,
        token: "iii_functions_test".into(),
    });
    let events_path = "/teams/t1/environments/e1/deployments/d1/events";
    // Over HTTP until the socket has listed (wiremock has no such route: an error, as a cloud down).
    let mut read = None;
    for _ in 0..150 {
        if let Ok(events) = h.ctx.sse_events(events_path, Duration::from_secs(5)).await {
            read = Some(events);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        read.expect("read from the channel"),
        [
            json!({ "type": "log", "line": "Building http, api." }),
            json!({ "type": "status", "status": "ready" })
        ]
    );
    // The worker leaving ends the server's stream.
    for _ in 0..50 {
        if left.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        left.load(Ordering::SeqCst),
        "the stream outlived its reader"
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method.as_str() == "GET"),
        "reads only"
    );
}
