//! Scriptable storage boundary; production SessionStore talks through the SDK.
use futures::{SinkExt, StreamExt};
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_tungstenite::{accept_async, tungstenite::Message};

pub const RESOLVE: &str = "secrets::resolve";
pub const SECRET_CAS: &str = "secrets::compare-and-set";
pub const STATE_CAS: &str = "provider-openai-codex::state::compare-and-set";

#[derive(Clone)]
pub struct Failure {
    pub function: &'static str,
    pub after_write: bool,
    pub skip: usize,
}

#[derive(Default)]
pub struct BusState {
    pub legacy: Value,
    pub secret: Option<String>,
    pub calls: Vec<String>,
    pub failure: Option<Failure>,
    pub resolve_error: Option<&'static str>,
    pub change_before_cleanup: bool,
    pub flushed: bool,
    pub omit_durable_ack: bool,
}

impl BusState {
    fn invoke(&mut self, id: &str, data: &Value) -> Result<Value, &'static str> {
        if id == "engine::workers::register" {
            return Ok(json!({}));
        }
        self.calls.push(id.into());
        let failure = if let Some(fail) = self.failure.as_mut().filter(|f| f.function == id) {
            if fail.skip > 0 {
                fail.skip -= 1;
                None
            } else {
                self.failure.take()
            }
        } else {
            None
        };
        if failure.as_ref().is_some_and(|f| !f.after_write) {
            return Err("STORAGE_UNAVAILABLE");
        }
        let result = match id {
            "state::claim-namespace" => json!({"claimed":true}),
            "provider-openai-codex::state::get" => self.legacy.clone(),
            RESOLVE => {
                if let Some(code) = self.resolve_error {
                    return Err(code);
                }
                match &self.secret {
                    Some(value) => json!({"name":"PROVIDER_OPENAI_CODEX_SESSION", "value":value}),
                    None => return Err("SECRET_NOT_FOUND"),
                }
            }
            SECRET_CAS => {
                assert_eq!(data["name"], "PROVIDER_OPENAI_CODEX_SESSION");
                let swapped = data["expected"].as_str() == self.secret.as_deref();
                if swapped {
                    if self.secret.is_none() {
                        assert_eq!(data["consumers"], json!(["provider-openai-codex"]));
                    }
                    self.secret = Some(data["value"].as_str().unwrap().into());
                }
                json!({"swapped":swapped})
            }
            STATE_CAS => {
                assert_eq!(data["durable"], true);
                assert!(data["value"].is_null());
                // Cleanup must follow successful readback of the copied secret.
                assert!(self.secret.is_some());
                if self.change_before_cleanup {
                    self.legacy["revision"] = json!("concurrent-revision");
                    self.change_before_cleanup = false;
                }
                let swapped = data["expected"] == self.legacy;
                if swapped {
                    self.legacy = Value::Null;
                }
                self.flushed = failure.is_none();
                let mut result = json!({"swapped":swapped,"current":self.legacy});
                if !self.omit_durable_ack {
                    result["durable"] = json!(true);
                }
                result
            }
            _ => panic!("unexpected RPC {id}"),
        };
        if failure.is_some() {
            Err("STORAGE_UNAVAILABLE")
        } else {
            Ok(result)
        }
    }
}

pub struct SessionBus {
    pub client: IIIClient,
    pub state: Arc<Mutex<BusState>>,
    task: JoinHandle<()>,
}

impl SessionBus {
    pub async fn start(legacy: Value, secret: Option<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(BusState {
            legacy,
            secret,
            ..Default::default()
        }));
        let shared = state.clone();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            socket
                .send(Message::Text(
                    json!({"type":"workerregistered","worker_id":"storage-test"})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(frame)) = socket.next().await {
                let Message::Text(text) = frame else {
                    continue;
                };
                let frame: Value = serde_json::from_str(&text).unwrap();
                if frame["type"] != "invokefunction" || !frame["invocation_id"].is_string() {
                    continue;
                }
                let response = shared
                    .lock()
                    .unwrap()
                    .invoke(frame["function_id"].as_str().unwrap(), &frame["data"]);
                let mut reply = json!({
                    "type":"invocationresult", "invocation_id":frame["invocation_id"],
                    "function_id":frame["function_id"]
                });
                match response {
                    Ok(value) => reply["result"] = value,
                    Err(code) => {
                        reply["error"] = json!({"code":code,"message":"synthetic storage failure"})
                    }
                }
                if socket
                    .send(Message::Text(reply.to_string().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let client = register_worker(&url, InitOptions::default());
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while client.get_connection_state() != iii_sdk::runtime::IIIConnectionState::Connected {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        Self {
            client,
            state,
            task,
        }
    }
}

impl Drop for SessionBus {
    fn drop(&mut self) {
        self.client.shutdown();
        self.task.abort();
    }
}
