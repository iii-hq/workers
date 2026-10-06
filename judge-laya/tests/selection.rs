//! The worker's functions are registered whatever the judge hub selects: a
//! request naming laya loads its checkpoints on demand, and the hub's
//! selection only pins them loaded, never unregisters anything.
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, sync::mpsc, time::timeout};
use tokio_tungstenite::{accept_async, tungstenite::Message};

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

const HUB_ID: &str = "hub-selection-test";
const CHANGED: &str = "judge-laya::on-judge-config-change";

#[tokio::test]
async fn a_request_loads_the_checkpoints_while_another_provider_is_the_default() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let hub = Arc::new(Mutex::new(json!({"provider": "typesafe"})));
    let (frames_tx, mut frames) = mpsc::unbounded_channel::<Value>();
    let (send_tx, mut send) = mpsc::unbounded_channel::<Value>();
    let server_hub = hub.clone();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        socket
            .send(Message::Text(
                json!({"type":"workerregistered","worker_id":"selection-test"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let mut own = Value::Null;
        loop {
            tokio::select! {
                Some(frame) = send.recv() => {
                    socket.send(Message::Text(frame.to_string().into())).await.unwrap();
                }
                frame = socket.next() => {
                    let Some(Ok(frame)) = frame else { break };
                    let Message::Text(text) = frame else { continue };
                    let value: Value = serde_json::from_str(&text).unwrap();
                    let _ = frames_tx.send(value.clone());
                    if value["type"] != "invokefunction" || !value["invocation_id"].is_string() {
                        continue;
                    }
                    let result = match value["function_id"].as_str().unwrap() {
                        "judge::configuration-id" => json!({"id": HUB_ID}),
                        "configuration::get" if value["data"]["id"] == HUB_ID => {
                            json!({"value": server_hub.lock().unwrap().clone()})
                        }
                        "configuration::get" => json!({"value": own}),
                        "configuration::ensure" => {
                            own = value["data"]["initial_value"].clone();
                            json!({"action":"seeded","entry":{"id":value["data"]["id"],"value":own}})
                        }
                        _ => json!({}),
                    };
                    socket.send(Message::Text(json!({"type":"invocationresult","invocation_id":value["invocation_id"],"function_id":value["function_id"],"result":result}).to_string().into())).await.unwrap();
                }
            }
        }
    });
    let _worker = Worker(
        Command::new(env!("CARGO_BIN_EXE_judge-laya"))
            .env("III_URL", url)
            .env("III_CONFIG_NAME", "laya-selection-test")
            .env(
                "III_LAYA_CHECKPOINT_DIR",
                concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/tiny"),
            )
            .env("RUST_LOG", "info")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    // Wait for `pred` among the frames, failing on anything `forbid` matches.
    async fn expect(
        frames: &mut mpsc::UnboundedReceiver<Value>,
        within: Duration,
        pred: impl Fn(&Value) -> bool,
        forbid: impl Fn(&Value) -> bool,
    ) -> bool {
        timeout(within, async {
            while let Some(frame) = frames.recv().await {
                assert!(!forbid(&frame), "unexpected frame {frame}");
                if pred(&frame) {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false)
    }
    let registered =
        |id: &'static str| move |f: &Value| f["type"] == "registerfunction" && f["id"] == id;
    let bound = |f: &Value| {
        f["type"] == "registertrigger"
            && f["function_id"] == CHANGED
            && f["config"]["configuration_id"] == HUB_ID
    };
    // 1. The hub names typesafe: the functions register anyway, and the worker
    //    follows the hub.
    let (mut evaluate, mut following) = (false, false);
    timeout(Duration::from_secs(10), async {
        while !(evaluate && following) {
            let frame = frames.recv().await.expect("worker stays connected");
            evaluate |= registered("judge-laya::evaluate")(&frame);
            following |= bound(&frame);
        }
    })
    .await
    .expect("judge-laya::evaluate registers and the hub is followed while laya is not the default");
    // 2. A request naming laya loads the checkpoints and answers.
    send_tx
        .send(json!({"type":"invokefunction","invocation_id":"00000000-0000-0000-0000-000000000201","function_id":"judge-laya::models::list","data":{"timeout_ms":20000,"_caller_worker_id":"selection-test"}}))
        .unwrap();
    let answered = |f: &Value| {
        f["type"] == "invocationresult"
            && f["invocation_id"] == "00000000-0000-0000-0000-000000000201"
    };
    let mut reply = Value::Null;
    timeout(Duration::from_secs(20), async {
        while let Some(frame) = frames.recv().await {
            if answered(&frame) {
                reply = frame;
                break;
            }
        }
    })
    .await
    .expect("models::list answers once the checkpoints load");
    assert_eq!(reply["result"]["status"], "ok", "{reply}");
    // 3. The hub's selection moves (laya, preload_all, away): nothing unregisters.
    let changed = |n: u32| json!({"type":"invokefunction","invocation_id":format!("00000000-0000-0000-0000-00000000010{n}"),"function_id":CHANGED,"data":{}});
    for (n, selection) in [
        json!({"provider": "laya"}),
        json!({"provider": "semif", "preload_all": true}),
        json!({"provider": "semif"}),
    ]
    .into_iter()
    .enumerate()
    {
        *hub.lock().unwrap() = selection;
        send_tx.send(changed(n as u32 + 1)).unwrap();
        assert!(
            !expect(
                &mut frames,
                Duration::from_secs(2),
                |f| f["type"] == "unregisterfunction",
                |_| false
            )
            .await,
            "a hub selection change never unregisters the judge functions"
        );
    }
}
