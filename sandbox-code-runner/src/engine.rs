//! The single seam between this worker and the iii bus.
//!
//! Everything that talks to the engine goes through [`Engine`] — the
//! `sandbox::*` calls out AND the dynamic function registrations in — so the
//! manager is testable without a live engine. The production implementation
//! is [`IIIEngine`]; tests use `FakeEngine`. Ported from node-engine's seam,
//! minus its per-runtime worker connections (no register_worker in v1).

use std::sync::Arc;

use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::error::{classify_probe_error, ProbeOutcome};

pub type CallResult = Result<Value, String>;

/// A registered function exposed to the bus. Dynamic registrations are
/// `Value`-in / `Value`-out by nature: the schema lives in the handler
/// source inside the VM, not in Rust types.
pub type ProxyHandler = Arc<dyn Fn(Value) -> BoxFuture<'static, CallResult> + Send + Sync>;

pub type UnregisterFn = Box<dyn Fn() + Send + Sync>;

pub trait Engine: Send + Sync + 'static {
    /// Invoke any engine function. Unrestricted by design — the deployment's
    /// `iii-permissions.yaml` is the gate.
    fn call(
        &self,
        fn_id: String,
        payload: Value,
        timeout_ms: u64,
    ) -> BoxFuture<'static, CallResult>;

    /// Publish a dynamically-created function. The returned closure removes it.
    fn register(
        &self,
        fn_id: String,
        description: Option<String>,
        handler: ProxyHandler,
    ) -> UnregisterFn;
}

/// Stands in when a caller registers without a description — a registration
/// with no description at all is indistinguishable from a missing function in
/// the catalog, which is worse than a generic line.
pub const DEFAULT_DYNAMIC_DESC: &str =
    "Registered at runtime by sandbox-code-runner; the handler runs inside an iii-sandbox microVM.";

pub struct IIIEngine {
    iii: Arc<IIIClient>,
}

impl IIIEngine {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

/// iii-sandbox is engine-owned: its `sandbox::*` functions exist only in
/// `default`, whatever namespace this worker runs in (a Compose project's,
/// say). Every other id keeps the SDK's routing, which already sends
/// `engine::*` to `default`.
fn call_namespace(fn_id: &str) -> Option<&'static str> {
    fn_id.starts_with("sandbox::").then_some("default")
}

impl Engine for IIIEngine {
    fn call(
        &self,
        fn_id: String,
        payload: Value,
        timeout_ms: u64,
    ) -> BoxFuture<'static, CallResult> {
        let iii = self.iii.clone();
        Box::pin(async move {
            let namespace = call_namespace(&fn_id);
            let request = TriggerRequest {
                function_id: fn_id,
                payload,
                action: None,
                timeout_ms: Some(timeout_ms),
            };
            match namespace {
                Some(namespace) => iii.trigger(request.namespace(namespace)).await,
                None => iii.trigger(request).await,
            }
            .map_err(|e| e.to_string())
        })
    }

    fn register(
        &self,
        fn_id: String,
        description: Option<String>,
        handler: ProxyHandler,
    ) -> UnregisterFn {
        let desc = description.unwrap_or_else(|| DEFAULT_DYNAMIC_DESC.to_string());
        let function_ref = self.iii.register_function(
            &fn_id,
            RegisterFunction::new_async(move |req: Value| {
                let handler = handler.clone();
                async move { handler(req).await.map_err(Error::Handler) }
            })
            .description(desc),
        );
        Box::new(move || function_ref.unregister())
    }
}

/// What the startup probe learned about the sandbox adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxProbe {
    /// An adapter is connected to the router.
    Ready,
    /// The engine answered and there is no adapter.
    Missing,
    /// An adapter is connected but implements another contract major, so the
    /// router refuses every call with S302; carries why.
    Incompatible(String),
    /// The engine did not answer; carries its raw error.
    Unverified(String),
}

/// Ask the sandbox router whether an adapter is connected. Engines without
/// the router get the older check, whether `sandbox::create` is registered.
/// Only a clean "not found" on the router id falls back: any other failure
/// (rbac, timeout) says nothing about the router being absent, and the
/// fallback would then answer for a function the router owns.
pub async fn probe_sandbox(engine: &dyn Engine) -> SandboxProbe {
    const ROUTER: &str = "sandbox::adapter::info";
    match engine.call(ROUTER.to_string(), json!({}), 5_000).await {
        Ok(v) => match v.get("info").filter(|i| !i.is_null()) {
            None => SandboxProbe::Missing,
            Some(info) if info.pointer("/contract/major") == v.pointer("/contract/major") => {
                SandboxProbe::Ready
            }
            Some(info) => SandboxProbe::Incompatible(format!(
                "adapter implements sandbox contract {}, but this engine needs major {}",
                info["contract"], v["contract"]["major"]
            )),
        },
        Err(raw) if classify_probe_error(&raw, ROUTER) == ProbeOutcome::Free => {
            let legacy = engine
                .call(
                    "engine::functions::info".to_string(),
                    json!({ "function_id": "sandbox::create" }),
                    5_000,
                )
                .await;
            match legacy {
                Ok(_) => SandboxProbe::Ready,
                // Same disambiguation as the register-time probe (see
                // `classify_probe_error`): a "not found" naming
                // `engine::functions::info` itself means this engine cannot
                // dispatch the probe, not that `sandbox::create` is absent.
                Err(raw) if classify_probe_error(&raw, "sandbox::create") == ProbeOutcome::Free => {
                    SandboxProbe::Missing
                }
                Err(raw) => SandboxProbe::Unverified(raw),
            }
        }
        Err(raw) => SandboxProbe::Unverified(raw),
    }
}

/// What each id was published with: `(id, description, handler)`.
#[cfg(test)]
type RegisteredHandlers = Arc<std::sync::Mutex<Vec<(String, Option<String>, ProxyHandler)>>>;

/// Per-id computed responders — the response is built from the request
/// payload rather than canned.
#[cfg(test)]
type Responders = std::sync::Mutex<
    std::collections::HashMap<String, Arc<dyn Fn(&Value) -> CallResult + Send + Sync>>,
>;

#[cfg(test)]
#[derive(Default)]
pub struct FakeEngine {
    responses: std::sync::Mutex<std::collections::HashMap<String, CallResult>>,
    /// Per-id queue of responses, indexed by how many times that id has
    /// already been called (pinned at the last entry once exhausted) — models
    /// an answer that changes across calls, e.g. an exec that succeeds once
    /// and then reports the sandbox gone.
    sequenced_responses:
        std::sync::Mutex<std::collections::HashMap<String, (Vec<CallResult>, usize)>>,
    calls: std::sync::Mutex<Vec<(String, Value)>>,
    /// Computed responders, checked FIRST: the response is built from the
    /// request payload. The manager generates a random sentinel per exec, so
    /// a canned response cannot contain it — only a responder that reads the
    /// sentinel out of the exec args can produce matching stdout.
    responders: Responders,
    /// `Arc` so the `'static` unregister closure can remove its own entry — a
    /// fake whose unregister only counted would let a test assert "torn-down
    /// functions are uncallable" and pass without teardown removing anything.
    handlers: RegisteredHandlers,
    unregisters: Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(test)]
impl FakeEngine {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn with_response(&self, fn_id: &str, result: CallResult) {
        self.responses
            .lock()
            .unwrap()
            .insert(fn_id.to_string(), result);
    }

    /// Queue `results` for `fn_id`, one per call. Once exhausted, later calls
    /// keep getting the LAST entry rather than falling through — a queue that
    /// quietly stopped answering would make callers look timed out.
    pub fn with_response_sequence(&self, fn_id: &str, results: Vec<CallResult>) {
        assert!(!results.is_empty(), "an empty sequence answers nothing");
        self.sequenced_responses
            .lock()
            .unwrap()
            .insert(fn_id.to_string(), (results, 0));
    }

    /// Compute the response for `fn_id` from each request's payload.
    /// Takes precedence over `with_response`/`with_response_sequence`.
    pub fn with_responder(
        &self,
        fn_id: &str,
        f: impl Fn(&Value) -> CallResult + Send + Sync + 'static,
    ) {
        self.responders
            .lock()
            .unwrap()
            .insert(fn_id.to_string(), Arc::new(f));
    }

    pub fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().unwrap().clone()
    }

    pub fn registered_ids(&self) -> Vec<String> {
        self.handlers
            .lock()
            .unwrap()
            .iter()
            .map(|(id, _, _)| id.clone())
            .collect()
    }

    /// What each id was published with — the fake's only view of the
    /// description reaching the bus, so a test can prove it is not dropped.
    pub fn registered_descriptions(&self) -> Vec<(String, Option<String>)> {
        self.handlers
            .lock()
            .unwrap()
            .iter()
            .map(|(id, desc, _)| (id.clone(), desc.clone()))
            .collect()
    }

    pub fn unregister_count(&self) -> usize {
        self.unregisters.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Drive a registered proxy the way the engine would.
    pub async fn invoke(&self, fn_id: &str, payload: Value) -> CallResult {
        let handler = self
            .handlers
            .lock()
            .unwrap()
            .iter()
            .find(|(id, _, _)| id == fn_id)
            .map(|(_, _, h)| h.clone());
        match handler {
            Some(h) => h(payload).await,
            None => Err(format!("no such registered function: {fn_id}")),
        }
    }
}

#[cfg(test)]
impl Engine for FakeEngine {
    fn call(
        &self,
        fn_id: String,
        payload: Value,
        _timeout_ms: u64,
    ) -> BoxFuture<'static, CallResult> {
        self.calls
            .lock()
            .unwrap()
            .push((fn_id.clone(), payload.clone()));

        if let Some(f) = self.responders.lock().unwrap().get(&fn_id).cloned() {
            let out = f(&payload);
            return Box::pin(async move { out });
        }

        {
            let mut sequenced = self.sequenced_responses.lock().unwrap();
            if let Some((seq, next)) = sequenced.get_mut(&fn_id) {
                let i = (*next).min(seq.len() - 1);
                *next += 1;
                let out = seq[i].clone();
                return Box::pin(async move { out });
            }
        }

        let out = self
            .responses
            .lock()
            .unwrap()
            .get(&fn_id)
            .cloned()
            .unwrap_or_else(|| Err(format!("no such function: {fn_id}")));
        Box::pin(async move { out })
    }

    fn register(
        &self,
        fn_id: String,
        description: Option<String>,
        handler: ProxyHandler,
    ) -> UnregisterFn {
        self.handlers
            .lock()
            .unwrap()
            .push((fn_id.clone(), description, handler));
        let counter = self.unregisters.clone();
        let handlers = self.handlers.clone();
        Box::new(move || {
            // Remove, not just count: the fake must be able to show that an
            // unregistered id is genuinely gone.
            handlers.lock().unwrap().retain(|(id, _, _)| *id != fn_id);
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_calls_resolve_in_default_and_nothing_else_is_pinned() {
        for fn_id in [
            "sandbox::create",
            "sandbox::exec",
            "sandbox::fs::write",
            "sandbox::list",
        ] {
            assert_eq!(call_namespace(fn_id), Some("default"), "{fn_id}");
        }
        // The SDK already sends engine::* to default; this worker's own
        // functions and lookalikes stay in the worker's namespace.
        for fn_id in [
            "engine::functions::info",
            "sandbox-code-runner::run",
            "app::greet",
        ] {
            assert_eq!(call_namespace(fn_id), None, "{fn_id}");
        }
    }
    use serde_json::json;

    #[tokio::test]
    async fn fake_records_calls_and_returns_canned_responses() {
        let fake = FakeEngine::new();
        fake.with_response("sandbox::create", Ok(json!({ "sandbox_id": "sb-1" })));
        let out = fake
            .call("sandbox::create".into(), json!({ "image": "node" }), 1_000)
            .await;
        assert_eq!(out, Ok(json!({ "sandbox_id": "sb-1" })));
        assert_eq!(
            fake.calls(),
            vec![("sandbox::create".to_string(), json!({ "image": "node" }))]
        );
    }

    #[tokio::test]
    async fn fake_returns_error_for_unconfigured_ids() {
        let fake = FakeEngine::new();
        let out = fake.call("nope::missing".into(), json!({}), 1_000).await;
        assert_eq!(out, Err("no such function: nope::missing".to_string()));
    }

    /// A sequence answers in order and pins at its last entry — this is what
    /// expiry tests lean on (exec succeeds once, then the sandbox is gone).
    #[tokio::test]
    async fn fake_sequences_answers_and_pins_the_last() {
        let fake = FakeEngine::new();
        fake.with_response_sequence(
            "sandbox::exec",
            vec![Ok(json!({ "exit_code": 0 })), Err("gone".into())],
        );
        assert!(fake
            .call("sandbox::exec".into(), json!({}), 1)
            .await
            .is_ok());
        assert!(fake
            .call("sandbox::exec".into(), json!({}), 1)
            .await
            .is_err());
        assert!(
            fake.call("sandbox::exec".into(), json!({}), 1)
                .await
                .is_err(),
            "pinned at the last entry, not falling through"
        );
    }

    #[tokio::test]
    async fn fake_responder_computes_the_answer_from_the_payload() {
        let fake = FakeEngine::new();
        fake.with_response("sandbox::exec", Ok(json!("canned, must lose")));
        fake.with_responder("sandbox::exec", |payload| {
            Ok(json!({ "echoed_cmd": payload["cmd"] }))
        });
        let out = fake
            .call("sandbox::exec".into(), json!({ "cmd": "node" }), 1)
            .await;
        assert_eq!(out, Ok(json!({ "echoed_cmd": "node" })));
    }

    const ADAPTER_INFO: &str = "sandbox::adapter::info";
    const FUNCTION_INFO: &str = "engine::functions::info";

    fn router_reply(info: Value) -> CallResult {
        Ok(json!({
            "adapter": "sandbox-libkrun",
            "namespace": "default",
            "contract": { "major": 1, "minor": 0 },
            "info": info,
        }))
    }

    fn called_ids(fake: &FakeEngine) -> Vec<String> {
        fake.calls().into_iter().map(|(id, _)| id).collect()
    }

    #[tokio::test]
    async fn probe_ready_when_the_router_reports_an_adapter() {
        let fake = FakeEngine::new();
        fake.with_response(
            ADAPTER_INFO,
            router_reply(json!({
                "adapter": "sandbox-libkrun",
                "adapter_version": "1.0.0",
                "contract": { "major": 1, "minor": 0 },
            })),
        );
        assert_eq!(probe_sandbox(fake.as_ref()).await, SandboxProbe::Ready);
        assert_eq!(called_ids(&fake), [ADAPTER_INFO]);
    }

    #[tokio::test]
    async fn probe_incompatible_when_the_adapter_major_differs() {
        let fake = FakeEngine::new();
        fake.with_response(
            ADAPTER_INFO,
            router_reply(json!({
                "adapter": "sandbox-libkrun",
                "adapter_version": "2.0.0",
                "contract": { "major": 2, "minor": 0 },
            })),
        );
        assert_eq!(
            probe_sandbox(fake.as_ref()).await,
            SandboxProbe::Incompatible(
                r#"adapter implements sandbox contract {"major":2,"minor":0}, but this engine needs major 1"#
                    .into()
            )
        );
    }

    #[tokio::test]
    async fn probe_missing_when_the_router_has_no_adapter() {
        let fake = FakeEngine::new();
        fake.with_response(ADAPTER_INFO, router_reply(Value::Null));
        assert_eq!(probe_sandbox(fake.as_ref()).await, SandboxProbe::Missing);
        assert_eq!(called_ids(&fake), [ADAPTER_INFO]);
    }

    #[tokio::test]
    async fn probe_falls_back_on_engines_without_the_router() {
        let no_router = "remote error (function_not_found): Function sandbox::adapter::info not found in namespace default.";

        let ready = FakeEngine::new();
        ready.with_response(ADAPTER_INFO, Err(no_router.into()));
        ready.with_response(FUNCTION_INFO, Ok(json!({})));
        assert_eq!(probe_sandbox(ready.as_ref()).await, SandboxProbe::Ready);
        assert_eq!(called_ids(&ready), [ADAPTER_INFO, FUNCTION_INFO]);
        assert_eq!(
            ready.calls()[1].1,
            json!({ "function_id": "sandbox::create" })
        );

        let missing = FakeEngine::new();
        missing.with_response(ADAPTER_INFO, Err(no_router.into()));
        missing.with_response(
            FUNCTION_INFO,
            Err("remote error (NOT_FOUND): Function 'sandbox::create' is not registered.".into()),
        );
        assert_eq!(probe_sandbox(missing.as_ref()).await, SandboxProbe::Missing);
    }

    /// A router that is there but did not answer (rbac, timeout) must not be
    /// read as "no router": falling back would probe `sandbox::create`, which
    /// the router owns, and report an adapter that may not exist.
    #[tokio::test]
    async fn probe_falls_back_only_when_the_router_is_absent() {
        for raw in [
            "remote error: FORBIDDEN: rbac denies sandbox::adapter::info",
            "invocation timed out",
        ] {
            let fake = FakeEngine::new();
            fake.with_response(ADAPTER_INFO, Err(raw.into()));
            fake.with_response(FUNCTION_INFO, Ok(json!({})));
            let probe = probe_sandbox(fake.as_ref()).await;
            assert_eq!(probe, SandboxProbe::Unverified(raw.to_string()));
            assert!(!called_ids(&fake).contains(&FUNCTION_INFO.to_string()));
        }
    }

    #[tokio::test]
    async fn fake_register_exposes_the_handler_and_counts_unregisters() {
        let fake = FakeEngine::new();
        let handler: ProxyHandler = Arc::new(|p: serde_json::Value| {
            Box::pin(async move { Ok(json!({ "echo": p })) }) as BoxFuture<'static, CallResult>
        });
        let un = fake.register("ns::hello".into(), None, handler);
        assert_eq!(fake.registered_ids(), vec!["ns::hello".to_string()]);
        assert_eq!(
            fake.invoke("ns::hello", json!({ "a": 1 })).await,
            Ok(json!({ "echo": { "a": 1 } }))
        );
        un();
        assert_eq!(fake.unregister_count(), 1);
        assert!(fake.registered_ids().is_empty());
        assert!(fake.invoke("ns::hello", json!({ "a": 1 })).await.is_err());
    }
}
