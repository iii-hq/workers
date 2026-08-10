//! The router: one public runtime id space over two engines.
//!
//! Ids are minted by whichever engine owns the runtime and passed through
//! unchanged; this map records only WHICH engine owns each, so `teardown` and
//! a `runtime_id`-addressed `run` route without parsing the id. Encoding the
//! language into the id was rejected: a `runtime_id` is documented as a
//! capability-secret, and routing must not depend on reading it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures::future::BoxFuture;
use iii_node_core::engine::{CallResult, Engine, ProxyHandler, UnregisterFn};
use iii_node_core::manager::RuntimeManager;
use iii_python_core::manager::Manager as PythonManager;

use crate::config::CodeRunnerConfig;
use crate::error::{redact_unheld, CodeRunnerError};
use crate::functions::register::{RegisterRequest, RegisterResponse};
use crate::functions::run::{RunRequest, RunResponse};
use crate::functions::teardown::{TeardownRequest, TeardownResponse};
use crate::lang::Lang;

use crate::translate;
/// node-core's own caps, reused so the two languages refuse the same inputs.
use iii_node_core::ops::{MAX_DESCRIPTION_BYTES, MAX_FUNCTION_ID_BYTES};

/// A python `register_function` namespace: one pinned interpreter, plus the
/// closure that removes each of its functions from the bus.
struct PyNamespace {
    runtime_id: String,
    functions: HashMap<String, UnregisterFn>,
}

pub struct Manager {
    cfg: Arc<CodeRunnerConfig>,
    node: Arc<RuntimeManager>,
    /// The bus. Node reaches it through `RuntimeManager`, which owns its own
    /// handle; python registrations are published from HERE, because the
    /// python guest has no `iii.registerFunction` to publish them from — its
    /// `iii` is `trigger` and `namespace` only.
    engine: Arc<dyn Engine>,
    /// Canonical namespace (`"app::"`) → its interpreter and registrations.
    py_ns: Mutex<HashMap<String, PyNamespace>>,
    /// `None` when the CPython artifact could not be prepared at boot. The
    /// worker still serves node rather than refusing to start, and every
    /// python call says why — a node-only deployment losing its whole worker
    /// to a python problem would be the wrong trade.
    python: Option<Arc<PythonManager>>,
    /// Public runtime id → the engine that owns it.
    owners: Mutex<HashMap<String, Lang>>,
}

impl Manager {
    pub fn new(
        cfg: Arc<CodeRunnerConfig>,
        node: Arc<RuntimeManager>,
        engine: Arc<dyn Engine>,
        python: Option<Arc<PythonManager>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            cfg,
            node,
            engine,
            py_ns: Mutex::new(HashMap::new()),
            python,
            owners: Mutex::new(HashMap::new()),
        })
    }

    fn owner_of(&self, runtime_id: &str) -> Option<Lang> {
        self.owners.lock().unwrap().get(runtime_id).copied()
    }

    /// Which engine a request addresses, and the refusal when it does not say.
    ///
    /// Messages are sandbox-code-runner's, so a caller written against that
    /// worker reads the same words.
    fn resolve_lang(&self, req: &RunRequest) -> Result<Lang, CodeRunnerError> {
        match (&req.runtime_id, req.lang) {
            (Some(id), asked) => {
                let owner = self
                    .owner_of(id)
                    .ok_or_else(|| CodeRunnerError::RuntimeNotFound(id.clone()))?;
                if let Some(asked) = asked {
                    if asked != owner {
                        return Err(CodeRunnerError::InvalidRequest(format!(
                            "this runtime runs {owner}; omit `lang` or pass the matching one \
                             — languages cannot be mixed in one runtime"
                        )));
                    }
                }
                Ok(owner)
            }
            (None, Some(lang)) => Ok(lang),
            (None, None) => Err(CodeRunnerError::InvalidRequest(
                "`lang` is required when there is no `runtime_id`: \"node\" or \"python\"".into(),
            )),
        }
    }

    pub async fn run(&self, req: RunRequest) -> Result<RunResponse, CodeRunnerError> {
        let lang = self.resolve_lang(&req)?;
        let held_id = req.runtime_id.clone();
        match lang {
            Lang::Node => self.run_node(req, held_id).await,
            Lang::Python => self.run_python(req, held_id).await,
        }
    }

    /// A python runtime persists its working directory **and** its
    /// interpreter. `python.wasm` is a WASI command module and cannot be
    /// re-entered once `_start` returns, so the wrapper never lets it return:
    /// it parks on stdin between calls and takes one turn at a time.
    ///
    /// A superset of what sandbox-code-runner promises for its own `keep`
    /// ("same filesystem, fresh interpreter process each time"), so code
    /// written against that worker ports here but not necessarily back.
    async fn run_python(
        &self,
        req: RunRequest,
        held_id: Option<String>,
    ) -> Result<RunResponse, CodeRunnerError> {
        let Some(python) = &self.python else {
            return Err(CodeRunnerError::Engine(
                "the python engine failed to start on this worker; check its boot logs".into(),
            ));
        };
        let core_req = iii_python_core::manager::RunRequest {
            code: req.code,
            payload: None,
            timeout_ms: req.timeout_ms,
            memory_mb: None,
        };
        let started = Instant::now();

        // A `keep` that fails must NOT leave a runtime behind — the caller
        // gets no id back, so one would be unreachable and would sit there
        // until the sweep. sandbox-code-runner makes the same choice.
        let (result, id) = match (&held_id, req.keep) {
            (Some(id), _) => (python.run_in(id, core_req).await, Some(id.clone())),
            (None, true) => {
                // `None` = the config default. `memory_mb` binds for the
                // runtime's whole life because wasm linear memory only grows,
                // and this worker's wire has no per-runtime memory knob to
                // pass through — a request that carries one against an
                // existing id is refused by the core.
                let id = python.create_runtime(None).map_err(|e| {
                    translate::python_err(e, 0)
                        .err()
                        .unwrap_or_else(|| CodeRunnerError::Engine("create failed".into()))
                })?;
                let out = python.run_in(&id, core_req).await;
                match &out {
                    Ok(_) => {
                        self.owners.lock().unwrap().insert(id.clone(), Lang::Python);
                        (out, Some(id))
                    }
                    Err(_) => {
                        let _ = python.destroy_runtime(&id);
                        (out, None)
                    }
                }
            }
            (None, false) => (python.run(core_req).await, None),
        };
        let duration_ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(out) => {
                let mut resp = translate::python_ok(out, duration_ms);
                resp.runtime_id = id;
                Ok(resp)
            }
            Err(e) => {
                let mapped = translate::python_err(e, duration_ms);
                match mapped {
                    Ok(mut r) => {
                        r.runtime_id = id;
                        Ok(r)
                    }
                    Err(err) if held_id.is_none() => Err(redact_unheld(err)),
                    Err(err) => Err(err),
                }
            }
        }
    }

    async fn run_node(
        &self,
        req: RunRequest,
        held_id: Option<String>,
    ) -> Result<RunResponse, CodeRunnerError> {
        let started = Instant::now();
        let core_req = iii_node_core::wire::run::RunRequest {
            code: req.code,
            runtime_id: req.runtime_id,
            // Namespaces come only from `register_function`'s `function_id`,
            // exactly as sandbox-code-runner does — `run` has no namespace
            // field on this worker's wire.
            namespace: None,
            keep: req.keep,
            timeout_ms: req.timeout_ms,
        };
        let result = self.node.run(core_req).await;
        let duration_ms = started.elapsed().as_millis() as u64;

        // The id a kept run minted, recorded before it can be addressed.
        match result {
            Ok(out) => {
                let id = out.runtime_id.clone();
                if let Some(id) = &id {
                    self.owners.lock().unwrap().insert(id.clone(), Lang::Node);
                }
                let eval = iii_node_core::runtime::EvalOutcome {
                    result: out.result,
                    logs: out.logs,
                    registered: out.registered,
                };
                Ok(translate::node_ok(eval, id, duration_ms))
            }
            Err(e) => {
                let resp = translate::node_err(e, held_id.clone(), duration_ms);
                match resp {
                    Ok(r) => Ok(r),
                    // A caller who never supplied an id must not learn one.
                    Err(err) if held_id.is_none() => Err(redact_unheld(err)),
                    Err(err) => Err(err),
                }
            }
        }
    }

    pub async fn register(
        &self,
        req: RegisterRequest,
    ) -> Result<RegisterResponse, CodeRunnerError> {
        match req.lang {
            Lang::Python => return self.register_python(req).await,
            Lang::Node => {}
        }
        let core_req = iii_node_core::wire::register::RegisterRequest {
            function_id: req.function_id.clone(),
            source: req.source,
            description: req.description,
        };
        match self.node.register(core_req).await {
            // node-core reports the namespace it claimed; this wire reports
            // only that the registration landed, matching
            // sandbox-code-runner's two-field response.
            Ok(out) => Ok(RegisterResponse {
                function_id: out.function_id,
                registered: true,
            }),
            // A register caller never holds the namespace runtime's id.
            Err(e) => Err(redact_unheld(
                translate::node_err(e, None, 0)
                    .err()
                    .unwrap_or_else(|| CodeRunnerError::Engine("registration failed".into())),
            )),
        }
    }

    /// Publish a python `handler(payload)` on the bus.
    ///
    /// Structurally different from the node path, and deliberately so. Node's
    /// guest publishes itself: the evaluated source calls
    /// `iii.registerFunction`, which reaches the bus through an op. Python's
    /// guest cannot — `python.wasm` imports one module and exports `_start`,
    /// so its `iii` is `trigger` and `namespace` only. The host publishes on
    /// its behalf and dispatches each invocation back in as one turn on the
    /// namespace's pinned interpreter.
    async fn register_python(
        &self,
        req: RegisterRequest,
    ) -> Result<RegisterResponse, CodeRunnerError> {
        let python = self
            .python
            .as_ref()
            .ok_or_else(|| {
                CodeRunnerError::Engine(
                    "the python engine failed to start on this worker; check its boot logs".into(),
                )
            })?
            .clone();
        let invalid = CodeRunnerError::InvalidRequest;

        if req.function_id.len() > MAX_FUNCTION_ID_BYTES {
            return Err(invalid(format!(
                "function id {:?}… is {} bytes; the limit is {MAX_FUNCTION_ID_BYTES}",
                req.function_id.chars().take(40).collect::<String>(),
                req.function_id.len()
            )));
        }
        if let Some(d) = &req.description {
            if d.len() > MAX_DESCRIPTION_BYTES {
                return Err(invalid(format!(
                    "description is {} bytes; the limit is {MAX_DESCRIPTION_BYTES}",
                    d.len()
                )));
            }
        }

        // The namespace IS the first segment; an id with no `::` cannot name
        // one and is refused rather than silently placed in a default.
        let ns = match req.function_id.split_once("::") {
            Some((head, _)) if !head.is_empty() => format!("{head}::"),
            _ => {
                return Err(invalid(format!(
                    "function id {:?} has no namespace; pass something like \"my-app::greet\"",
                    req.function_id
                )))
            }
        };
        // node-core's validator, shared rather than copied: the two languages
        // have to agree byte-for-byte on what a legal namespace is.
        let namespace = iii_node_core::manager::normalize_namespace(
            &ns,
            std::slice::from_ref(&req.function_id),
        )
        .map_err(invalid)?;

        // Claim BEFORE creating anything, in the SAME registry node claims
        // into — including the guest-driven op. Two registries would each
        // believe they owned this id, both would reach the SDK's
        // `register_function`, and the second would hit its duplicate-id
        // panic in a non-unwinding V8 callback, which aborts the process.
        //
        // The OWNER must not be the bare namespace: that is exactly the
        // placeholder node's `register` claims under before it has a runtime
        // id, so a python id claimed that way looks to node like its own
        // earlier claim and is silently reclaimed — after which node evals the
        // source and `op_iii_register` takes the process down. Observed, as an
        // abort, by the e2e suite. `python:` cannot collide: a node owner is
        // either a minted runtime id or `<stem>::`, and a stem is
        // `[a-z0-9._-]` only, so it can never contain a colon.
        let owner = format!("python:{namespace}");
        let one_id = std::slice::from_ref(&req.function_id);
        self.node.ids().claim_all(one_id, &owner).map_err(|_| {
            invalid(format!(
                "function id {:?} is already taken",
                req.function_id
            ))
        })?;

        let runtime_id = match self.py_namespace_runtime(&python, &namespace) {
            Ok(id) => id,
            Err(e) => {
                self.node.ids().release_ids(one_id, &owner);
                return Err(e);
            }
        };

        if let Err(e) = self
            .py_define_handler(&python, &runtime_id, &req.source)
            .await
        {
            // Nothing reached the bus, so the id is free to release. Unlike
            // node's path there is no guest-driven registration that could
            // have landed one behind our back: this function is the only way
            // a python id reaches the bus at all.
            self.node.ids().release_ids(one_id, &owner);
            // And if this call is what created the namespace, take its
            // interpreter with it. A pinned runtime is exempt from the idle
            // sweep, so an orphan left here is one nothing will ever reclaim,
            // and it counts against `max_runtimes` forever. Only when the
            // namespace has no OTHER registrations: an existing namespace owns
            // its interpreter and a failed addition must not kill it.
            let mut map = self.py_ns.lock().unwrap();
            if map.get(&namespace).is_some_and(|e| e.functions.is_empty()) {
                map.remove(&namespace);
                drop(map);
                let _ = python.destroy_runtime(&runtime_id);
            }
            return Err(e);
        }

        // A redeploy replaces. Retire the previous registration FIRST:
        // `unregister` is keyed by function id, so calling the old closure
        // after publishing the new one removes the NEW registration and leaves
        // the id advertised by nothing. Dropping the closure instead of calling
        // it is the other way to get this wrong — `UnregisterFn` is an explicit
        // call, not a `Drop` — and leaves two handlers on one id.
        let previous = self
            .py_ns
            .lock()
            .unwrap()
            .get_mut(&namespace)
            .and_then(|e| e.functions.remove(&req.function_id));
        if let Some(old) = previous {
            old();
        }

        let handler = self.py_handler(&python, &runtime_id);
        let unregister =
            self.engine
                .register(req.function_id.clone(), req.description.clone(), handler);
        self.py_ns
            .lock()
            .unwrap()
            .get_mut(&namespace)
            .expect("py_namespace_runtime inserted it")
            .functions
            .insert(req.function_id.clone(), unregister);

        Ok(RegisterResponse {
            function_id: req.function_id,
            registered: true,
        })
    }

    /// The namespace's interpreter, created and pinned on first use.
    fn py_namespace_runtime(
        &self,
        python: &Arc<PythonManager>,
        namespace: &str,
    ) -> Result<String, CodeRunnerError> {
        // Held across the create so two first-registrations on one namespace
        // cannot each mint an interpreter and leave one orphaned.
        let mut map = self.py_ns.lock().unwrap();
        if let Some(ns) = map.get(namespace) {
            return Ok(ns.runtime_id.clone());
        }
        let to_err = |e| {
            translate::python_err(e, 0)
                .err()
                .unwrap_or_else(|| CodeRunnerError::Engine("creating the runtime failed".into()))
        };
        let runtime_id = python.create_runtime(None).map_err(to_err)?;
        // A registration's lifetime is the registration's, not its traffic's:
        // without this an unpopular function is reaped by the idle sweep and
        // its next caller gets a runtime error for a function the catalog
        // still advertises.
        if let Err(e) = python.pin_runtime(&runtime_id) {
            let _ = python.destroy_runtime(&runtime_id);
            return Err(to_err(e));
        }
        map.insert(
            namespace.to_string(),
            PyNamespace {
                runtime_id: runtime_id.clone(),
                functions: HashMap::new(),
            },
        );
        Ok(runtime_id)
    }

    /// Run the caller's source in the namespace interpreter and prove it left
    /// a callable `handler` behind.
    async fn py_define_handler(
        &self,
        python: &Arc<PythonManager>,
        runtime_id: &str,
        source: &str,
    ) -> Result<(), CodeRunnerError> {
        // The check runs in the same turn as the source, in the same
        // namespace, so it sees exactly what the source defined. A separate
        // turn would also work — the interpreter persists — but would report a
        // missing handler as a second, unrelated-looking failure.
        let code = format!("{source}\n\n{HANDLER_CHECK}");
        let req = iii_python_core::manager::RunRequest {
            code,
            payload: None,
            timeout_ms: Some(self.cfg.default_timeout_ms),
            memory_mb: None,
        };
        python
            .run_in(runtime_id, req)
            .await
            .map(|_| ())
            .map_err(|e| {
                use iii_python_core::error::ErrorKind;
                match e.kind {
                    // On `run` these three are a RESPONSE — a failing script is
                    // not an infrastructure failure. On `register_function` there
                    // is no response to carry them: the caller asked to publish a
                    // handler and no handler exists, so the registration itself
                    // failed. Same three kinds, opposite verdict, because the
                    // request means something different.
                    ErrorKind::SyntaxError
                    | ErrorKind::PythonException
                    | ErrorKind::ResultTooLarge => {
                        let mut m = e.message;
                        if let Some(tb) = e.traceback {
                            m.push('\n');
                            m.push_str(&tb);
                        }
                        CodeRunnerError::InvalidRequest(m)
                    }
                    _ => translate::python_err(e, 0).err().unwrap_or_else(|| {
                        CodeRunnerError::Engine("defining the handler failed".into())
                    }),
                }
            })
    }

    /// What the bus calls. One turn on the namespace interpreter per
    /// invocation, so a handler sees every global its registration left
    /// behind — and every global a previous invocation left behind.
    fn py_handler(&self, python: &Arc<PythonManager>, runtime_id: &str) -> ProxyHandler {
        let python = python.clone();
        let runtime_id = runtime_id.to_string();
        let timeout_ms = self.cfg.default_timeout_ms;
        Arc::new(move |payload: serde_json::Value| {
            let python = python.clone();
            let runtime_id = runtime_id.clone();
            Box::pin(async move {
                let req = iii_python_core::manager::RunRequest {
                    code: "result = handler(payload)".into(),
                    payload: Some(payload),
                    timeout_ms: Some(timeout_ms),
                    memory_mb: None,
                };
                match python.run_in(&runtime_id, req).await {
                    Ok(out) => Ok(out.result),
                    // The bus wants a message, not a taxonomy. `stderr` is
                    // already folded into it by the core for a tenant
                    // exception, which is the case a caller actually debugs.
                    Err(e) => Err(e.message),
                }
            }) as BoxFuture<'static, CallResult>
        })
    }

    /// Tear down a python namespace: unregister its functions, release their
    /// ids, and destroy the interpreter. Returns the ids removed.
    fn teardown_py_namespace(&self, namespace: &str) -> Option<Vec<String>> {
        let entry = self.py_ns.lock().unwrap().remove(namespace)?;
        let mut removed: Vec<String> = Vec::new();
        for (id, unregister) in entry.functions {
            unregister();
            removed.push(id);
        }
        // Released under the same owner that claimed them (see
        // `register_python`), so a later registration can take the same ids.
        self.node
            .ids()
            .release_ids(&removed, &format!("python:{namespace}"));
        removed.sort();
        if let Some(python) = &self.python {
            let _ = python.destroy_runtime(&entry.runtime_id);
        }
        Some(removed)
    }

    pub async fn teardown(
        &self,
        req: TeardownRequest,
    ) -> Result<TeardownResponse, CodeRunnerError> {
        match (&req.runtime_id, &req.namespace) {
            (Some(id), None) => {
                let owner = self
                    .owner_of(id)
                    .ok_or_else(|| CodeRunnerError::RuntimeNotFound(id.clone()))?;
                match owner {
                    Lang::Node => {
                        let out = self
                            .node
                            .teardown_runtime(id)
                            .await
                            .map_err(|e| self.core_err(e))?;
                        self.owners.lock().unwrap().remove(id);
                        Ok(TeardownResponse {
                            runtime_id: Some(id.clone()),
                            namespace: None,
                            torn_down: true,
                            unregistered: out.unregistered,
                        })
                    }
                    Lang::Python => {
                        let python = self.python.as_ref().ok_or_else(|| {
                            CodeRunnerError::Engine("the python engine is not running".into())
                        })?;
                        python.destroy_runtime(id).map_err(|e| {
                            translate::python_err(e, 0).err().unwrap_or_else(|| {
                                CodeRunnerError::Engine("teardown failed".into())
                            })
                        })?;
                        self.owners.lock().unwrap().remove(id);
                        Ok(TeardownResponse {
                            runtime_id: Some(id.clone()),
                            namespace: None,
                            torn_down: true,
                            // A python runtime registers nothing: handlers
                            // need an interpreter that outlives the call.
                            unregistered: Vec::new(),
                        })
                    }
                }
            }
            (None, Some(ns)) => {
                let canonical = normalize_namespace(ns);
                // Both halves are tried. A namespace is not owned by a
                // language — ids are claimed one at a time — so `app::a` in
                // node and `app::b` in python can coexist, and tearing down
                // "app" has to mean both. Python first because it cannot fail:
                // it is a map removal, so a node failure afterwards cannot
                // leave python half-torn-down with nothing recorded.
                let py = self.teardown_py_namespace(&canonical);

                let core_req = iii_node_core::wire::teardown::TeardownRequest {
                    runtime_id: None,
                    namespace: Some(ns.clone()),
                };
                let node = match self.node.teardown(core_req).await {
                    Ok(out) => Some(out.unregistered),
                    // Not found in node is only fatal when python had nothing
                    // either — otherwise this namespace WAS torn down, just
                    // not on the side that answered.
                    Err(iii_node_core::error::NodeEngineError::RuntimeNotFound(n)) => {
                        if py.is_none() {
                            return Err(CodeRunnerError::NamespaceNotFound(n));
                        }
                        None
                    }
                    Err(other) => return Err(self.core_err(other)),
                };

                // Any id these runtimes held is now dead; ownership entries
                // naming them are pruned on the next sweep. The namespace path
                // never returns a runtime id, so nothing more precise is
                // available here.
                let mut unregistered = node.unwrap_or_default();
                unregistered.extend(py.unwrap_or_default());
                unregistered.sort();
                unregistered.dedup();
                Ok(TeardownResponse {
                    runtime_id: None,
                    // Canonical form, even when the caller passed the bare one.
                    namespace: Some(canonical),
                    torn_down: true,
                    unregistered,
                })
            }
            (Some(_), Some(_)) => Err(CodeRunnerError::InvalidRequest(
                "pass exactly one of runtime_id or namespace, not both: runtime_id tears down a \
                 single kept run's runtime, namespace tears down every runtime backing a \
                 register_function namespace"
                    .into(),
            )),
            (None, None) => Err(CodeRunnerError::InvalidRequest(
                "pass exactly one of runtime_id (a kept run's runtime) or namespace (a \
                 register_function namespace, e.g. \"app\" for ids like app::greet)"
                    .into(),
            )),
        }
    }

    fn core_err(&self, e: iii_node_core::error::NodeEngineError) -> CodeRunnerError {
        translate::node_err(e, None, 0)
            .err()
            .unwrap_or_else(|| CodeRunnerError::Engine("unexpected engine outcome".into()))
    }

    /// Reap runtimes idle past `idle_ttl_secs`, forgetting their ownership
    /// entries so a stale id cannot route anywhere.
    pub fn sweep_idle(&self) -> Vec<String> {
        let mut reaped = self.node.sweep_idle();
        if let Some(python) = &self.python {
            reaped.extend(python.sweep_idle());
        }
        if !reaped.is_empty() {
            let mut owners = self.owners.lock().unwrap();
            for id in &reaped {
                owners.remove(id);
            }
        }
        reaped
    }

    pub fn idle_ttl_secs(&self) -> u64 {
        self.cfg.idle_ttl_secs
    }
}

/// `app` and `app::` both mean `app::`, and that canonical form is what a
/// teardown response echoes.
/// Proves the caller's source left a callable `handler` behind, in words that
/// say what to write rather than what went wrong. A bare `NameError: handler`
/// from a failed registration reads like a bug in this worker.
const HANDLER_CHECK: &str = r#"if not callable(globals().get("handler")):
    raise TypeError(
        "register_function: your source must define handler(payload) — a callable "
        "taking one argument. Found: " + repr(globals().get("handler"))
    )"#;

fn normalize_namespace(ns: &str) -> String {
    let trimmed = ns.trim_end_matches(':');
    format!("{trimmed}::")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_namespace_echoes_back_canonical() {
        assert_eq!(normalize_namespace("app"), "app::");
        assert_eq!(normalize_namespace("app::"), "app::");
        assert_eq!(normalize_namespace("app:"), "app::");
    }
}

#[cfg(test)]
mod router_tests {
    use super::*;

    fn manager() -> Arc<Manager> {
        manager_and_bus().0
    }

    fn manager_and_bus() -> (Arc<Manager>, Arc<TestBus>) {
        manager_with(CodeRunnerConfig {
            // Small enough to reach the cap in a test without spawning 32
            // isolates.
            max_runtimes: 4,
            ..CodeRunnerConfig::default()
        })
    }

    fn manager_with(cfg: CodeRunnerConfig) -> (Arc<Manager>, Arc<TestBus>) {
        iii_node_core::runtime::init_v8_platform();
        let cfg = Arc::new(cfg);
        let bus = Arc::new(TestBus::default());
        let node = RuntimeManager::new(
            Arc::new(cfg.node()),
            bus.clone(),
            // The SAME seed as production (`main.rs`): seeding only
            // `STATIC_IDS` here would leave the console UI's content function
            // claimable in tests and make the regression test below pass for
            // the wrong reason.
            &crate::functions::seeded_ids(),
        );
        let python = iii_python_core::runner::Runner::boot()
            .ok()
            .map(|r| iii_python_core::manager::Manager::new(Arc::new(cfg.python()), r));
        (Manager::new(cfg, node, bus.clone(), python), bus)
    }

    /// A bus-free `Engine` that REMEMBERS what was published. This crate's
    /// tests never need a live engine, and the core's own `FakeEngine` is
    /// `cfg(test)`-gated to that crate — but a registration whose handler is
    /// dropped on the floor cannot be told from one that never happened, so
    /// this one keeps them callable.
    #[derive(Default)]
    struct TestBus {
        published: Arc<Mutex<HashMap<String, ProxyHandler>>>,
        /// Every id whose `UnregisterFn` was actually CALLED. A closure that
        /// is dropped instead of called leaves a registration live on a real
        /// bus, and nothing else here would notice.
        retired: Arc<Mutex<Vec<String>>>,
    }

    impl TestBus {
        fn ids(&self) -> Vec<String> {
            let mut v: Vec<String> = self.published.lock().unwrap().keys().cloned().collect();
            v.sort();
            v
        }
        fn retired(&self) -> Vec<String> {
            self.retired.lock().unwrap().clone()
        }
        async fn invoke(&self, fn_id: &str, payload: serde_json::Value) -> CallResult {
            let h = self
                .published
                .lock()
                .unwrap()
                .get(fn_id)
                .cloned()
                .unwrap_or_else(|| panic!("{fn_id} was never published"));
            h(payload).await
        }
    }

    impl iii_node_core::engine::Engine for TestBus {
        fn call(
            &self,
            _: String,
            _: serde_json::Value,
            _: u64,
            _: Option<String>,
        ) -> futures::future::BoxFuture<'static, iii_node_core::engine::CallResult> {
            Box::pin(async { Err("no bus in tests".to_string()) })
        }
        fn register(
            &self,
            fn_id: String,
            _: Option<String>,
            handler: iii_node_core::engine::ProxyHandler,
        ) -> iii_node_core::engine::UnregisterFn {
            self.published
                .lock()
                .unwrap()
                .insert(fn_id.clone(), handler);
            // Keyed by id, exactly like the SDK's `function_ref`: calling a
            // stale one after the id was re-registered removes whatever holds
            // the id NOW.
            let published = self.published.clone();
            let retired = self.retired.clone();
            Box::new(move || {
                published.lock().unwrap().remove(&fn_id);
                retired.lock().unwrap().push(fn_id.clone());
            })
        }
        fn register_trigger(
            &self,
            _: serde_json::Value,
        ) -> futures::future::BoxFuture<'static, Result<String, String>> {
            Box::pin(async { Ok("trig-test".to_string()) })
        }
        fn unregister_trigger(
            &self,
            _: String,
        ) -> futures::future::BoxFuture<'static, Result<(), String>> {
            Box::pin(async { Ok(()) })
        }
        fn register_trigger_type(
            &self,
            _: String,
            _: Option<String>,
            _: iii_node_core::engine::ProxyHandler,
        ) -> iii_node_core::engine::UnregisterFn {
            Box::new(|| {})
        }
    }

    fn run(code: &str) -> RunRequest {
        RunRequest {
            code: code.into(),
            runtime_id: None,
            lang: Some(Lang::Node),
            keep: false,
            timeout_ms: Some(5_000),
        }
    }

    #[tokio::test]
    async fn a_one_shot_run_returns_a_process_shaped_response_and_no_id() {
        let m = manager();
        let r = m.run(run("console.log('hi'); return 2 + 2")).await.unwrap();
        assert_eq!(r.stdout, "hi\n");
        assert_eq!(r.exit_code, 0);
        assert!(r.success);
        assert_eq!(r.result, serde_json::json!(4));
        assert!(
            r.runtime_id.is_none(),
            "a one-shot run has nothing to address"
        );
    }

    /// The contract this worker exists to reproduce. Mutation: make
    /// `translate::node_err` return `Err` for `EvalFailed` — the call then
    /// fails where sandbox-code-runner's contract says it succeeds.
    #[tokio::test]
    async fn a_throwing_script_is_a_response_not_an_error() {
        let m = manager();
        let r = m
            .run(run("console.log('before'); throw new Error('boom')"))
            .await
            .expect("a tenant throw must not be a bus error");
        assert_eq!(r.stdout, "before\n");
        assert!(r.stderr.contains("boom"), "stderr was {:?}", r.stderr);
        assert_eq!(r.exit_code, 1);
        assert!(!r.success);
    }

    /// `keep` mints an id, that id addresses the same runtime, and teardown
    /// takes it away. Mutation: drop the `owners.insert` and the second call
    /// reports `runtime_not_found`.
    #[tokio::test]
    async fn a_kept_runtime_is_addressable_until_it_is_torn_down() {
        let m = manager();
        let first = m
            .run(RunRequest {
                keep: true,
                ..run("globalThis.n = 41; return 1")
            })
            .await
            .unwrap();
        let id = first.runtime_id.clone().expect("keep must mint an id");

        let second = m
            .run(RunRequest {
                runtime_id: Some(id.clone()),
                lang: None,
                ..run("return globalThis.n + 1")
            })
            .await
            .unwrap();
        assert_eq!(second.result, serde_json::json!(42), "globals must persist");
        assert_eq!(second.runtime_id.as_deref(), Some(id.as_str()));

        let torn = m
            .teardown(TeardownRequest {
                runtime_id: Some(id.clone()),
                namespace: None,
            })
            .await
            .unwrap();
        assert!(torn.torn_down);
        assert_eq!(torn.runtime_id.as_deref(), Some(id.as_str()));

        let after = m
            .run(RunRequest {
                runtime_id: Some(id),
                lang: None,
                ..run("return 1")
            })
            .await
            .expect_err("a torn-down id must not resolve");
        assert_eq!(after.code(), "code-runner::runtime_not_found");
    }

    /// Both refusals use sandbox-code-runner's own wording, so a caller
    /// written against that worker reads the same words.
    #[tokio::test]
    async fn lang_is_required_without_a_runtime_id_and_may_not_contradict_one() {
        let m = manager();
        let err = m
            .run(RunRequest {
                lang: None,
                ..run("return 1")
            })
            .await
            .expect_err("lang must be required");
        assert_eq!(err.code(), "code-runner::invalid_request");
        assert!(err.to_string().contains("`lang` is required"));

        let kept = m
            .run(RunRequest {
                keep: true,
                ..run("return 1")
            })
            .await
            .unwrap();
        let mismatch = m
            .run(RunRequest {
                runtime_id: kept.runtime_id.clone(),
                lang: Some(Lang::Python),
                ..run("return 1")
            })
            .await
            .expect_err("a contradicting lang must be refused");
        assert!(
            mismatch.to_string().contains("cannot be mixed"),
            "got {mismatch}"
        );
    }

    fn py(code: &str) -> RunRequest {
        RunRequest {
            code: code.into(),
            runtime_id: None,
            lang: Some(Lang::Python),
            keep: false,
            timeout_ms: Some(10_000),
        }
    }

    /// The point of wiring python at all: one API, two languages, the same
    /// process-shaped response.
    #[tokio::test]
    async fn a_python_run_returns_the_same_shape_a_node_run_does() {
        let m = manager();
        let r = m
            .run(py("print('hi')\nresult = 2 + 2"))
            .await
            .expect("python must run");
        assert_eq!(r.stdout, "hi\n");
        assert_eq!(r.exit_code, 0);
        assert!(r.success);
        assert_eq!(r.result, serde_json::json!(4));
        assert!(r.runtime_id.is_none());
    }

    /// Identical rule to the node side. Mutation: move `PythonException`
    /// into `python_err`'s error arm and the two languages stop agreeing.
    #[tokio::test]
    async fn a_throwing_python_script_is_also_a_response() {
        let m = manager();
        let r = m
            .run(py("raise ValueError('boom')"))
            .await
            .expect("a tenant exception must not be a bus error");
        assert_eq!(r.exit_code, 1);
        assert!(!r.success);
        assert!(r.stderr.contains("boom"), "stderr was {:?}", r.stderr);
    }

    /// The python half of `keep`: the files AND the interpreter persist.
    ///
    /// Both halves asserted, because they used to differ — python's `keep`
    /// persisted only files until park-and-loop landed, and node's persists
    /// only files still. A caller reading `keep` on this worker's wire has to
    /// be able to trust that "the runtime persists" means the same thing here
    /// as `a_kept_node_runtime_*` says it means there.
    #[tokio::test]
    async fn a_kept_python_runtime_persists_files_and_globals() {
        let m = manager();
        let first = m
            .run(RunRequest {
                keep: true,
                ..py("leaked = 1\nopen('/work/f', 'w').write('kept')\nresult = None")
            })
            .await
            .expect("python keep must work");
        let id = first.runtime_id.clone().expect("keep must mint an id");

        let second = m
            .run(RunRequest {
                runtime_id: Some(id.clone()),
                lang: None,
                ..py("result = ['leaked' in globals(), open('/work/f').read()]")
            })
            .await
            .unwrap();
        assert_eq!(second.result, serde_json::json!([true, "kept"]));

        let torn = m
            .teardown(TeardownRequest {
                runtime_id: Some(id.clone()),
                namespace: None,
            })
            .await
            .unwrap();
        assert!(torn.torn_down);

        let after = m
            .run(RunRequest {
                runtime_id: Some(id),
                lang: None,
                ..py("result = 1")
            })
            .await
            .expect_err("a torn-down python id must not resolve");
        assert_eq!(after.code(), "code-runner::runtime_not_found");
    }

    /// A `keep` that fails must leave nothing behind: the caller gets no id,
    /// so any runtime created for it would be unreachable until the sweep.
    /// Mutation: drop the `destroy_runtime` in the failure arm.
    #[tokio::test]
    async fn a_failed_python_keep_does_not_strand_a_runtime() {
        let m = manager();
        let before = m.python.as_ref().unwrap().live_runtime_count();
        let r = m
            .run(RunRequest {
                keep: true,
                timeout_ms: Some(200),
                ..py("while True:\n    pass")
            })
            .await;
        assert!(r.is_err(), "a runaway must fail");
        assert_eq!(
            m.python.as_ref().unwrap().live_runtime_count(),
            before,
            "a failed keep must not strand a runtime"
        );
    }

    /// A one-line `def` body is still a definition. This spelling used to be
    /// the fixture for "python handlers are refused"; it is now the smallest
    /// registration that works, and it stays as a regression test that the
    /// wrapper does not require a multi-line body.
    #[tokio::test]
    async fn a_single_line_def_registers() {
        let (m, bus) = manager_and_bus();
        m.register(RegisterRequest {
            function_id: "app::greet".into(),
            source: "def handler(payload): return 'hi'".into(),
            description: None,
            lang: Lang::Python,
        })
        .await
        .expect("a single-line def defines a handler");
        assert_eq!(
            bus.invoke("app::greet", serde_json::json!(null))
                .await
                .unwrap(),
            serde_json::json!("hi")
        );
    }

    #[tokio::test]
    async fn teardown_refuses_both_and_neither() {
        let m = manager();
        for (rt, ns) in [
            (Some("rt-1".to_string()), Some("app".to_string())),
            (None, None),
        ] {
            let err = m
                .teardown(TeardownRequest {
                    runtime_id: rt,
                    namespace: ns,
                })
                .await
                .expect_err("exactly one must be required");
            assert_eq!(err.code(), "code-runner::invalid_request");
        }
    }

    /// A caller who ran one-shot never held an id, so a mid-run runtime death
    /// must not disclose one. Mutation: drop the `redact_unheld` call in
    /// `run_node`'s error arm.
    #[tokio::test]
    async fn a_one_shot_caller_never_learns_a_runtime_id() {
        let m = manager();
        let err = m
            .run(RunRequest {
                timeout_ms: Some(50),
                ..run("while (true) {}")
            })
            .await
            .expect_err("a runaway must fail");
        assert!(
            !err.to_string().contains("rt-"),
            "a one-shot caller must not learn an id: {err}"
        );
    }
    fn reg(function_id: &str, source: &str) -> RegisterRequest {
        RegisterRequest {
            function_id: function_id.into(),
            source: source.into(),
            description: Some("a test handler".into()),
            lang: Lang::Python,
        }
    }

    /// The end-to-end shape: source defines `handler`, the id reaches the bus,
    /// and invoking it there runs the handler.
    #[tokio::test]
    async fn a_python_registration_publishes_a_callable_handler() {
        let (m, bus) = manager_and_bus();
        let out = m
            .register(reg(
                "pyapp::double",
                "def handler(payload):\n    return payload['n'] * 2",
            ))
            .await
            .expect("python register_function must work");
        assert_eq!(out.function_id, "pyapp::double");
        assert!(out.registered);
        assert_eq!(bus.ids(), vec!["pyapp::double".to_string()]);

        let answer = bus
            .invoke("pyapp::double", serde_json::json!({"n": 21}))
            .await
            .expect("the handler must answer");
        assert_eq!(answer, serde_json::json!(42));
    }

    /// The interpreter behind a registration persists, so a handler keeps
    /// whatever its registration set up — the reason python `register_function`
    /// could not exist before park-and-loop landed.
    #[tokio::test]
    async fn a_handler_keeps_the_state_its_registration_built() {
        let (m, bus) = manager_and_bus();
        m.register(reg(
            "pyapp::counter",
            "import itertools\n_seq = itertools.count(1)\n\
             def handler(payload):\n    return next(_seq)",
        ))
        .await
        .unwrap();

        for expected in 1..=3 {
            let got = bus
                .invoke("pyapp::counter", serde_json::json!(null))
                .await
                .unwrap();
            assert_eq!(got, serde_json::json!(expected), "state did not persist");
        }
    }

    /// Source that never defines `handler` must fail the REGISTRATION, not
    /// publish an id that fails on first call.
    ///
    /// Mutation: drop `HANDLER_CHECK` from `py_define_handler`. The register
    /// then succeeds, the catalog advertises `pyapp::nope`, and the caller
    /// finds out at invocation time instead.
    #[tokio::test]
    async fn source_without_a_handler_is_refused_at_registration() {
        let (m, bus) = manager_and_bus();
        let err = m
            .register(reg("pyapp::nope", "x = 1  # no handler here"))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("handler(payload)"),
            "message must say what to write: {err}"
        );
        assert!(
            bus.ids().is_empty(),
            "a failed registration reached the bus"
        );

        // And the id is free, so a corrected redeploy works.
        m.register(reg(
            "pyapp::nope",
            "def handler(payload):\n    return 'fixed'",
        ))
        .await
        .expect("the id must have been released");
    }

    /// A registration that fails to define its handler leaves nothing behind.
    ///
    /// Mutation: drop the namespace cleanup on the define-failure path. The
    /// interpreter it booted is PINNED, so the idle sweep will never reclaim
    /// it — a typo in a redeploy would permanently consume a `max_runtimes`
    /// slot, and `teardown` would report success for a namespace that holds
    /// no functions.
    #[tokio::test]
    async fn a_failed_registration_leaves_no_runtime_behind() {
        let (m, _bus) = manager_and_bus();
        let before = m.python.as_ref().unwrap().live_runtime_count();

        m.register(reg("pyorphan::x", "x = 1  # no handler"))
            .await
            .unwrap_err();

        assert_eq!(
            m.python.as_ref().unwrap().live_runtime_count(),
            before,
            "a failed registration left a pinned interpreter behind"
        );
        // And the namespace does not exist, so teardown says so.
        assert!(m
            .teardown(TeardownRequest {
                runtime_id: None,
                namespace: Some("pyorphan".into()),
            })
            .await
            .is_err());
    }

    /// A non-callable `handler` is the same failure as a missing one — binding
    /// the NAME is not the contract, being callable is.
    #[tokio::test]
    async fn a_non_callable_handler_is_refused() {
        let (m, _bus) = manager_and_bus();
        let err = m
            .register(reg("pyapp::notfn", "handler = 42"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("handler(payload)"), "{err}");
    }

    /// A python id is claimed in the SAME registry node claims into.
    ///
    /// Two registries would each believe they owned the id, both would reach
    /// the SDK's `register_function`, and the second would hit its duplicate-id
    /// panic — which aborts the process, so there is no recovering from it at
    /// runtime. Asserted against the registry directly rather than by racing a
    /// node registration through its guest-eval path, because the claim IS the
    /// mechanism and a colliding registration is only one thing it stops.
    ///
    /// Mutation: give the python path its own `IdRegistry`, or claim under the
    /// bare `namespace`. Either way this claim succeeds — and the second is
    /// worse than it looks: the bare namespace is the placeholder node claims
    /// under before it has a runtime id, so node reads a python id claimed
    /// that way as its own earlier claim, reclaims it, evals the source, and
    /// `op_iii_register` aborts the process from a non-unwinding V8 callback.
    #[tokio::test]
    async fn a_python_id_is_claimed_in_the_registry_node_shares() {
        let (m, _bus) = manager_and_bus();
        m.register(reg(
            "shared::fn",
            "def handler(payload):\n    return payload",
        ))
        .await
        .unwrap();

        assert!(
            m.node
                .ids()
                .claim_all(&["shared::fn".to_string()], "some-other-owner")
                .is_err(),
            "node's registry does not know about the python registration"
        );
        // And specifically NOT reclaimable under the bare namespace, which is
        // the owner node's own `register` uses before it has a runtime id.
        assert!(
            m.node
                .ids()
                .claim_all(&["shared::fn".to_string()], "shared::")
                .is_err(),
            "node would reclaim this id as its own and then abort on register"
        );

        // And teardown releases it in that same registry.
        m.teardown(TeardownRequest {
            runtime_id: None,
            namespace: Some("shared".into()),
        })
        .await
        .unwrap();
        assert!(m
            .node
            .ids()
            .claim_all(&["shared::fn".to_string()], "some-other-owner")
            .is_ok());
    }

    /// A registered function's runtime is exempt from the idle sweep: its
    /// lifetime is the registration's, not its traffic's.
    ///
    /// Mutation: drop the `pin_runtime` call in `py_namespace_runtime`. The
    /// sweep then reaps the interpreter behind a merely unpopular function,
    /// and its next caller gets a runtime error for something the catalog
    /// still advertises.
    #[tokio::test]
    async fn a_registered_functions_runtime_survives_the_idle_sweep() {
        // Everything idle is stale at a zero TTL, which is what makes the
        // exemption the only thing keeping this runtime alive.
        let (m, bus) = manager_with(CodeRunnerConfig {
            idle_ttl_secs: 0,
            ..CodeRunnerConfig::default()
        });
        m.register(reg(
            "pyapp::rare",
            "def handler(payload):\n    return 'still here'",
        ))
        .await
        .unwrap();

        m.sweep_idle();

        assert_eq!(
            bus.invoke("pyapp::rare", serde_json::json!(null))
                .await
                .unwrap(),
            serde_json::json!("still here"),
            "the sweep reaped a registered function's interpreter"
        );
    }

    /// A redeploy replaces rather than doubling up. `UnregisterFn` is an
    /// explicit call, not a `Drop`, so dropping the old closure would leave the
    /// first handler live alongside the second.
    #[tokio::test]
    async fn re_registering_an_id_replaces_its_handler() {
        let (m, bus) = manager_and_bus();
        m.register(reg("pyapp::v", "def handler(payload):\n    return 'v1'"))
            .await
            .unwrap();
        m.register(reg("pyapp::v", "def handler(payload):\n    return 'v2'"))
            .await
            .expect("a redeploy of the same id must be allowed");

        // Both halves matter. `retired` catches never retiring the old
        // registration, which on a real bus leaves two handlers on one id;
        // `ids` catches retiring it too late, which removes the id entirely.
        assert_eq!(bus.retired(), vec!["pyapp::v".to_string()]);
        assert_eq!(bus.ids(), vec!["pyapp::v".to_string()]);
        assert_eq!(
            bus.invoke("pyapp::v", serde_json::json!(null))
                .await
                .unwrap(),
            serde_json::json!("v2")
        );
    }

    /// An id with no `::` names no namespace and is refused rather than
    /// silently placed in a default — the same rule node applies.
    #[tokio::test]
    async fn a_python_id_without_a_namespace_is_refused() {
        let (m, _bus) = manager_and_bus();
        let err = m
            .register(reg("bare", "def handler(payload):\n    return 1"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("namespace"), "{err}");
    }

    /// `teardown namespace=` removes python registrations too, and frees their
    /// ids for a later registration.
    #[tokio::test]
    async fn tearing_down_a_namespace_unregisters_its_python_functions() {
        let (m, _bus) = manager_and_bus();
        m.register(reg("pyapp::a", "def handler(payload):\n    return 'a'"))
            .await
            .unwrap();
        m.register(reg("pyapp::b", "def handler(payload):\n    return 'b'"))
            .await
            .unwrap();

        let torn = m
            .teardown(TeardownRequest {
                runtime_id: None,
                namespace: Some("pyapp".into()),
            })
            .await
            .unwrap();
        assert!(torn.torn_down);
        assert_eq!(torn.namespace.as_deref(), Some("pyapp::"));
        assert_eq!(torn.unregistered, vec!["pyapp::a", "pyapp::b"]);

        // The ids are free again.
        m.register(reg("pyapp::a", "def handler(payload):\n    return 'again'"))
            .await
            .expect("teardown must release the ids it unregistered");
    }

    /// A namespace nothing registered under is still not found — teardown must
    /// not report success for a namespace that never existed.
    #[tokio::test]
    async fn tearing_down_an_unknown_namespace_is_not_found() {
        let (m, _bus) = manager_and_bus();
        let err = m
            .teardown(TeardownRequest {
                runtime_id: None,
                namespace: Some("ghost".into()),
            })
            .await
            .unwrap_err();
        // `NamespaceNotFound` shares `runtime_not_found` on the wire — one
        // code for "the thing you addressed is not here", which is what
        // sandbox-code-runner returns too.
        assert_eq!(err.code(), "code-runner::runtime_not_found", "{err}");
    }

    /// A handler that raises comes back as an error to the bus, carrying the
    /// tenant's own message — not a generic engine failure.
    #[tokio::test]
    async fn a_raising_handler_answers_the_bus_with_its_own_message() {
        let (m, bus) = manager_and_bus();
        m.register(reg(
            "pyapp::boom",
            "def handler(payload):\n    raise ValueError('handler said no')",
        ))
        .await
        .unwrap();
        let err = bus
            .invoke("pyapp::boom", serde_json::json!(null))
            .await
            .unwrap_err();
        assert!(err.contains("handler said no"), "{err}");
    }
    /// Every id this worker owns is claimed in the registry, so no caller can
    /// register one — including the console UI's content function, which is
    /// published by `ui::register` rather than by `register_all`.
    ///
    /// This is not hygiene. `register_function` lets a caller choose ANY
    /// namespace, `code-runner::` included, and an unseeded worker id is
    /// claimable — the claim then reaches `iii_sdk`'s `register_function` on an
    /// already-registered id, and its duplicate-id panic ABORTS THE PROCESS.
    ///
    /// Mutation: seed `STATIC_IDS` instead of `seeded_ids()`. The `ui-content`
    /// case then returns `registered: true` — exactly what it did before this
    /// was fixed, and a live worker dies on that call.
    #[tokio::test]
    async fn no_caller_can_claim_an_id_this_worker_owns() {
        let (m, _bus) = manager_and_bus();
        for owned in [
            "code-runner::run",
            "code-runner::teardown",
            "code-runner::register_function",
            "code-runner::inject-guidance",
            crate::ui::CONTENT_FUNCTION_ID,
        ] {
            let err = m
                .register(reg(owned, "def handler(payload):\n    return 1"))
                .await
                .err()
                .unwrap_or_else(|| panic!("{owned} was claimable — a live worker aborts on this"));
            assert!(err.to_string().contains("already taken"), "{owned}: {err}");
        }
    }
}
