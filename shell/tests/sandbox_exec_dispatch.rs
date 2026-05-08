//! Integration tests for the public sandbox-exec surface. The handler-
//! level dispatch via `target` field is exercised by the existing e2e
//! shell scripts and the unit tests in `src/functions/exec.rs::tests`;
//! this file cements the public-API surface (consumed via `iii_shell::*`)
//! so downstream consumers pinning these paths catch drift.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::IIIError;
use serde_json::{json, Value};
use uuid::Uuid;

use iii_shell::exec::sandbox::SandboxExecBackend;
use iii_shell::exec::ExecBackend;
use iii_shell::triggers::TriggerFwd;

struct StubFwd {
    captured: Mutex<Vec<(String, Value)>>,
    next: Mutex<Option<Result<Value, IIIError>>>,
}

impl StubFwd {
    fn new(resp: Result<Value, IIIError>) -> Arc<Self> {
        Arc::new(Self {
            captured: Mutex::new(Vec::new()),
            next: Mutex::new(Some(resp)),
        })
    }
    fn last(&self) -> (String, Value) {
        self.captured.lock().unwrap().last().cloned().unwrap()
    }
}

#[async_trait]
impl TriggerFwd for StubFwd {
    async fn trigger(&self, fid: &str, payload: Value) -> Result<Value, IIIError> {
        self.captured.lock().unwrap().push((fid.into(), payload));
        self.next
            .lock()
            .unwrap()
            .take()
            .expect("StubFwd: response already consumed")
    }
}

#[tokio::test]
async fn sandbox_backend_emits_sandbox_exec_payload() {
    let stub = StubFwd::new(Ok(json!({
        "stdout": "ok\n",
        "stderr": "",
        "exit_code": 0,
        "duration_ms": 12,
        "timed_out": false,
    })));
    let id = Uuid::new_v4();
    let b = SandboxExecBackend::new(stub.clone(), true, id);

    let out = b
        .run(&["echo".into(), "ok".into()], 5000)
        .await
        .expect("ok");
    assert_eq!(out.stdout, "ok\n");
    let (fid, payload) = stub.last();
    assert_eq!(fid, "sandbox::exec");
    assert_eq!(
        payload["sandbox_id"].as_str(),
        Some(id.to_string().as_str())
    );
    assert_eq!(payload["cmd"], "echo");
    assert_eq!(payload["timeout_ms"], 5000);
}
