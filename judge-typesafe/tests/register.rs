use futures_util::{SinkExt, StreamExt};
#[path = "support/fake_engine.rs"]
mod fake_engine;
use iii_sdk::{register_worker, InitOptions};
use judge_typesafe::{JevClient, JevConfig};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
    task::{JoinHandle, JoinSet},
    time::timeout,
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

async fn invoke(payload: Value) -> (Value, Value) {
    invoke_function("judge-typesafe::evaluate", payload).await
}

async fn invoke_function(function_id: &'static str, payload: Value) -> (Value, Value) {
    invoke_client(
        function_id,
        payload,
        judge_typesafe::configuration::new_cell(JevConfig::default()),
        JevClient::new(None),
    )
    .await
}

async fn invoke_client(
    function_id: &'static str,
    payload: Value,
    config: judge_typesafe::SharedConfig,
    client: JevClient,
) -> (Value, Value) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut payload = Some(payload);
    let engine = fake_engine::start(move |message| {
        if message["type"] == "registerfunction" && message["id"] == function_id {
            tx.send(message).unwrap();
            let data = payload.take().expect("registered once");
            vec![json!({"type":"invokefunction","invocation_id":"00000000-0000-0000-0000-000000000001","function_id":function_id,"data":data})]
        } else {
            if message["type"] == "invocationresult" {
                tx.send(message).unwrap();
            }
            vec![]
        }
    })
    .await;
    let iii = Arc::new(register_worker(&engine.url, InitOptions::default()));
    judge_typesafe::register(&iii, config, client);
    let registration = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("requested function must be registered")
        .unwrap();
    let response = timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    iii.shutdown_async().await;
    (registration, response)
}

#[tokio::test]
async fn models_registration_has_strict_schemas_and_accepts_root_metadata() {
    let (registration, response) = invoke_function(
        "judge-typesafe::models::list",
        json!({"_caller_worker_id":"engine-worker"}),
    )
    .await;
    assert_eq!(
        registration["request_format"]["additionalProperties"],
        false
    );
    assert!(registration["request_format"]["properties"]["timeout_ms"].is_object());
    assert!(registration["request_format"]["properties"]["request_id"].is_object());
    assert!(registration["request_format"]["properties"]["options"].is_object());
    for field in ["api_key", "endpoint", "_caller_worker_id", "model"] {
        assert!(registration["request_format"]["properties"]
            .get(field)
            .is_none());
    }
    assert_eq!(
        registration["response_format"]["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(response.get("error").is_none());
    assert_eq!(response["result"]["code"], "missing_key");
    assert_eq!(response["result"]["stats"]["attempts"], 0);
}

#[tokio::test]
async fn models_registration_rejects_unknown_fields_and_malformed_timeouts() {
    for payload in [
        json!({"_caller_worker_id":"engine-worker","api_key":"test-marker"}),
        json!({"endpoint":"http://localhost"}),
        json!({"_caller_namespace":"unknown"}),
        json!({"model":"jev-latest"}),
        json!({"timeout_ms":null}),
        json!({"timeout_ms":0}),
        json!({"timeout_ms":1.5}),
    ] {
        let (_, response) = invoke_function("judge-typesafe::models::list", payload).await;
        assert!(response.get("error").is_none());
        assert_eq!(response["result"]["code"], "invalid_request");
        assert_eq!(response["result"]["stats"]["attempts"], 0);
        assert!(!response.to_string().contains("test-marker"));
    }
}

#[tokio::test]
async fn null_score_levels_are_rejected_before_provider_calls() {
    let mut payload = ticket();
    payload["evaluations"][0]["questions"]["urgent"] =
        json!({"type":"score","criteria":[null,"blocking"]});
    let (_, response) = invoke(payload).await;
    assert_eq!(response["result"]["code"], "invalid_request");
    assert_eq!(response["result"]["stats"]["attempts"], 0);
}

#[tokio::test]
async fn registered_handlers_apply_current_operator_limits() {
    let cell = judge_typesafe::configuration::new_cell(JevConfig::default());
    let client = JevClient::new(None);
    for (function_id, payload) in [
        ("judge-typesafe::evaluate", ticket()),
        ("judge-typesafe::models::list", json!({"timeout_ms":1000})),
    ] {
        assert!(
            judge_typesafe::configuration::apply_config(
                &cell,
                JevConfig {
                    max_timeout_ms: 500,
                    ..JevConfig::default()
                }
            )
            .await
        );
        let (_, response) =
            invoke_client(function_id, payload.clone(), cell.clone(), client.clone()).await;
        assert_eq!(response["result"]["code"], "invalid_request");
        assert_eq!(response["result"]["stats"]["attempts"], 0);
        assert!(judge_typesafe::configuration::apply_config(&cell, JevConfig::default()).await);
        let (_, response) = invoke_client(function_id, payload, cell.clone(), client.clone()).await;
        assert_eq!(response["result"]["code"], "missing_key");
    }
    assert!(
        judge_typesafe::configuration::apply_config(
            &cell,
            JevConfig {
                max_request_bytes: 32,
                ..JevConfig::default()
            }
        )
        .await
    );
    let (_, response) = invoke_client("judge-typesafe::evaluate", ticket(), cell, client).await;
    assert_eq!(response["result"]["code"], "payload_too_large");
    assert_eq!(response["result"]["stats"]["attempts"], 0);
}

fn ticket() -> Value {
    json!({"timeout_ms":1000,"evaluations":[{"id":"ticket","state":{"text":"Cannot sign in"},"questions":{"urgent":{"type":"noul","instructions":"Is this urgent?"}}}]})
}

#[tokio::test]
async fn registered_function_has_typed_schemas_and_returns_missing_key_envelope() {
    let (registration, response) = invoke(ticket()).await;
    assert_eq!(
        registration["request_format"]["additionalProperties"],
        false
    );
    assert!(registration["request_format"]["properties"]["evaluations"].is_object());
    assert!(registration["request_format"]["properties"]["request_id"].is_object());
    assert!(registration["request_format"]["properties"]["options"].is_object());
    assert!(registration["request_format"]["properties"]
        .get("api_key")
        .is_none());
    assert!(registration["request_format"]["properties"]
        .get("endpoint")
        .is_none());
    let variants = registration["response_format"]["oneOf"].as_array().unwrap();
    assert_eq!(variants.len(), 2);
    assert!(response.get("error").is_none());
    assert_eq!(response["result"]["status"], "error");
    assert_eq!(response["result"]["code"], "missing_key");
    assert_eq!(response["result"]["stats"]["attempts"], 0);
}

#[tokio::test]
async fn engine_caller_metadata_is_removed_only_at_registration_boundary() {
    let mut payload = ticket();
    payload["_caller_worker_id"] = json!("00000000-0000-0000-0000-000000000002");
    assert!(serde_json::from_value::<judge_contract::EvaluateRequest>(payload.clone()).is_err());
    let (registration, response) = invoke(payload).await;
    assert!(registration["request_format"]["properties"]
        .get("_caller_worker_id")
        .is_none());
    assert_eq!(
        registration["request_format"]["additionalProperties"],
        false
    );
    assert!(
        response.get("error").is_none(),
        "engine metadata must reach the handler: {response}"
    );
    assert_eq!(response["result"]["code"], "missing_key");
    assert_eq!(response["result"]["stats"]["attempts"], 0);
}

#[tokio::test]
async fn registration_rejects_unknown_fields_after_removing_only_reserved_metadata() {
    for (field, value) in [
        ("api_key", json!("test-marker")),
        ("endpoint", json!("http://127.0.0.1")),
        ("_caller_namespace", json!("unrecognized")),
    ] {
        let mut payload = ticket();
        payload["_caller_worker_id"] = json!("test-worker");
        payload[field] = value;
        let (_, response) = invoke(payload).await;
        assert!(
            response.get("error").is_none(),
            "invalid request must use the typed envelope"
        );
        assert_eq!(response["result"]["code"], "invalid_request");
        assert_eq!(response["result"]["stats"]["attempts"], 0);
        assert!(!response.to_string().contains("test-marker"));
    }
    let mut payload = ticket();
    payload["evaluations"][0]["_caller_worker_id"] = json!("not-top-level-metadata");
    let (_, response) = invoke(payload).await;
    assert_eq!(response["result"]["code"], "invalid_request");
}

#[tokio::test]
async fn cancel_registration_has_strict_schemas_and_unknown_ids_return_false() {
    let (registration, response) = invoke_function(
        "judge-typesafe::cancel",
        json!({"request_id":"completed", "_caller_worker_id":"owner"}),
    )
    .await;
    assert_eq!(
        registration["request_format"]["additionalProperties"],
        false
    );
    assert_eq!(
        registration["request_format"]["required"],
        json!(["request_id"])
    );
    assert_eq!(
        registration["request_format"]["properties"]["request_id"]["type"],
        "string"
    );
    assert!(registration["request_format"]["properties"]
        .get("_caller_worker_id")
        .is_none());
    assert_eq!(
        registration["response_format"]["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(response.get("error").is_none());
    assert_eq!(response["result"], json!({"status":"ok","cancelled":false}));
}

#[tokio::test]
async fn cancel_rejects_missing_identity_invalid_ids_and_unknown_fields() {
    for payload in [
        json!({"request_id":"request"}),
        json!({"request_id":"request", "_caller_worker_id":null}),
        json!({"request_id":"request", "_caller_worker_id":17}),
        json!({"request_id":"request", "_caller_worker_id":"  "}),
        json!({"_caller_worker_id":"owner"}),
        json!({"request_id":"", "_caller_worker_id":"owner"}),
        json!({"request_id":"\nrequest", "_caller_worker_id":"owner"}),
        json!({"request_id":"café", "_caller_worker_id":"owner"}),
        json!({"request_id":"x".repeat(513), "_caller_worker_id":"owner"}),
        json!({"request_id":"request", "_caller_worker_id":"owner", "_caller_namespace":"local"}),
        json!({"request_id":"request", "_caller_worker_id":"owner", "api_key":"test-marker"}),
        json!({"request_id":"request", "_caller_worker_id":"owner", "options":{}}),
    ] {
        let (_, response) = invoke_function("judge-typesafe::cancel", payload).await;
        assert!(response.get("error").is_none());
        assert_eq!(
            response["result"],
            json!({"status":"error","code":"invalid_request"})
        );
        assert!(!response.to_string().contains("test-marker"));
    }
}

#[tokio::test]
async fn identified_bus_calls_require_trusted_identity() {
    for (function_id, mut payload) in [
        ("judge-typesafe::evaluate", ticket()),
        ("judge-typesafe::models::list", json!({})),
    ] {
        payload["request_id"] = json!("request");
        for caller in [None, Some(Value::Null), Some(json!(42)), Some(json!(" \t"))] {
            let mut payload = payload.clone();
            if let Some(caller) = caller {
                payload["_caller_worker_id"] = caller;
            }
            let (_, response) = invoke_function(function_id, payload).await;
            assert_eq!(response["result"]["code"], "invalid_request");
            assert_eq!(response["result"]["stats"]["attempts"], 0);
        }
        payload["_caller_worker_id"] = json!("owner");
        let (_, response) = invoke_function(function_id, payload).await;
        assert_eq!(response["result"]["code"], "missing_key");
    }
}

/// A WebSocket engine fixture that keeps several invocations in flight.
struct LiveBus {
    iii: Arc<iii_sdk::IIIClient>,
    server: JoinHandle<()>,
    commands: mpsc::UnboundedSender<Value>,
    incoming: mpsc::UnboundedReceiver<Value>,
    responses: BTreeMap<String, Value>,
    next_id: u64,
}

impl LiveBus {
    async fn start(config: judge_typesafe::SharedConfig, client: JevClient) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("ws://{}", listener.local_addr().unwrap());
        let (commands, mut outgoing) = mpsc::unbounded_channel::<Value>();
        let (events, incoming) = mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            socket
                .send(Message::Text(
                    json!({"type":"workerregistered", "worker_id":"jev-test-worker"})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            loop {
                tokio::select! {
                    command = outgoing.recv() => {
                        let Some(command) = command else { break };
                        socket.send(Message::Text(command.to_string().into())).await.unwrap();
                    }
                    frame = socket.next() => {
                        match frame {
                            Some(Ok(Message::Text(text))) => {
                                let message: Value = serde_json::from_str(&text).unwrap();
                                if message["type"] == "registerfunction" || message["type"] == "invocationresult" {
                                    events.send(message).unwrap();
                                }
                            }
                            Some(Ok(Message::Ping(bytes))) => socket.send(Message::Pong(bytes)).await.unwrap(),
                            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                            _ => {}
                        }
                    }
                }
            }
        });
        let iii = Arc::new(register_worker(&address, InitOptions::default()));
        judge_typesafe::register(&iii, config, client);
        let mut bus = Self {
            iii,
            server,
            commands,
            incoming,
            responses: BTreeMap::new(),
            next_id: 0,
        };
        let mut registered = std::collections::BTreeSet::new();
        while registered.len() < 3 {
            let registration = timeout(Duration::from_secs(3), bus.incoming.recv())
                .await
                .expect("all JEV functions registered")
                .unwrap();
            registered.insert(registration["id"].as_str().unwrap().to_owned());
        }
        assert_eq!(
            registered,
            [
                "judge-typesafe::evaluate",
                "judge-typesafe::models::list",
                "judge-typesafe::cancel"
            ]
            .map(str::to_owned)
            .into_iter()
            .collect()
        );
        bus
    }

    fn send(&mut self, function_id: &str, payload: Value) -> String {
        self.next_id += 1;
        let invocation_id = format!("00000000-0000-0000-0000-{:012}", self.next_id);
        self.commands.send(json!({"type":"invokefunction", "invocation_id":invocation_id, "function_id":function_id, "data":payload})).unwrap();
        invocation_id
    }

    async fn response(&mut self, invocation_id: &str) -> Value {
        timeout(Duration::from_secs(3), async {
            loop {
                if let Some(response) = self.responses.remove(invocation_id) {
                    assert!(response.get("error").is_none(), "{response}");
                    return response["result"].clone();
                }
                let response = self.incoming.recv().await.unwrap();
                self.responses.insert(
                    response["invocation_id"].as_str().unwrap().to_owned(),
                    response,
                );
            }
        })
        .await
        .expect("invocation completes promptly")
    }

    async fn call(&mut self, function_id: &str, payload: Value) -> Value {
        let invocation_id = self.send(function_id, payload);
        self.response(&invocation_id).await
    }

    async fn cancel(&mut self, caller: &str, request_id: &str) -> Value {
        self.call(
            "judge-typesafe::cancel",
            json!({"_caller_worker_id":caller, "request_id":request_id}),
        )
        .await
    }
}

impl Drop for LiveBus {
    fn drop(&mut self) {
        self.iii.shutdown();
        self.server.abort();
    }
}

struct StalledProvider {
    endpoint: String,
    task: JoinHandle<()>,
    started: mpsc::UnboundedReceiver<()>,
    closed: mpsc::UnboundedReceiver<()>,
}

impl StalledProvider {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/evaluate", listener.local_addr().unwrap());
        let (started_tx, started) = mpsc::unbounded_channel();
        let (closed_tx, closed) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    connection = listener.accept() => {
                        let (mut stream, _) = connection.unwrap();
                        let started = started_tx.clone();
                        let closed = closed_tx.clone();
                        connections.spawn(async move {
                            let mut bytes = Vec::new();
                            let mut buffer = [0; 4096];
                            let length = loop {
                                let count = stream.read(&mut buffer).await.unwrap();
                                assert_ne!(count, 0);
                                bytes.extend_from_slice(&buffer[..count]);
                                if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                                    let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                                    let body = headers.lines().find_map(|line| line.strip_prefix("content-length: ")).unwrap_or("0").parse::<usize>().unwrap();
                                    break end + 4 + body;
                                }
                            };
                            while bytes.len() < length {
                                let count = stream.read(&mut buffer).await.unwrap();
                                assert_ne!(count, 0);
                                bytes.extend_from_slice(&buffer[..count]);
                            }
                            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 10000\r\nConnection: close\r\n\r\n{").await.unwrap();
                            started.send(()).unwrap();
                            let _ = stream.read_to_end(&mut Vec::new()).await;
                            let _ = closed.send(());
                        });
                    }
                    completed = connections.join_next(), if !connections.is_empty() => {
                        completed.unwrap().unwrap();
                    }
                }
            }
        });
        Self {
            endpoint,
            task,
            started,
            closed,
        }
    }

    async fn started(&mut self) {
        timeout(Duration::from_secs(3), self.started.recv())
            .await
            .expect("provider request arrives")
            .unwrap();
    }

    async fn closed(&mut self) {
        timeout(Duration::from_secs(3), self.closed.recv())
            .await
            .expect("cancelled provider connection closes")
            .unwrap();
    }
}

impl Drop for StalledProvider {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn identified_payload(function_id: &str, caller: &str, request_id: &str) -> Value {
    let mut payload = if function_id == "judge-typesafe::evaluate" {
        ticket()
    } else {
        json!({})
    };
    payload["timeout_ms"] = json!(10_000);
    payload["request_id"] = json!(request_id);
    payload["_caller_worker_id"] = json!(caller);
    payload
}

#[tokio::test]
async fn bus_cancellation_shares_ids_between_evaluate_and_models_and_releases_them() {
    let mut provider = StalledProvider::start().await;
    let config = judge_typesafe::configuration::new_cell(JevConfig {
        api_key: Some("test-key".into()),
        ..JevConfig::default()
    });
    let client = JevClient::with_endpoint(None, provider.endpoint.clone());
    let mut bus = LiveBus::start(config, client).await;
    for (function_id, other) in [
        ("judge-typesafe::evaluate", "judge-typesafe::models::list"),
        ("judge-typesafe::models::list", "judge-typesafe::evaluate"),
    ] {
        let invocation = bus.send(
            function_id,
            identified_payload(function_id, "owner", "shared"),
        );
        provider.started().await;
        assert_eq!(
            bus.cancel("stranger", "shared").await,
            json!({"status":"ok", "cancelled":false})
        );
        let duplicate = bus
            .call(other, identified_payload(other, "owner", "shared"))
            .await;
        assert_eq!(duplicate["code"], "invalid_request");
        assert_eq!(duplicate["stats"]["attempts"], 0);
        assert_eq!(
            bus.cancel("owner", "shared").await,
            json!({"status":"ok", "cancelled":true})
        );
        let response = bus.response(&invocation).await;
        assert_eq!(response["code"], "cancelled");
        assert_eq!(response["stats"]["attempts"], 1);
        assert!(response.get("results").is_none());
        provider.closed().await;
        assert_eq!(
            bus.cancel("owner", "shared").await,
            json!({"status":"ok", "cancelled":false})
        );
    }
}

#[tokio::test]
async fn bus_callers_with_the_same_id_can_only_cancel_their_own_call() {
    let mut provider = StalledProvider::start().await;
    let config = judge_typesafe::configuration::new_cell(JevConfig {
        api_key: Some("test-key".into()),
        ..JevConfig::default()
    });
    let mut bus = LiveBus::start(
        config,
        JevClient::with_endpoint(None, provider.endpoint.clone()),
    )
    .await;
    let first = bus.send(
        "judge-typesafe::evaluate",
        identified_payload("judge-typesafe::evaluate", "first", "shared"),
    );
    provider.started().await;
    let second = bus.send(
        "judge-typesafe::models::list",
        identified_payload("judge-typesafe::models::list", "second", "shared"),
    );
    provider.started().await;
    assert_eq!(
        bus.cancel("first", "shared").await,
        json!({"status":"ok", "cancelled":true})
    );
    assert_eq!(bus.response(&first).await["code"], "cancelled");
    provider.closed().await;
    assert_eq!(
        bus.cancel("first", "shared").await,
        json!({"status":"ok", "cancelled":false})
    );
    assert_eq!(
        bus.cancel("second", "shared").await,
        json!({"status":"ok", "cancelled":true})
    );
    assert_eq!(bus.response(&second).await["code"], "cancelled");
    provider.closed().await;
}

#[test]
fn provider_ids_follow_the_hub_convention() {
    use judge_contract::{
        provider_function_id, CANCEL_FUNCTION_ID, FUNCTION_ID, MODELS_FUNCTION_ID,
    };
    use judge_typesafe::{CANCEL_ID, EVALUATE_ID, MODELS_ID, PROVIDER};
    assert_eq!(provider_function_id(PROVIDER, FUNCTION_ID), EVALUATE_ID);
    assert_eq!(
        provider_function_id(PROVIDER, MODELS_FUNCTION_ID),
        MODELS_ID
    );
    assert_eq!(
        provider_function_id(PROVIDER, CANCEL_FUNCTION_ID),
        CANCEL_ID
    );
}
