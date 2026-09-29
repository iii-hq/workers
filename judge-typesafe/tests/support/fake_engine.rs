//! One-connection fake engine for bus-contract tests: acknowledges the worker,
//! hands every text frame to `handle` and sends back the frames it returns.
//! Shared by the hub and provider test crates through `#[path]`.
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_tungstenite::{accept_async, tungstenite::Message};

pub struct FakeEngine {
    pub url: String,
    task: JoinHandle<()>,
}

impl Drop for FakeEngine {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn start(mut handle: impl FnMut(Value) -> Vec<Value> + Send + 'static) -> FakeEngine {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let registered = json!({"type":"workerregistered","worker_id":"fake-engine-worker"});
        socket
            .send(Message::Text(registered.to_string().into()))
            .await
            .unwrap();
        while let Some(Ok(frame)) = socket.next().await {
            let Message::Text(text) = frame else {
                continue;
            };
            for reply in handle(serde_json::from_str(&text).unwrap()) {
                socket
                    .send(Message::Text(reply.to_string().into()))
                    .await
                    .unwrap();
            }
        }
    });
    FakeEngine { url, task }
}
