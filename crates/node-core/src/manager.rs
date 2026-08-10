//! Ownership and lifecycle for the live isolates.
//!
//! Evals against one runtime are serialised by a per-runtime async mutex, so
//! the log buffer and the registration delta belong unambiguously to one
//! caller. Handler invocations still interleave — they come in through the
//! runtime's own command channel, not through here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use crate::config::NodeEngineConfig;
use crate::engine::Engine;
use crate::error::NodeEngineError;
use crate::ops::{MAX_DESCRIPTION_BYTES, MAX_FUNCTION_ID_BYTES};
use crate::runtime::{Command, RuntimeOpts, RuntimeThread, Unregisters};
use crate::wire::register::{RegisterRequest, RegisterResponse};
use crate::wire::run::{RunRequest, RunResponse};
use crate::wire::teardown::{TeardownRequest, TeardownResponse};

/// A worker name becomes a namespace prefix and a display name, so it is kept
/// to a shape that cannot change how an id parses and cannot render two
/// different names alike: lowercase ASCII, digits, and `.`/`_`/`-`.
const MAX_WORKER_NAME_BYTES: usize = 64;

/// A namespace's first segment IS a worker name on the bus (see
/// `normalize_namespace`), so it is held to the same shape upstream
/// `iii-worker` requires of one (`iii-worker/src/core/types.rs`): a name
/// becomes a path segment under `~/.iii/logs/<name>` in that program, so
/// being looser here would accept a namespace stem this crate happily runs
/// with, only for `iii worker add` to refuse the equivalent name minutes
/// later, in another program.
fn validate_worker_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("worker name must not be empty; it becomes the namespace, \
                    so pass something like \"my-app\""
            .to_string());
    }
    if name.len() > MAX_WORKER_NAME_BYTES {
        return Err(format!(
            "worker name is {} bytes; the limit is {MAX_WORKER_NAME_BYTES}",
            name.len()
        ));
    }
    if name.contains("::") {
        return Err(format!(
            "worker name {name:?} must not contain \"::\" — the name IS the \
             namespace, so pass \"my-app\", not \"my-app::\""
        ));
    }
    if name.contains("..") {
        return Err(format!(
            "worker name {name:?} must not contain \"..\" — it becomes a \
             directory name, so pass \"my-app\", not \"my..app\""
        ));
    }
    if name.starts_with('.') {
        return Err(format!(
            "worker name {name:?} must not start with \".\" — it becomes a \
             directory name, so pass \"my-app\", not \".my-app\""
        ));
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')))
    {
        return Err(format!(
            "worker name {name:?} contains {bad:?}; use lowercase letters, \
             digits, and \".\", \"_\" or \"-\""
        ));
    }
    Ok(())
}

/// Canonical form of a caller-supplied namespace: `foo::`.
///
/// The guard in `op_iii_register` is a prefix test, so the prefix's shape
/// carries its whole security value: `""` would let a runtime register
/// `state::get`, and `"app"` would let it register `application::anything`.
/// The ONE near-miss that is repaired rather than rejected is the delimiter:
/// trailing colons are stripped and exactly two appended, so `app`, `app:`,
/// and `app::` all become `app::`. Every result is a strictly NARROWER prefix
/// than the string the caller passed, so normalizing can only reduce what a
/// runtime may register, never widen it. Writing the namespace without its
/// trailing `::` is the mistake everyone makes first, because the ids read that
/// way in prose.
///
/// What survives that trim must be ONE worker name — literally
/// `validate_worker_name`, because a namespace's first segment IS a worker
/// name on the bus: the engine splits `service::sub::function` into service
/// `service` (`iii/engine/src/services.rs`), and a worker name may never
/// contain `:` (`iii-worker/src/core/types.rs`). So `a::b::` names an identity
/// the engine cannot represent, and is refused rather than published.
///
/// Nothing expressive is lost, which is the non-obvious part: the guard is a
/// PREFIX test, so `myapp::v2::save` still registers under namespace `myapp::`.
/// Only the namespace itself is held to one segment.
///
/// Deliberately STRICTER than upstream `iii`, which allows any
/// `is_ascii_alphanumeric` in a worker name (`types.rs`): `MyApp` is refused
/// here though `iii` would install it. LOOSER than upstream would be the bug,
/// since it defers the refusal to `iii worker add`, in another program,
/// minutes later.
///
/// `Err` is the message the caller sees. `""`, `":"` and `"::"` leave nothing
/// to build a prefix from and keep their own diagnosis, which draws a concrete
/// suggestion from `ids` — hence that parameter, empty where the caller sent
/// none to draw on.
/// Canonicalise a namespace and enforce the charset rule.
///
/// Public because `code-runner` registers **python** handlers too, and the two
/// languages must agree byte-for-byte on what a legal namespace is. A second
/// copy of this rule elsewhere is how `app::x` becomes legal in one language
/// and not the other — and the rule is not cosmetic: the charset check is what
/// closed a YAML-injection reachable through the old `eject`.
pub fn normalize_namespace(ns: &str, ids: &[String]) -> Result<String, String> {
    let stem = ns.trim_end_matches(':');
    if stem.is_empty() {
        return Err(empty_namespace_message(ns, ids));
    }
    validate_worker_name(stem).map_err(|why| invalid_namespace_message(ns, stem, &why))?;
    Ok(format!("{stem}::"))
}

/// The one namespace we cannot repair. Suggests a concrete value taken from
/// the caller's own ids when there are any, because "write a prefix" is advice
/// and `"app::"` is an answer.
fn empty_namespace_message(ns: &str, ids: &[String]) -> String {
    let suggestion = ids
        .iter()
        .find_map(|id| id.split_once("::"))
        .map(|(head, _)| format!("{head}::"));
    match suggestion {
        Some(s) => format!(
            "namespace {ns:?} names no prefix. Your ids start with {s:?} — pass that as the \
             namespace."
        ),
        None => format!(
            "namespace {ns:?} names no prefix. Pass the prefix your ids start with, such as \
             \"app::\" for ids like \"app::save\"."
        ),
    }
}

/// The other namespace we cannot repair: a stem that is not one worker name.
///
/// `validate_worker_name`'s own text is written for a `name` field, and its
/// `"::"` case tells the caller to drop the delimiter a namespace legitimately
/// ends with — advice that is simply wrong here. So that one reason is
/// restated, and every reason is wrapped in a sentence that says what a
/// namespace has to be and what nesting still works.
fn invalid_namespace_message(ns: &str, stem: &str, why: &str) -> String {
    let reason = if stem.contains("::") {
        format!("{stem:?} is more than one segment")
    } else {
        why.to_string()
    };
    format!(
        "namespace {ns:?} is not one worker name: {reason}. A namespace's first segment IS the \
         worker name on the bus, so pass a single segment — \"my-app::\", with lowercase letters, \
         digits, \".\", \"_\" or \"-\". Ids may still nest below it: \"my-app::v2::save\" \
         registers fine under namespace \"my-app::\"."
    )
}

struct Runtime {
    thread: RuntimeThread,
    /// Serialises evals so per-eval logs and registrations are exact.
    eval_lock: Arc<tokio::sync::Mutex<()>>,
    unregisters: Unregisters,
    /// Shared with the isolate thread via `RuntimeOpts.last_activity` — see
    /// `ops::OpsState::last_activity` for why an owned `Mutex` is not enough:
    /// an INVOKE proxied straight from the bus to the isolate's command
    /// channel never reaches `run`/`register`, so only a handle the isolate
    /// thread can also write through keeps such a runtime from looking idle.
    last_activity: Arc<Mutex<Instant>>,
    /// Required prefix for ids this runtime may register.
    namespace: String,
}

pub struct RuntimeManager {
    cfg: Arc<NodeEngineConfig>,
    engine: Arc<dyn Engine>,
    runtimes: Mutex<HashMap<String, Arc<Runtime>>>,
    ids: crate::ids::IdRegistry,
    /// Namespace → the runtime serving its registered functions. Distinct
    /// from `runtimes`, which is keyed by runtime id: this is the only way
    /// to reach a registration runtime, since its id is never returned to
    /// any caller.
    namespaces: Mutex<HashMap<String, String>>,
    /// Serialises namespace creation. Without it, two first-registrations
    /// racing on one namespace each create a runtime and one is orphaned —
    /// unreachable, holding a slot until the idle sweep.
    namespace_create_lock: Mutex<()>,
}

impl RuntimeManager {
    /// `worker_ids` are the function ids the hosting worker registers on its
    /// own client. They seed the [`crate::ids::IdRegistry`] so a runtime can
    /// never claim — and then abort on — one of them. This crate cannot know
    /// them: which functions a host registers is the host's business, so the
    /// list crosses the seam as an argument rather than being reached for.
    pub fn new(
        cfg: Arc<NodeEngineConfig>,
        engine: Arc<dyn Engine>,
        worker_ids: &[&str],
    ) -> Arc<Self> {
        Arc::new(Self {
            cfg,
            engine,
            runtimes: Mutex::new(HashMap::new()),
            ids: crate::ids::IdRegistry::with_worker_ids(worker_ids),
            namespaces: Mutex::new(HashMap::new()),
            namespace_create_lock: Mutex::new(()),
        })
    }

    /// The id registry this manager claims into.
    ///
    /// Public so `code-runner` can claim **python** function ids in the SAME
    /// registry. Two registries would each believe they owned `app::greet`,
    /// both would reach `iii_sdk`'s `register_function`, and the second would
    /// hit its duplicate-id panic — which aborts the process. Every path that
    /// can register a function must claim here, including the guest-driven
    /// `iii.registerFunction` op, which already does.
    pub fn ids(&self) -> &crate::ids::IdRegistry {
        &self.ids
    }

    /// The existing runtime for `namespace`, or a freshly created one —
    /// race-safe against two first-registrations landing on the same
    /// namespace at once (the "thundering herd"). Double-checked locking:
    /// the fast path never takes `namespace_create_lock` once a namespace's
    /// runtime exists, so steady-state registrations pay only the
    /// `namespaces` lock, not the create lock.
    ///
    /// Private rather than `pub(crate)`: its only caller (`register`) lives
    /// in this same module. `teardown`'s namespace dispatch deliberately
    /// does NOT call this — it must never mint a runtime just to destroy it,
    /// so it calls `live_namespace_runtime` directly instead.
    fn namespace_runtime(
        &self,
        namespace: &str,
    ) -> Result<(String, Arc<Runtime>), NodeEngineError> {
        // Fast path: an existing, still-live runtime.
        if let Some(rt) = self.live_namespace_runtime(namespace) {
            return Ok(rt);
        }
        let _create = self.namespace_create_lock.lock().unwrap();
        // Re-check under the lock — another caller may have created it
        // between the fast path and here.
        if let Some(rt) = self.live_namespace_runtime(namespace) {
            return Ok(rt);
        }
        let (id, runtime) = self.create(Some(namespace.to_string()))?;
        self.namespaces
            .lock()
            .unwrap()
            .insert(namespace.to_string(), id.clone());
        Ok((id, runtime))
    }

    /// The namespace's runtime, but only if it is still in `runtimes`. A
    /// torn-down or swept runtime leaves a stale map entry; handing that id
    /// back would answer with a runtime that no longer exists.
    fn live_namespace_runtime(&self, namespace: &str) -> Option<(String, Arc<Runtime>)> {
        let id = self.namespaces.lock().unwrap().get(namespace).cloned()?;
        let runtime = self.runtimes.lock().unwrap().get(&id).cloned()?;
        Some((id, runtime))
    }

    /// Live runtimes. Tests assert on this to prove disposal actually
    /// happened rather than trusting a returned error.
    pub fn live_runtime_count(&self) -> usize {
        self.runtimes.lock().unwrap().len()
    }

    fn create(&self, namespace: Option<String>) -> Result<(String, Arc<Runtime>), NodeEngineError> {
        let mut runtimes = self.runtimes.lock().unwrap();
        if runtimes.len() >= self.cfg.max_runtimes {
            return Err(NodeEngineError::Capacity(self.cfg.max_runtimes));
        }

        let runtime_id = format!("rt-{}", uuid::Uuid::new_v4());

        let namespace = match namespace {
            // Exempt from the one-segment rule by construction, not by flag:
            // this is the `None` arm, so it never passes through
            // `normalize_namespace` — which would refuse it, since
            // `code-runner::<runtime_id>` is two segments. Its FIRST segment
            // is `code-runner`, the hosting worker's own name on the bus, which
            // is what the rule actually protects; the runtime_id below it only
            // keeps two eval runtimes from colliding. Leave it: routing it
            // through validation would refuse every namespace-less eval.
            None => format!("code-runner::{runtime_id}::"),
            Some(ns) => normalize_namespace(&ns, &[]).map_err(NodeEngineError::InvalidRequest)?,
        };
        // Built BEFORE `spawn` and cloned into `RuntimeOpts` so the manager
        // and the isolate thread share the SAME clock rather than each
        // keeping its own — see `ops::OpsState::last_activity`.
        let last_activity = Arc::new(Mutex::new(Instant::now()));
        let thread = RuntimeThread::spawn(
            RuntimeOpts {
                heap_mb: self.cfg.heap_mb,
                external_mb: self.cfg.external_mb,
                namespace: namespace.clone(),
                call_timeout_ms: self.cfg.default_timeout_ms,
                max_timeout_ms: self.cfg.max_timeout_ms,
                ids: self.ids.clone(),
                runtime_id: runtime_id.clone(),
                last_activity: last_activity.clone(),
                scratch_mb: self.cfg.scratch_mb,
                scratch_files: self.cfg.scratch_files,
                scratch_root: self.cfg.scratch_root.clone(),
            },
            self.engine.clone(),
        );
        let runtime = Arc::new(Runtime {
            unregisters: thread.unregisters(),
            thread,
            eval_lock: Arc::new(tokio::sync::Mutex::new(())),
            last_activity,
            namespace,
        });
        runtimes.insert(runtime_id.clone(), runtime.clone());
        Ok((runtime_id, runtime))
    }

    fn lookup(&self, runtime_id: &str) -> Result<Arc<Runtime>, NodeEngineError> {
        self.runtimes
            .lock()
            .unwrap()
            .get(runtime_id)
            .cloned()
            .ok_or_else(|| NodeEngineError::RuntimeNotFound(runtime_id.to_string()))
    }

    pub async fn run(&self, req: RunRequest) -> Result<RunResponse, NodeEngineError> {
        let (runtime_id, runtime) = match req.runtime_id.as_deref() {
            Some(id) => {
                if req.namespace.is_some() {
                    return Err(NodeEngineError::InvalidRequest(
                        "namespace is accepted only when creating a runtime (omit runtime_id)"
                            .into(),
                    ));
                }
                (id.to_string(), self.lookup(id)?)
            }
            None => self.create(req.namespace.clone())?,
        };
        // Whether THIS call minted the runtime — the only case where its
        // lifetime is ours to decide. A caller-supplied `runtime_id` is
        // never disposed here regardless of `req.keep`: that runtime
        // belongs to the caller.
        let created = req.runtime_id.is_none();

        let timeout = self.cfg.clamp_timeout(req.timeout_ms);
        let _guard = runtime.eval_lock.clone().lock_owned().await;

        // Re-verify membership under the lock. Between `lookup` above and
        // here the sweeper or an explicit teardown may have removed this
        // runtime; running on would answer the caller "successfully" for an id
        // that no longer resolves and orphan anything this eval registers.
        if !self.runtimes.lock().unwrap().contains_key(&runtime_id) {
            return Err(NodeEngineError::RuntimeNotFound(runtime_id));
        }

        // Mark activity at the START of the eval, not only on completion. A
        // runtime idle for nearly the whole TTL that then receives an eval
        // would otherwise still look stale to the sweeper while it runs.
        *runtime.last_activity.lock().unwrap() = Instant::now();

        let (reply, rx) = oneshot::channel();
        if runtime
            .thread
            .send(Command::Eval {
                code: req.code,
                timeout,
                reply,
            })
            .is_err()
        {
            self.reap(&runtime_id);
            return Err(NodeEngineError::RuntimeGone(runtime_id));
        }

        let outcome = match rx.await {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => {
                // Timeout and OOM kill the isolate; drop it so callers get a
                // clean not-found instead of talking to a corpse.
                //
                // On a CREATE, reap on ANY error. The caller never received
                // this runtime_id and the error carries no id, so nothing can
                // ever address the runtime again — it holds one of
                // `max_runtimes` until the idle sweep, minutes later. A syntax
                // error is the ordinary case while someone iterates on a
                // snippet, so this leaked a slot per typo. Same reasoning as
                // the `register` failure arm.
                if req.runtime_id.is_none()
                    || matches!(e, NodeEngineError::Timeout | NodeEngineError::Oom)
                {
                    self.reap(&runtime_id);
                }
                return Err(e);
            }
            Err(_) => {
                self.reap(&runtime_id);
                return Err(NodeEngineError::RuntimeGone(runtime_id));
            }
        };

        *runtime.last_activity.lock().unwrap() = Instant::now();
        // Disposed on the way out when this call minted the runtime and
        // nobody asked to keep it: nothing survives to address, so
        // returning an id would be a lie.
        let keep_alive = !created || req.keep;
        if !keep_alive {
            // Drop the guard before reaping: `reap` takes the runtimes lock
            // and destroys the isolate thread this guard belongs to.
            drop(_guard);
            self.reap(&runtime_id);
        }
        Ok(RunResponse {
            runtime_id: keep_alive.then_some(runtime_id),
            result: outcome.result,
            logs: outcome.logs,
            registered: outcome.registered,
        })
    }

    /// One function, into its namespace's persistent runtime — created on the
    /// first registration, reused by every later one. The backing runtime and
    /// its id are never returned: this is the property that makes `teardown`'s
    /// namespace path necessary at all, since a caller has nothing else to
    /// address it by.
    pub async fn register(
        &self,
        req: RegisterRequest,
    ) -> Result<RegisterResponse, NodeEngineError> {
        let invalid = |m: String| NodeEngineError::InvalidRequest(m);

        if req.function_id.len() > MAX_FUNCTION_ID_BYTES {
            return Err(invalid(format!(
                "function id {:?}… is {} bytes; the limit is {MAX_FUNCTION_ID_BYTES}",
                req.function_id.chars().take(40).collect::<String>(),
                req.function_id.len()
            )));
        }
        if let Some(desc) = &req.description {
            if desc.len() > MAX_DESCRIPTION_BYTES {
                return Err(invalid(format!(
                    "description is {} bytes; the limit is {MAX_DESCRIPTION_BYTES}",
                    desc.len()
                )));
            }
        }

        // The namespace IS the first segment; an id with no `::` cannot name
        // one and is refused rather than silently placed in a default.
        let namespace = match req.function_id.split_once("::") {
            Some((head, _)) if !head.is_empty() => format!("{head}::"),
            _ => {
                return Err(invalid(format!(
                    "function id {:?} has no namespace; pass something like \
                     \"my-app::greet\"",
                    req.function_id
                )))
            }
        };
        // The existing validator, unchanged: it trims trailing colons and
        // applies the worker-name charset rule to the stem.
        let namespace = normalize_namespace(&namespace, std::slice::from_ref(&req.function_id))
            .map_err(invalid)?;

        // Claim BEFORE creating: a registration doomed by a taken id must not
        // have booted a runtime first.
        //
        // The owner passed here matters. `op_iii_register` (ops.rs) claims
        // every id it registers too, keyed by the RUNTIME's own id — the
        // check that stands between it and `Engine::register`'s panic on a
        // duplicate, and it fires for every path into that op, not just this
        // one. For that claim to succeed rather than see a foreign owner
        // where this call already approved one, the two have to agree.
        //
        // When the namespace already has a live runtime, its id is right
        // there — claim under it directly, and re-registering an id this
        // SAME runtime already holds is then just a `claim()` an owner
        // reclaims for free, so it survives this check unchanged. Only on
        // the very first registration for a namespace is there no runtime_id
        // yet to claim under; `&namespace` stands in for it until one
        // exists, then hands off — see the release right after
        // `namespace_runtime` below.
        let existing = self.live_namespace_runtime(&namespace).map(|(id, _)| id);
        let claim_owner = existing.as_deref().unwrap_or(&namespace);
        self.ids
            .claim_all(std::slice::from_ref(&req.function_id), claim_owner)
            .map_err(|_| NodeEngineError::IdTaken(req.function_id.clone()))?;

        let (runtime_id, runtime) = match self.namespace_runtime(&namespace) {
            Ok(rt) => rt,
            Err(e) => {
                self.ids
                    .release_ids(std::slice::from_ref(&req.function_id), claim_owner);
                return Err(e);
            }
        };

        // The placeholder claim only stood in for a runtime_id that did not
        // exist yet. One does now — hand off to it so `op_iii_register`'s own
        // claim (below, inside `define_handler`'s eval) sees an owner it
        // recognises rather than `&namespace`.
        if existing.is_none() {
            self.ids
                .release_ids(std::slice::from_ref(&req.function_id), &namespace);
        }

        match self.define_handler(&runtime_id, &runtime, &req).await {
            Ok(()) => Ok(RegisterResponse {
                function_id: req.function_id,
                namespace,
            }),
            Err(e) => {
                // Release ONLY if this id never reached the bus. On the reuse
                // path `claim_owner` IS the runtime that may already hold a
                // LIVE registration for this exact id from an earlier,
                // successful call — an ordinary typo'd redeploy fails before
                // `iii.registerFunction` ever runs, and unconditionally
                // releasing here would free that live claim out from under
                // it, letting a second runtime claim the same id and reach
                // `Engine::register` on a duplicate — the process abort
                // `op_iii_register`'s own claim exists to prevent. Mirrors the
                // old batch loop's guard: `unregisters` names every id that
                // provably reached the bus, so its absence is what "never
                // landed" actually means — checked and released under ONE
                // lock acquisition so a concurrent success cannot land in the
                // gap between the check and the release.
                //
                // Kind-AGNOSTIC on purpose, matching `OpsState::unregister`'s
                // predicate (ops.rs): `ids` tracks id ownership flatly, so
                // "may this claim be released" is "does ANY kind still hold
                // this id", never "does a Function hold it". Scoped to
                // `Function` this released a live TRIGGER TYPE's claim —
                // trivially reachable, since `req.source` runs arbitrary
                // statements in the namespace runtime and can call
                // `iii.registerTriggerType('app::x', …)` before failing to
                // define `handler`. The freed id then let a second runtime in
                // the same namespace claim it and silently clobber the first's
                // trigger-type handler.
                let held = runtime.unregisters.lock().unwrap();
                if !held.iter().any(|(_, id, _)| *id == req.function_id) {
                    self.ids
                        .release_ids(std::slice::from_ref(&req.function_id), claim_owner);
                }
                drop(held);
                Err(e)
            }
        }
    }

    /// Resolve `req.source` to `handler(payload)` inside `runtime`, and
    /// register it. Extracted from the old batch loop's per-function body —
    /// `wrap_register`, the resolved-form protocol, and its error mapping are
    /// all unchanged; only the shape feeding them is (one function, not a
    /// slice of `FunctionDef`). Routed through `run` rather than talking to
    /// `runtime.thread` directly so eval serialisation, activity tracking,
    /// and Timeout/Oom classification stay exactly what every other eval
    /// gets — this is not a special path.
    async fn define_handler(
        &self,
        runtime_id: &str,
        runtime: &Arc<Runtime>,
        req: &RegisterRequest,
    ) -> Result<(), NodeEngineError> {
        debug_assert!(
            req.function_id.starts_with(&runtime.namespace),
            "namespace_runtime({:?}) returned a runtime whose namespace does not cover {:?}",
            runtime.namespace,
            req.function_id
        );
        let code = crate::protocol::wrap_register(
            &req.function_id,
            &req.source,
            req.description.as_deref(),
        );
        self.run(RunRequest {
            code,
            runtime_id: Some(runtime_id.to_string()),
            namespace: None,
            // This IS the namespace's persistent runtime; a registration
            // attempt — successful or not — must never be the thing that
            // disposes it out from under a namespace other calls still rely
            // on. `run` only reaps a caller-supplied runtime on Timeout/Oom,
            // same as it would for any other eval into it.
            keep: true,
            timeout_ms: None,
        })
        .await
        // `run`'s `RuntimeNotFound`/`RuntimeGone` messages quote `runtime_id`
        // verbatim (error.rs) — fine for `run`'s own caller, who already
        // holds that id, but `register_function`'s caller never does and
        // must never learn it: it addresses the whole namespace's shared
        // runtime, not a runtime of their own. Reachable if the idle sweep
        // (or a concurrent teardown) drops this runtime between
        // `namespace_runtime` resolving it and `run`'s own re-check.
        // Redacted rather than dropping the variant, so the wire error CODE
        // stays what it always was.
        .map_err(|e| match e {
            NodeEngineError::RuntimeNotFound(_) => {
                NodeEngineError::RuntimeNotFound("<namespace-runtime>".into())
            }
            NodeEngineError::RuntimeGone(_) => {
                NodeEngineError::RuntimeGone("<namespace-runtime>".into())
            }
            other => other,
        })?;
        Ok(())
    }

    pub async fn teardown_runtime(
        &self,
        runtime_id: &str,
    ) -> Result<TeardownResponse, NodeEngineError> {
        // Look up WITHOUT removing, so the eval lock is taken first.
        let runtime = self.lookup(runtime_id)?;

        // Wait for any in-flight eval. Tearing down underneath one lets it run
        // to completion and answer its caller "successfully" for a runtime that
        // no longer exists — and anything it registers after `destroy` drains
        // the list becomes a permanent bus orphan: the id no longer resolves,
        // so nothing can ever unregister it. Bounded wait: an eval holds this
        // for at most its own clamped timeout.
        let _eval_guard = runtime.eval_lock.clone().lock_owned().await;

        // Re-check under the guard — a concurrent teardown or the sweeper may
        // have won the race while we waited.
        let removed = self.runtimes.lock().unwrap().remove(runtime_id);
        let Some(removed) = removed else {
            return Err(NodeEngineError::RuntimeNotFound(runtime_id.to_string()));
        };
        Ok(TeardownResponse {
            unregistered: self.destroy(runtime_id, removed),
        })
    }

    /// Dispatches on which selector the caller sent — exactly one of
    /// `runtime_id` or `namespace` — and delegates to `teardown_runtime`.
    ///
    /// The namespace path resolves through `live_namespace_runtime`, which
    /// looks up WITHOUT creating: `namespace_runtime` would mint a fresh
    /// runtime for an unknown namespace just to immediately destroy it,
    /// turning a not-found into a false "torn down".
    pub async fn teardown(
        &self,
        req: TeardownRequest,
    ) -> Result<TeardownResponse, NodeEngineError> {
        let runtime_id = match (req.runtime_id, req.namespace) {
            (Some(id), None) => id,
            (None, Some(ns)) => {
                let ns = normalize_namespace(&ns, &[]).map_err(NodeEngineError::InvalidRequest)?;
                self.live_namespace_runtime(&ns)
                    .map(|(id, _)| id)
                    .ok_or(NodeEngineError::RuntimeNotFound(ns))?
            }
            _ => {
                return Err(NodeEngineError::InvalidRequest(
                    "pass exactly one of runtime_id or namespace".into(),
                ));
            }
        };
        self.teardown_runtime(&runtime_id).await
    }

    /// Remove a runtime that already died, ignoring the case where another
    /// caller got there first.
    fn reap(&self, runtime_id: &str) {
        // Two statements on purpose: an `if let` would hold the map's guard
        // for the whole body, and `destroy` blocks joining the isolate thread.
        let removed = self.runtimes.lock().unwrap().remove(runtime_id);
        if let Some(runtime) = removed {
            self.destroy(runtime_id, runtime);
        }
    }

    fn destroy(&self, runtime_id: &str, runtime: Arc<Runtime>) -> Vec<String> {
        // Both callers (`reap` and `teardown_runtime`) already removed this
        // id from `runtimes` before calling in, so this is the one place a
        // dying runtime's id can still be reached: through a namespace entry
        // pointing at it. Pruned here rather than duplicated in both
        // callers, so `namespaces` cannot grow one stale entry per teardown
        // no matter which path disposed of the runtime.
        self.namespaces
            .lock()
            .unwrap()
            .retain(|_, id| id != runtime_id);
        let mut ids = Vec::new();
        // ONE critical section spanning the drain AND the release, held against
        // `op_iii_register`'s claim→register→push. Splitting them lets a
        // registration land on the bus after its claim was freed, leaving an id
        // live, unclaimed and impossible to unregister — the abort this registry
        // closes. The only release point that frees a whole owner: no failure
        // path releases wholesale, because a registration committed before a
        // mid-batch throw stays live on the bus (`register`'s failure arm
        // releases only the ids that provably never landed).
        {
            let mut held = runtime.unregisters.lock().unwrap();
            for (_, id, unregister) in held.drain(..) {
                unregister();
                ids.push(id);
            }
            self.ids.release_owner(runtime_id);
        }
        // NOT necessarily the last Arc: both callers keep their own clone
        // alive for the rest of their body, so the channel-close and the
        // thread join actually happen at the caller's scope exit — after its
        // `eval_lock` guard has dropped, which is what releases a waiting eval
        // first.
        drop(runtime);
        ids
    }

    /// Drop runtimes idle for longer than `idle_ttl_secs`. Returns the ids
    /// reaped, for logging by the caller.
    pub fn sweep_idle(&self) -> Vec<String> {
        let ttl = Duration::from_secs(self.cfg.idle_ttl_secs);
        let candidates: Vec<(String, Arc<Runtime>)> = {
            let runtimes = self.runtimes.lock().unwrap();
            runtimes
                .iter()
                .filter(|(_, r)| r.last_activity.lock().unwrap().elapsed() >= ttl)
                .map(|(id, r)| (id.clone(), r.clone()))
                .collect()
        };

        let mut reaped = Vec::new();
        for (id, runtime) in candidates {
            // Never reap a runtime with an eval in flight. Killing one mid-eval
            // hands its caller a success for a runtime_id that no longer
            // exists, and orphans forever any function the eval's tail
            // registered after `destroy` took its snapshot — nothing can
            // unregister those, because the id can no longer be looked up.
            let Ok(_eval_guard) = runtime.eval_lock.try_lock() else {
                continue;
            };
            // Re-check under that guard: an eval may have finished between the
            // scan above and now, bumping `last_activity`.
            if runtime.last_activity.lock().unwrap().elapsed() < ttl {
                continue;
            }
            self.reap(&id);
            reaped.push(id);
        }
        reaped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::FakeEngine;

    /// Stand-in for the ids a hosting worker registers on its own client.
    /// The real list is the worker's (`node_engine::functions::STATIC_IDS`);
    /// this crate only needs one of them to prove the seed is honoured — see
    /// `a_runtime_cannot_claim_an_id_the_worker_owns`.
    const TEST_WORKER_IDS: &[&str] = &[
        "node-engine::run",
        "node-engine::teardown",
        "node-engine::register_function",
        "node-engine::inject-guidance",
    ];
    use crate::wire::register::RegisterRequest;
    use crate::wire::run::RunRequest;
    use serde_json::json;

    fn manager(cfg: NodeEngineConfig) -> (Arc<RuntimeManager>, Arc<FakeEngine>) {
        crate::runtime::init_v8_platform();
        let fake = FakeEngine::new();
        (
            RuntimeManager::new(Arc::new(cfg), fake.clone(), TEST_WORKER_IDS),
            fake,
        )
    }

    fn req(code: &str, runtime_id: Option<&str>) -> RunRequest {
        RunRequest {
            code: code.to_string(),
            runtime_id: runtime_id.map(str::to_string),
            namespace: None,
            keep: false,
            timeout_ms: Some(2_000),
        }
    }

    /// `req(...)` with `keep: true` — for tests whose runtime must survive
    /// the call, to be reused or torn down afterward. `req`'s own default is
    /// `keep: false` on purpose (most tests want the ordinary one-shot
    /// path), so this is the opt-in for the rest.
    fn kept(mut r: RunRequest) -> RunRequest {
        r.keep = true;
        r
    }

    /// The id out of a response built with `kept`. Panics rather than
    /// silently building a fresh-runtime request if a test forgets `kept`
    /// somewhere upstream — a `None` here means the assumption the test
    /// relies on already broke.
    fn addr(r: &RunResponse) -> &str {
        r.runtime_id
            .as_deref()
            .expect("test runtime must have been kept alive")
    }

    #[test]
    fn accepts_ordinary_worker_names() {
        // "my.app" and "app.v2" are a single INTERIOR dot each — not a
        // leading dot and not ".." — so they stay accepted; only the two
        // upstream directory-safety shapes are refused, below.
        for name in ["app", "my-app", "my_app", "app.v2", "my.app", "a", "app2"] {
            assert!(
                validate_worker_name(name).is_ok(),
                "{name:?} should be valid"
            );
        }
    }

    /// The name becomes a namespace prefix, so anything that could change how
    /// an id parses, or make two names render alike, is refused.
    #[test]
    fn refuses_names_that_are_not_safe_as_a_prefix() {
        for bad in ["", "App", "my app", "app::x", "app:", "a/b", "a\nb", "ünï"] {
            let err = validate_worker_name(bad).unwrap_err();
            assert!(!err.is_empty(), "{bad:?} must explain itself");
        }
        let long = "a".repeat(MAX_WORKER_NAME_BYTES + 1);
        assert!(validate_worker_name(&long).is_err());
    }

    /// Mirrors upstream `iii-worker`'s own directory-safety rules: a name
    /// that could traverse out of, or land inside, a hidden control
    /// directory once joined into `~/.iii/logs/<name>`. Being looser than
    /// upstream here is the bug this test guards against — see the doc
    /// comment on `validate_worker_name`.
    #[test]
    fn refuses_dotdot_and_leading_dot() {
        for bad in ["..", "a..b", ".", ".hidden"] {
            let err = validate_worker_name(bad).unwrap_err();
            assert!(!err.is_empty(), "{bad:?} must explain itself");
        }
    }

    #[tokio::test]
    async fn eval_without_a_runtime_id_creates_one() {
        let (m, _) = manager(NodeEngineConfig::default());
        let out = m.run(kept(req("return 1", None))).await.unwrap();
        assert!(out.runtime_id.is_some());
        assert_eq!(out.result, json!(1));
        m.teardown_runtime(addr(&out)).await.unwrap();
    }

    #[tokio::test]
    async fn eval_with_a_runtime_id_reuses_the_isolate() {
        let (m, _) = manager(NodeEngineConfig::default());
        let first = m
            .run(kept(req("globalThis.n = 5; return null", None)))
            .await
            .unwrap();
        let second = m
            .run(req("return globalThis.n", Some(addr(&first))))
            .await
            .unwrap();
        assert_eq!(second.runtime_id.as_deref(), Some(addr(&first)));
        assert_eq!(second.result, json!(5));
        m.teardown_runtime(addr(&first)).await.unwrap();
    }

    #[tokio::test]
    async fn unknown_runtime_id_is_runtime_not_found() {
        let (m, _) = manager(NodeEngineConfig::default());
        let err = m.run(req("return 1", Some("rt-nope"))).await.unwrap_err();
        assert_eq!(err.code(), "node-engine::runtime_not_found");
        assert_eq!(
            m.teardown_runtime("rt-nope").await.unwrap_err().code(),
            "node-engine::runtime_not_found"
        );
    }

    #[tokio::test]
    async fn namespace_with_an_existing_runtime_is_an_invalid_request() {
        let (m, _) = manager(NodeEngineConfig::default());
        let first = m.run(kept(req("return 1", None))).await.unwrap();
        let mut second = req("return 1", Some(addr(&first)));
        second.namespace = Some("other::".into());
        assert_eq!(
            m.run(second).await.unwrap_err().code(),
            "node-engine::invalid_request"
        );
        m.teardown_runtime(addr(&first)).await.unwrap();
    }

    #[tokio::test]
    async fn the_default_namespace_is_derived_from_the_runtime_id() {
        let (m, fake) = manager(NodeEngineConfig::default());
        let created = m.run(kept(req("return 1", None))).await.unwrap();
        let ns = format!("code-runner::{}::", addr(&created));

        let ok = m
            .run(req(
                &format!("iii.registerFunction('{ns}hello', () => 1); return null"),
                Some(addr(&created)),
            ))
            .await
            .unwrap();
        assert_eq!(ok.registered, vec![format!("{ns}hello")]);
        assert_eq!(fake.registered_ids(), vec![format!("{ns}hello")]);

        // Anything outside that prefix is refused.
        let denied = m
            .run(req(
                "try { iii.registerFunction('state::get', () => 1); return 'no throw' } \
                 catch (e) { return 'denied' }",
                Some(addr(&created)),
            ))
            .await
            .unwrap();
        assert_eq!(denied.result, json!("denied"));
        assert_eq!(fake.registered_ids().len(), 1);

        m.teardown_runtime(addr(&created)).await.unwrap();
    }

    /// The guard is a prefix test, so an EMPTY prefix would silently disable
    /// it. A prefix merely missing its delimiter is repaired instead — see
    /// `normalize_namespace` — so these are the values that name no prefix at
    /// all. A stem that is not one worker name is refused too, by the same
    /// function; `a_namespace_that_is_not_one_worker_name_is_refused` covers it.
    #[tokio::test]
    async fn a_malformed_namespace_is_rejected() {
        let (m, _) = manager(NodeEngineConfig::default());
        for bad in ["", "::", ":"] {
            let mut req = req("return 1", None);
            req.namespace = Some(bad.to_string());
            assert_eq!(
                m.run(req).await.unwrap_err().code(),
                "node-engine::invalid_request",
                "namespace {bad:?} should have been rejected"
            );
        }
        assert_eq!(
            m.live_runtime_count(),
            0,
            "a rejected namespace must not leak a runtime"
        );
    }

    #[tokio::test]
    async fn a_custom_namespace_is_honoured_on_creation() {
        let (m, _) = manager(NodeEngineConfig::default());
        let mut first = req(
            "iii.registerFunction('app::go', () => 1); return null",
            None,
        );
        first.namespace = Some("app::".into());
        let out = m.run(kept(first)).await.unwrap();
        assert_eq!(out.registered, vec!["app::go".to_string()]);
        m.teardown_runtime(addr(&out)).await.unwrap();
    }

    #[tokio::test]
    async fn teardown_unregisters_everything_and_frees_the_slot() {
        let (m, fake) = manager(NodeEngineConfig {
            max_runtimes: 1,
            ..Default::default()
        });
        let mut first = req(
            "iii.registerFunction('app::go', () => 1); return null",
            None,
        );
        first.namespace = Some("app::".into());
        let out = m.run(kept(first)).await.unwrap();

        let torn = m.teardown_runtime(addr(&out)).await.unwrap();
        assert_eq!(torn.unregistered, vec!["app::go".to_string()]);
        assert_eq!(fake.unregister_count(), 1);

        // Slot is free again — and with the default one-shot behaviour, this
        // call disposes its own runtime immediately, freeing the slot again
        // too, so there is nothing left to tear down afterward.
        m.run(req("return 1", None)).await.unwrap();
    }

    #[tokio::test]
    async fn exceeding_max_runtimes_is_a_capacity_error() {
        let (m, _) = manager(NodeEngineConfig {
            max_runtimes: 1,
            ..Default::default()
        });
        // Must occupy the slot for the second call to hit capacity: with the
        // default one-shot behaviour it would free its own slot immediately.
        let first = m.run(kept(req("return 1", None))).await.unwrap();
        assert_eq!(
            m.run(req("return 1", None)).await.unwrap_err().code(),
            "node-engine::capacity"
        );
        m.teardown_runtime(addr(&first)).await.unwrap();
    }

    #[tokio::test]
    async fn a_timed_out_runtime_is_reaped_so_the_next_eval_is_not_found() {
        let (m, _) = manager(NodeEngineConfig::default());
        let created = m.run(kept(req("return 1", None))).await.unwrap();
        let mut busy = req("for(;;){}", Some(addr(&created)));
        busy.timeout_ms = Some(300);
        assert_eq!(
            m.run(busy).await.unwrap_err().code(),
            "node-engine::timeout"
        );
        assert_eq!(
            m.run(req("return 1", Some(addr(&created))))
                .await
                .unwrap_err()
                .code(),
            "node-engine::runtime_not_found"
        );
    }

    /// The dangerous case is not "sweep after an eval" but "sweep DURING one":
    /// a runtime idle for nearly the TTL that receives a fresh eval.
    #[tokio::test]
    async fn the_idle_sweep_does_not_reap_a_runtime_mid_eval() {
        let (m, _) = manager(NodeEngineConfig {
            idle_ttl_secs: 0,
            ..Default::default()
        });
        let created = m.run(kept(req("return 1", None))).await.unwrap();
        let id = addr(&created).to_string();

        // A slow eval holds this runtime's eval lock while the sweep runs.
        let manager_handle = m.clone();
        let eval_id = id.clone();
        let slow = tokio::spawn(async move {
            let mut r = req("await new Promise(() => {}); return 1", Some(&eval_id));
            r.timeout_ms = Some(1_500);
            manager_handle.run(r).await
        });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        assert!(
            m.sweep_idle().is_empty(),
            "swept a runtime with an eval in flight"
        );

        // The eval owns its own fate: it times out rather than being told the
        // runtime vanished underneath it.
        assert_eq!(
            slow.await.unwrap().unwrap_err().code(),
            "node-engine::timeout"
        );
    }

    #[tokio::test]
    async fn the_idle_sweep_reaps_stale_runtimes() {
        let (m, _) = manager(NodeEngineConfig {
            idle_ttl_secs: 0,
            ..Default::default()
        });
        let created = m.run(kept(req("return 1", None))).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(m.sweep_idle(), vec![addr(&created).to_string()]);
        assert_eq!(
            m.run(req("return 1", Some(addr(&created))))
                .await
                .unwrap_err()
                .code(),
            "node-engine::runtime_not_found"
        );
    }

    /// The bug this guards: an INVOKE proxied straight from the bus to the
    /// isolate's command channel (`op_iii_register`'s handler, in `ops.rs`)
    /// never passes through `RuntimeManager::run`/`register`, so a runtime
    /// doing exactly what it exists for — sitting on the bus and answering
    /// calls — used to look idle to `sweep_idle` and get reaped mid-service.
    ///
    /// One second of real sleep on each side is unavoidable: `idle_ttl_secs`
    /// is whole seconds, and the assertion only means anything if the runtime
    /// is aged PAST the ttl before each check — a check taken immediately
    /// after registering would pass even without the fix, since `register`
    /// itself just set `last_activity`.
    #[tokio::test]
    async fn an_invoke_through_the_registered_proxy_counts_as_activity() {
        let (m, fake) = manager(NodeEngineConfig {
            idle_ttl_secs: 1,
            ..Default::default()
        });
        m.register(reg(
            "app::ping",
            "export function handler(p) { return p.n + 1 }",
            None,
        ))
        .await
        .unwrap();
        // The namespace runtime's id is never returned by `register` — this
        // test reaches it directly through the same-module `namespace_runtime`
        // lookup (`teardown`'s own namespace dispatch uses the non-creating
        // `live_namespace_runtime` instead, since it must never mint a
        // runtime just to destroy it).
        let (runtime_id, _) = m.namespace_runtime("app::").unwrap();

        // Age the runtime past the ttl BEFORE invoking it, so a clean sweep
        // afterward can only be explained by the invoke itself resetting the
        // clock.
        tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
        assert_eq!(
            fake.invoke("app::ping", json!({ "n": 1 })).await,
            Ok(json!(2))
        );

        assert!(
            m.sweep_idle().is_empty(),
            "an invoke through the registered proxy must count as activity"
        );
        assert_eq!(m.live_runtime_count(), 1);

        // No further activity: the SAME runtime is now stale on the SAME ttl.
        tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
        assert_eq!(m.sweep_idle(), vec![runtime_id]);
        assert_eq!(m.live_runtime_count(), 0);
    }

    /// The regression test for a process abort: with the real `iii-sdk`, the
    /// second registration reaches a `panic!` inside a `#[op2(fast)]` callback
    /// and kills the worker.
    #[tokio::test]
    async fn the_same_id_from_two_runtimes_is_refused_and_the_worker_survives() {
        let (m, _) = manager(NodeEngineConfig::default());

        let mut first = req(
            "iii.registerFunction('app::dup', () => 1); return 'ok'",
            None,
        );
        first.namespace = Some("app::".into());
        let a = m.run(kept(first)).await.unwrap();
        assert_eq!(a.registered, vec!["app::dup".to_string()]);

        let mut second = req(
            "try { iii.registerFunction('app::dup', () => 2); return 'registered' } \
             catch (e) { return 'refused' }",
            None,
        );
        second.namespace = Some("app::".into());
        let b = m.run(kept(second)).await.unwrap();
        assert_eq!(b.result, json!("refused"));
        assert!(b.registered.is_empty());

        // Still serving: the process did not die.
        let c = m.run(kept(req("return 1 + 1", None))).await.unwrap();
        assert_eq!(c.result, json!(2));

        m.teardown_runtime(addr(&a)).await.unwrap();
        m.teardown_runtime(addr(&b)).await.unwrap();
        m.teardown_runtime(addr(&c)).await.unwrap();
    }

    /// The highest-value scratch test: four lifecycle exits, one assertion
    /// shape. `scratch_root` makes the directory observable from the host
    /// without ever handing a path to the guest.
    ///
    /// Mutation: `std::mem::forget` the `TempDir`, or move it into a
    /// `static` — the count stays at 1 after every exit.
    #[tokio::test]
    async fn the_scratch_directory_dies_with_its_runtime() {
        let root = tempfile::Builder::new()
            .prefix("node-core-lifecycle-")
            .tempdir()
            .expect("tempdir");
        let count = || {
            std::fs::read_dir(root.path())
                .map(|d| d.flatten().count())
                .unwrap_or(0)
        };
        let cfg = NodeEngineConfig {
            scratch_root: Some(root.path().to_string_lossy().into_owned()),
            idle_ttl_secs: 0,
            ..NodeEngineConfig::default()
        };
        let (m, _) = manager(cfg);

        // 1. explicit teardown
        let out = m
            .run(kept(req("iii.files.write('a', 'x'); return 1", None)))
            .await
            .unwrap();
        assert_eq!(count(), 1, "a kept runtime must own a directory");
        m.teardown_runtime(addr(&out)).await.unwrap();
        assert_eq!(count(), 0, "teardown must remove it");

        // 2. one-shot: created and destroyed inside the call
        m.run(req("iii.files.write('a', 'x'); return 1", None))
            .await
            .unwrap();
        assert_eq!(count(), 0, "a one-shot run must leave nothing behind");

        // 3. the idle sweep
        m.run(kept(req("return 1", None))).await.unwrap();
        assert_eq!(count(), 1);
        let reaped = m.sweep_idle();
        assert_eq!(reaped.len(), 1, "the sweep must have reaped it");
        assert_eq!(count(), 0, "the sweep must remove its directory");

        // 4. a timeout kill
        let killed = m
            .run(kept(RunRequest {
                timeout_ms: Some(50),
                ..req("while (true) {}", None)
            }))
            .await;
        assert!(killed.is_err(), "the runaway must have been killed");
        assert_eq!(count(), 0, "a killed runtime must not leak its directory");
    }

    /// `scratch_mb: 0` is the operator kill switch: no directory, and the
    /// guest surface is gone rather than present-and-refusing. Mutation:
    /// delete the `drop_files` interpolation in `runtime.rs`.
    #[tokio::test]
    async fn scratch_mb_zero_removes_the_guest_surface() {
        let cfg = NodeEngineConfig {
            scratch_mb: 0,
            ..NodeEngineConfig::default()
        };
        let (m, _) = manager(cfg);
        let out = m
            .run(req("return typeof iii.files === 'undefined'", None))
            .await
            .unwrap();
        assert_eq!(out.result, json!(true));
    }

    /// `namespace: "node-engine::"` passes shape validation, so without the
    /// worker-id seed a single request could claim `node-engine::run`, reach
    /// `engine.register`, and abort.
    #[tokio::test]
    async fn a_runtime_cannot_claim_an_id_the_worker_owns() {
        let (m, _) = manager(NodeEngineConfig::default());
        let mut r = req(
            "try { iii.registerFunction('node-engine::run', () => 1); return 'registered' } \
             catch (e) { return 'refused' }",
            None,
        );
        r.namespace = Some("node-engine::".into());
        let out = m.run(kept(r)).await.unwrap();
        assert_eq!(out.result, json!("refused"));

        let alive = m.run(kept(req("return 'alive'", None))).await.unwrap();
        assert_eq!(alive.result, json!("alive"));

        m.teardown_runtime(addr(&out)).await.unwrap();
        m.teardown_runtime(addr(&alive)).await.unwrap();
    }

    #[tokio::test]
    async fn teardown_releases_the_claim_for_a_later_runtime() {
        let (m, _) = manager(NodeEngineConfig::default());
        let mut first = req("iii.registerFunction('app::x', () => 1); return 'ok'", None);
        first.namespace = Some("app::".into());
        let a = m.run(kept(first)).await.unwrap();
        m.teardown_runtime(addr(&a)).await.unwrap();

        let mut second = req("iii.registerFunction('app::x', () => 2); return 'ok'", None);
        second.namespace = Some("app::".into());
        let b = m.run(kept(second)).await.unwrap();
        assert_eq!(b.registered, vec!["app::x".to_string()]);
        m.teardown_runtime(addr(&b)).await.unwrap();
    }

    #[tokio::test]
    async fn re_registering_within_one_runtime_still_succeeds() {
        let (m, _) = manager(NodeEngineConfig::default());
        let mut r = req(
            "iii.registerFunction('app::y', () => 1); \
             iii.registerFunction('app::y', () => 2); \
             return 'ok'",
            None,
        );
        r.namespace = Some("app::".into());
        let out = m.run(kept(r)).await.unwrap();
        assert_eq!(out.result, json!("ok"));
        m.teardown_runtime(addr(&out)).await.unwrap();
    }

    fn reg(function_id: &str, source: &str, description: Option<&str>) -> RegisterRequest {
        RegisterRequest {
            function_id: function_id.to_string(),
            source: source.to_string(),
            description: description.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn registering_creates_the_namespace_runtime_and_the_function_answers() {
        let (m, _fake) = manager(NodeEngineConfig {
            max_runtimes: 4,
            ..Default::default()
        });
        let out = m
            .register(RegisterRequest {
                function_id: "app::double".into(),
                source: "export function handler(p) { return p.n * 2 }".into(),
                description: Some("doubles n".into()),
            })
            .await
            .unwrap();
        assert_eq!(out.function_id, "app::double");
        assert_eq!(out.namespace, "app::");
        assert_eq!(m.live_runtime_count(), 1);
    }

    #[tokio::test]
    async fn a_second_registration_reuses_the_namespace_runtime() {
        let (m, _fake) = manager(NodeEngineConfig {
            max_runtimes: 4,
            ..Default::default()
        });
        m.register(RegisterRequest {
            function_id: "app::a".into(),
            source: "export function handler() { return 1 }".into(),
            description: None,
        })
        .await
        .unwrap();
        m.register(RegisterRequest {
            function_id: "app::b".into(),
            source: "export function handler() { return 2 }".into(),
            description: None,
        })
        .await
        .unwrap();
        assert_eq!(
            m.live_runtime_count(),
            1,
            "the second registration created a runtime"
        );
    }

    /// Claim before you create. The sibling worker shipped the reverse and
    /// booted a whole VM for a registration that was already doomed.
    #[tokio::test]
    async fn a_doomed_registration_creates_no_runtime() {
        let (m, _fake) = manager(NodeEngineConfig {
            max_runtimes: 4,
            ..Default::default()
        });
        // node-engine's own ids seed the registry, so this one is always taken.
        let err = m
            .register(RegisterRequest {
                function_id: "node-engine::run".into(),
                source: "export function handler() { return 1 }".into(),
                description: None,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), "node-engine::id_taken");
        assert_eq!(
            m.live_runtime_count(),
            0,
            "a doomed registration created a runtime"
        );
    }

    /// Two functions in one namespace, both actually invokable through the
    /// engine — `registering_creates_the_namespace_runtime_and_the_function_
    /// answers` proves the response shape; this proves the handler that lands
    /// on the bus is the right one, for both the first and a later id sharing
    /// its runtime.
    #[tokio::test]
    async fn registered_functions_are_invokable_and_share_one_runtime() {
        let (m, fake) = manager(NodeEngineConfig::default());
        m.register(reg(
            "app::a",
            "export function handler(p) { return p.n * 2 }",
            None,
        ))
        .await
        .unwrap();
        m.register(reg(
            "app::b",
            "export function handler(p) { return p.n + 1 }",
            None,
        ))
        .await
        .unwrap();
        assert_eq!(
            fake.invoke("app::a", json!({ "n": 21 })).await,
            Ok(json!(42))
        );
        assert_eq!(fake.invoke("app::b", json!({ "n": 1 })).await, Ok(json!(2)));
        assert_eq!(fake.registered_ids().len(), 2);
        assert_eq!(m.live_runtime_count(), 1);
    }

    /// Redeploying a function is just calling `register_function` again with
    /// the same `function_id` — there is no `runtime_id` to reuse anymore, so
    /// this is the only way to update one. The pre-claim in `register` must
    /// not mistake this for a conflict with itself.
    #[tokio::test]
    async fn re_registering_the_same_function_id_redeploys_the_handler() {
        let (m, fake) = manager(NodeEngineConfig::default());
        m.register(reg(
            "app::x",
            "export function handler() { return 1 }",
            None,
        ))
        .await
        .expect("first registration");
        let out = m
            .register(reg(
                "app::x",
                "export function handler() { return 2 }",
                None,
            ))
            .await
            .expect("redeploy");
        assert_eq!(out.function_id, "app::x");
        assert_eq!(fake.invoke("app::x", json!({})).await, Ok(json!(2)));
        assert_eq!(
            m.live_runtime_count(),
            1,
            "redeploy must not spawn a runtime"
        );
    }

    /// A definition failure's stack is entirely prelude internals — `Array.map`,
    /// the generated eval wrapper, `toHandler` itself. It buried the one line
    /// that told the caller what to do.
    #[tokio::test]
    async fn a_definition_error_carries_no_internal_stack() {
        let (m, _) = manager(NodeEngineConfig::default());
        let err = m.register(reg("app::a", "42", None)).await.unwrap_err();

        let msg = err.to_string();
        for frame in ["code-runner:prelude", "code-runner:eval", "Array.map", "\n"] {
            assert!(
                !msg.contains(frame),
                "definition error leaked {frame:?}: {msg}"
            );
        }
    }

    /// The old bare-expression handler (`"(p) => p.n * 2"`) is the ability
    /// this task retires: `source` must DEFINE `handler`, and with nothing
    /// bound to that name the generated wrapper's own `typeof handler !==
    /// "function"` check throws a `TypeError` rather than silently
    /// registering the expression's value.
    #[tokio::test]
    async fn a_bare_expression_handler_is_refused() {
        let (m, fake) = manager(NodeEngineConfig::default());
        let err = m
            .register(reg("app::a", "(p) => p.n * 2", None))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "node-engine::eval_failed");
        assert!(fake.registered_ids().is_empty());
    }

    /// The other bare-expression shape: `"payload.n * 2"` evaluates `payload`
    /// at definition time and dies with a ReferenceError before the
    /// "must define handler" check is ever reached, so naming the contract is
    /// on that error arm specifically.
    ///
    /// The second half is the real point. `toHandler`'s hint used to
    /// interpolate `src`, which since `wrap_register` became its only caller
    /// is the GENERATED WRAPPER — and that wrapper contains the literal text
    /// `source must define handler(payload)`. So an assertion that the
    /// message "names the contract" passed on a ~1,400-character dump of
    /// machine-written code rather than on any message (the E2E case
    /// `a bare expression that never defines handler is refused` was doing
    /// exactly that). Asserting the wrapper is ABSENT is what keeps this
    /// honest.
    #[tokio::test]
    async fn a_definition_time_reference_error_names_the_contract_without_dumping_the_wrapper() {
        let (m, _) = manager(NodeEngineConfig::default());
        let err = m
            .register(reg("app::a", "payload.n * 2", None))
            .await
            .unwrap_err();

        assert_eq!(err.code(), "node-engine::eval_failed");
        let msg = err.to_string();
        assert!(
            msg.contains("source must define handler(payload)"),
            "must name the contract: {msg}"
        );
        for wrapper_token in ["var handler", "typeof handler===", "__def", "function(){"] {
            assert!(
                !msg.contains(wrapper_token),
                "the generated wrapper leaked into the message via {wrapper_token:?}: {msg}"
            );
        }
    }

    /// A GENUINE syntax error (unlike `"(p) => p.n * 2"` above, which parses
    /// fine and just binds nothing) fails `toHandler`'s internal
    /// expression-then-body fallback on BOTH attempts. `toHandler`'s only
    /// caller is `wrap_register` now, so that fallback exists solely to serve
    /// THIS contract — the message it produces must recommend defining
    /// `handler(payload)`, not the bare-expression/function-body forms this
    /// task retired. Telling an agent to fix malformed JS by writing exactly
    /// what the worker no longer accepts is worse than no hint at all.
    #[tokio::test]
    async fn a_syntax_error_in_source_recommends_the_current_contract_not_the_retired_forms() {
        let (m, _) = manager(NodeEngineConfig::default());
        let err = m
            .register(reg("app::a", "function(", None))
            .await
            .unwrap_err();

        assert_eq!(err.code(), "node-engine::eval_failed");
        let msg = err.to_string();
        assert!(
            msg.contains("source must define handler(payload)"),
            "must recommend the current contract: {msg}"
        );
        assert!(
            !msg.contains("(payload) => payload.n * 2"),
            "must not recommend the retired bare-expression form: {msg}"
        );
        assert!(
            !msg.contains("a function body containing"),
            "must not recommend the retired function-body-only form: {msg}"
        );
    }

    /// CROSS-TASK: the failure arm of `register` released the id's
    /// worker-global claim whenever no FUNCTION held it — but `release_ids`
    /// is kind-agnostic, so a live TRIGGER TYPE at the same id had its claim
    /// freed out from under it. Reachable in one call: `source` executes
    /// arbitrary statements in the namespace runtime, so it can publish a
    /// trigger type and THEN fail to define `handler`. The freed id let a
    /// second runtime in the same namespace claim it and silently clobber the
    /// first's trigger-type handler (`register_trigger_type` overwrites
    /// rather than panicking, so this is a hijack, not the process abort the
    /// function path would have been).
    #[tokio::test]
    async fn a_doomed_registration_does_not_release_a_live_trigger_types_claim() {
        let (m, fake) = manager(NodeEngineConfig::default());

        // The seed is what makes this test discriminate, and it is not
        // incidental setup. `release_ids` is OWNER-scoped (ids.rs), and the
        // failure arm passes `claim_owner`. On a FIRST registration into a
        // namespace there is no runtime yet, so `claim_owner` is the
        // `"app::"` placeholder — which `register` already released at the
        // hand-off before `define_handler` runs. The release then no-ops on
        // the owner check no matter what the predicate says, and the bug is
        // unreachable. Only on the REUSE path is `claim_owner` the live
        // runtime_id that actually owns the trigger type's claim.
        m.register(reg(
            "app::seed",
            "export function handler(p) { return 1 }",
            None,
        ))
        .await
        .expect("the seed registration creates the namespace runtime");

        let err = m
            .register(reg(
                "app::x",
                "iii.registerTriggerType({ id: 'app::x' }, \
                 { registerTrigger() {}, unregisterTrigger() {} });",
                None,
            ))
            .await
            .expect_err("the source defines no handler, so the registration must fail");
        assert_eq!(err.code(), "node-engine::eval_failed");

        // The side effect landed even though the registration failed: the
        // trigger type is live on the bus and must stay claimed.
        assert_eq!(fake.registered_trigger_types(), vec!["app::x".to_string()]);
        assert!(
            !m.ids.claim("app::x", "some-other-runtime"),
            "a live trigger type's claim was released by an unrelated failed registration"
        );
    }

    /// `export` is stripped only as the source's FIRST token, so a leading
    /// statement ahead of it leaves a bare `export` inside the generated
    /// wrapper and the whole thing fails to parse. That is fine as a
    /// limitation; recommending the exact form the author already wrote is
    /// not — the message has to name the actual rule.
    #[tokio::test]
    async fn an_export_after_a_leading_statement_says_export_must_come_first() {
        let (m, _) = manager(NodeEngineConfig::default());
        let err = m
            .register(reg(
                "app::a",
                "const helper = 2;\nexport function handler(p) { return helper }",
                None,
            ))
            .await
            .unwrap_err();

        assert_eq!(err.code(), "node-engine::eval_failed");
        let msg = err.to_string();
        assert!(
            msg.contains("only recognised as the first token"),
            "must state the actual rule rather than recommending what was written: {msg}"
        );
    }

    /// `toHandler` resolves the wrapper via `new Function`, which runs in
    /// sloppy mode: an undeclared `handler = ...` assignment — exactly what
    /// "source must define handler(payload)" invites just as much as
    /// `function handler(p) {}` — creates an IMPLICIT GLOBAL in the shared
    /// namespace isolate rather than a local. Without a local `handler`
    /// already shadowing it, a LATER registration whose source binds nothing
    /// at all would resolve `typeof handler` up the scope chain to a
    /// PREVIOUS registration's leaked global and silently register that
    /// stale handler under the new id.
    #[tokio::test]
    async fn a_source_that_binds_nothing_cannot_see_a_previous_registrations_implicit_global() {
        let (m, fake) = manager(NodeEngineConfig::default());
        m.register(reg("app::a", "handler = (p) => \"TENANT-A-SECRET\"", None))
            .await
            .expect("registers via a bare, undeclared assignment");

        let err = m
            .register(reg("app::b", "var somethingElse = 1;", None))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "node-engine::eval_failed");
        assert!(
            fake.invoke("app::b", json!({})).await.is_err(),
            "app::b must never have registered another tenant's handler"
        );
    }

    /// `const`/`let handler = ...` are exactly as valid a way to "define
    /// handler(payload)" as `function handler(p) {}` — `export const
    /// handler = async (p) => ...` is the canonical ESM idiom. An earlier
    /// version of the generated wrapper put `var handler;` in the SAME
    /// scope as the source, which collides with a `let`/`const` of the same
    /// name (`SyntaxError: Identifier 'handler' has already been
    /// declared`) and refused both forms outright.
    #[tokio::test]
    async fn const_and_let_handler_forms_register_and_invoke() {
        let (m, fake) = manager(NodeEngineConfig::default());
        m.register(reg("app::c", "const handler = (p) => p.n + 1;", None))
            .await
            .expect("const handler registers");
        m.register(reg("app::l", "let handler = (p) => p.n + 2;", None))
            .await
            .expect("let handler registers");
        assert_eq!(fake.invoke("app::c", json!({ "n": 1 })).await, Ok(json!(2)));
        assert_eq!(fake.invoke("app::l", json!({ "n": 1 })).await, Ok(json!(3)));
    }

    /// `class handler {}` is, by JS semantics, `typeof handler ===
    /// "function"`, so it passes the wrapper's own check and REGISTERS — and
    /// then fails on every single call, because a class constructor has no
    /// `[[Call]]`. This test pins that asymmetry end to end; it used to
    /// assert the registration alone, which is why the guidance and the
    /// README could go on recommending `class handler {}` as a working form
    /// with nothing to contradict them.
    ///
    /// The accept is deliberate, not an oversight to fix here: refusing it at
    /// definition time needs source-text sniffing
    /// (`Function.prototype.toString` starts with `class`), which is a
    /// heuristic that a bound or transpiled class slips past anyway, so it
    /// would buy a guarantee it cannot actually keep. The invoke-time
    /// `TypeError` names the exact problem. The docs no longer recommend the
    /// form; this test is what keeps the real behaviour honest.
    #[tokio::test]
    async fn a_class_named_handler_registers_but_can_never_be_invoked() {
        let (m, fake) = manager(NodeEngineConfig::default());
        m.register(reg("app::k", "class handler {}", None))
            .await
            .expect("class handler registers");

        let err = fake
            .invoke("app::k", json!({}))
            .await
            .expect_err("a class constructor cannot be invoked without 'new'");
        assert!(
            err.to_string().contains("without 'new'"),
            "expected the class-constructor TypeError, got: {err}"
        );
    }

    /// A tenant's own `"use strict"` directive must survive the wrapper.
    /// The earlier single-scope version put `var handler;` ahead of the
    /// source, which is not a valid directive-prologue position (a
    /// directive must be the first statement(s) of a function body), so the
    /// directive was silently dropped: a typo that should throw
    /// `ReferenceError` at invoke time instead created an implicit global
    /// and ran to completion. Giving the source its own inner function
    /// scope (nothing precedes it there) restores prologue position.
    #[tokio::test]
    async fn a_sources_own_use_strict_directive_still_applies() {
        let (m, fake) = manager(NodeEngineConfig::default());
        m.register(reg(
            "app::strict",
            "\"use strict\";\nfunction handler(p) { typoedVar = 1; return 1; }",
            None,
        ))
        .await
        .expect("registers");
        assert!(
            fake.invoke("app::strict", json!({})).await.is_err(),
            "the source's own strict mode should have caught the undeclared assignment"
        );
    }

    /// The description crosses Rust → generated JS → the prelude → the op →
    /// `Engine::register`, and only the far end is observable. Asserting it
    /// there is the only way to know the whole chain carried it: every earlier
    /// link can be intact while the value is still dropped at the next one.
    #[tokio::test]
    async fn a_supplied_description_reaches_the_engine() {
        let (m, fake) = manager(NodeEngineConfig::default());
        m.register(reg(
            "app::documented",
            "export function handler() { return 1 }",
            Some("Adds one. Payload: {}"),
        ))
        .await
        .expect("registers");
        m.register(reg(
            "app::bare",
            "export function handler() { return 2 }",
            None,
        ))
        .await
        .expect("registers");

        assert_eq!(
            fake.registered_descriptions(),
            vec![
                (
                    "app::documented".to_string(),
                    Some("Adds one. Payload: {}".to_string())
                ),
                // Omitted stays None here: `IIIEngine` substitutes
                // DEFAULT_DYNAMIC_DESC, so the fake sees the caller's intent.
                ("app::bare".to_string(), None),
            ]
        );
    }

    /// Quotes and newlines in a description are interpolated into generated
    /// JavaScript, same as the id and the source. If they escaped their
    /// literal the script would not parse — so a clean round-trip is also the
    /// escaping proof.
    #[tokio::test]
    async fn a_description_with_javascript_metacharacters_round_trips() {
        let (m, fake) = manager(NodeEngineConfig::default());
        let nasty = "quote \" backslash \\ newline \n `backtick` ${expr} */";
        m.register(reg(
            "app::x",
            "export function handler() { return 1 }",
            Some(nasty),
        ))
        .await
        .expect("registers");

        assert_eq!(
            fake.registered_descriptions(),
            vec![("app::x".to_string(), Some(nasty.to_string()))]
        );
    }

    #[tokio::test]
    async fn register_rejects_an_over_long_description() {
        let (m, fake) = manager(NodeEngineConfig::default());
        let long = "x".repeat(MAX_DESCRIPTION_BYTES + 1);
        let err = m
            .register(reg(
                "app::x",
                "export function handler() { return 1 }",
                Some(&long),
            ))
            .await
            .unwrap_err();

        assert_eq!(err.code(), "node-engine::invalid_request");
        assert!(fake.registered_ids().is_empty());
        assert_eq!(
            m.live_runtime_count(),
            0,
            "rejected before a runtime is created"
        );
    }

    #[tokio::test]
    async fn teardown_by_namespace_disposes_the_registration_runtime() {
        let (m, _fake) = manager(NodeEngineConfig {
            max_runtimes: 4,
            ..Default::default()
        });
        m.register(RegisterRequest {
            function_id: "app::hello".into(),
            source: "export function handler() { return 1 }".into(),
            description: None,
        })
        .await
        .unwrap();
        assert_eq!(m.live_runtime_count(), 1);

        let out = m
            .teardown(TeardownRequest {
                runtime_id: None,
                namespace: Some("app::".into()),
            })
            .await
            .unwrap();
        assert!(out.unregistered.contains(&"app::hello".to_string()));
        assert_eq!(m.live_runtime_count(), 0);
    }

    #[tokio::test]
    async fn teardown_requires_exactly_one_selector() {
        let (m, _fake) = manager(NodeEngineConfig {
            max_runtimes: 4,
            ..Default::default()
        });
        for (rid, ns) in [
            (None, None),
            (Some("rt-x".to_string()), Some("app::".to_string())),
        ] {
            let err = m
                .teardown(TeardownRequest {
                    runtime_id: rid,
                    namespace: ns,
                })
                .await
                .unwrap_err();
            assert_eq!(err.code(), "node-engine::invalid_request");
        }
    }

    #[tokio::test]
    async fn teardown_of_an_unknown_namespace_is_not_found() {
        let (m, _fake) = manager(NodeEngineConfig {
            max_runtimes: 4,
            ..Default::default()
        });
        let err = m
            .teardown(TeardownRequest {
                runtime_id: None,
                namespace: Some("nobody::".into()),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), "node-engine::runtime_not_found");
    }

    /// Unknown namespace must be reported not-found, never a runtime minted
    /// just to destroy: `teardown` uses `live_namespace_runtime` (no create),
    /// not `namespace_runtime`. A regression to the creating lookup would
    /// still return `runtime_not_found` on the SECOND call (nothing was ever
    /// registered into it) but would leave a live, unreachable runtime behind
    /// — this asserts the slot was never spent at all.
    #[tokio::test]
    async fn teardown_of_an_unknown_namespace_creates_no_runtime() {
        let (m, _fake) = manager(NodeEngineConfig {
            max_runtimes: 4,
            ..Default::default()
        });
        let _ = m
            .teardown(TeardownRequest {
                runtime_id: None,
                namespace: Some("nobody::".into()),
            })
            .await
            .unwrap_err();
        assert_eq!(
            m.live_runtime_count(),
            0,
            "teardown of an unregistered namespace must not mint a runtime"
        );
    }

    /// A behavioural check on the teardown → re-register sequence, NOT a
    /// guard on `destroy`'s `namespaces` prune: `namespace_runtime`'s create
    /// path unconditionally `.insert()`s on every fresh creation (overwriting
    /// any stale entry regardless of whether it was pruned), and
    /// `live_namespace_runtime` independently re-checks the resolved id
    /// against `runtimes` before trusting it — so this sequence passes
    /// whether or not the prune runs at all. See
    /// `teardown_by_namespace_prunes_the_dead_map_entry` below for the test
    /// that actually falsifies on a disabled prune.
    #[tokio::test]
    async fn a_registration_after_namespace_teardown_gets_a_fresh_runtime() {
        let (m, _fake) = manager(NodeEngineConfig {
            max_runtimes: 4,
            ..Default::default()
        });
        m.register(reg(
            "app::hello",
            "export function handler() { return 1 }",
            None,
        ))
        .await
        .unwrap();
        assert_eq!(m.live_runtime_count(), 1);

        m.teardown(TeardownRequest {
            runtime_id: None,
            namespace: Some("app::".into()),
        })
        .await
        .unwrap();
        assert_eq!(m.live_runtime_count(), 0);

        // Documents current behaviour on re-registration; does not by
        // itself prove `namespaces` was pruned (see the comment on this
        // test above).
        let out = m
            .register(reg(
                "app::hello",
                "export function handler() { return 2 }",
                None,
            ))
            .await
            .unwrap();
        assert_eq!(out.namespace, "app::");
        assert_eq!(m.live_runtime_count(), 1, "namespace runtime was not fresh");

        // And it really is the new registration answering, not a ghost of
        // the old one.
        m.teardown(TeardownRequest {
            runtime_id: None,
            namespace: Some("app::".into()),
        })
        .await
        .unwrap();
        assert_eq!(m.live_runtime_count(), 0);
    }

    /// The property `destroy`'s prune (`self.namespaces.lock().unwrap()
    /// .retain(|_, id| id != runtime_id)`) actually guards: `namespaces`
    /// must not grow one dead entry per teardown that is never followed by a
    /// re-registration into the same namespace. Reads the map directly
    /// rather than going through any lookup, because both
    /// `namespace_runtime` (unconditional `.insert()` on every create) and
    /// `live_namespace_runtime` (re-checks the id against `runtimes` before
    /// trusting it) tolerate a stale entry just fine — a test that
    /// re-registers and asserts on behaviour, like the one above, cannot
    /// fail even with the prune line deleted. Three distinct namespaces, so
    /// a single surviving stale entry is unmistakable rather than lost in
    /// rounding.
    ///
    /// Mutation-tested: commenting out the `retain` call in `destroy` turns
    /// this test red (`left: 3, right: 0`) while the rest of the suite,
    /// including the test above, stays green — see task-8-report.md.
    #[tokio::test]
    async fn teardown_by_namespace_prunes_the_dead_map_entry() {
        let (m, _fake) = manager(NodeEngineConfig {
            max_runtimes: 4,
            ..Default::default()
        });
        for ns in ["a", "b", "c"] {
            m.register(reg(
                &format!("{ns}::hello"),
                "export function handler() { return 1 }",
                None,
            ))
            .await
            .unwrap();
            m.teardown(TeardownRequest {
                runtime_id: None,
                namespace: Some(format!("{ns}::")),
            })
            .await
            .unwrap();
        }
        assert_eq!(
            m.namespaces.lock().unwrap().len(),
            0,
            "a torn-down namespace with nothing re-registered into it leaked a map entry"
        );
    }

    /// Normalizing (unchanged: it repairs a namespace string missing its
    /// trailing `::`, and never WIDENS a prefix) is exercised directly here —
    /// `register` no longer has an independent `namespace` field to feed it a
    /// mismatched value, since the namespace IS the id's own first segment.
    #[tokio::test]
    async fn normalize_namespace_repairs_the_delimiter_and_never_widens() {
        let norm = |ns: &str| normalize_namespace(ns, &[]);
        assert_eq!(norm("app").unwrap(), "app::");
        assert_eq!(norm("app:").unwrap(), "app::");
        assert_eq!(norm("app::").unwrap(), "app::");
        assert_eq!(norm("app:::").unwrap(), "app::");
        // Nothing to build a prefix from.
        for empty in ["", ":", "::"] {
            let why = norm(empty).unwrap_err();
            assert!(why.contains("names no prefix"), "ns {empty:?}: {why}");
        }
        // One segment only: `a::b` names an identity the bus cannot represent.
        // The stem is a WORKER NAME, so `validate_worker_name`'s rule applies
        // whole — including its byte limit, which is bytes and not chars.
        for bad in [
            "a::b",
            "my app",
            "MyApp",
            &"a".repeat(65),
            // 33 chars, 66 bytes: a char-counted limit would let this past.
            &"é".repeat(33),
        ] {
            let why = norm(bad).unwrap_err();
            assert!(
                why.contains("is not one worker name") && why.contains("my-app::"),
                "ns {bad:?}: {why}"
            );
        }
        assert!(norm(&"é".repeat(33)).unwrap_err().contains("66 bytes"));
        let max = "a".repeat(MAX_WORKER_NAME_BYTES);
        assert_eq!(norm(&max).unwrap(), format!("{max}::"));
    }

    /// A namespace's first segment IS a worker name on the bus, so the id's
    /// own prefix has to be one — `validate_worker_name`'s rule, applied to
    /// whatever `function_id` puts in front of the first `::`. Every one of
    /// these is refused before an isolate exists, which is why none of them
    /// leaves a runtime behind.
    #[tokio::test]
    async fn a_namespace_that_is_not_one_worker_name_is_refused() {
        let (m, fake) = manager(NodeEngineConfig::default());
        let long = "a".repeat(65);
        // 33 chars, 66 bytes: a char-counted limit would wave this through.
        let multibyte = "é".repeat(33);
        // Not `"a::b"`: `function_id.split_once("::")` always stops at the
        // FIRST `::`, so a derived namespace can never itself contain one —
        // `"a::b::x"` derives namespace `"a::"`, one clean segment, and
        // nests `b::x` below it exactly as `myapp::v2::save` is meant to.
        for stem in ["my app", "MyApp", long.as_str(), multibyte.as_str()] {
            let id = format!("{stem}::x");
            let err = m
                .register(reg(&id, "export function handler() {}", None))
                .await
                .unwrap_err();
            assert_eq!(err.code(), "node-engine::invalid_request", "stem {stem:?}");
            let msg = err.to_string();
            assert!(
                msg.contains("my-app::"),
                "must say what to pass instead: {msg}"
            );
            assert_eq!(m.live_runtime_count(), 0, "stem {stem:?} leaked a runtime");
            assert!(fake.registered_ids().is_empty(), "stem {stem:?}");
        }
    }

    /// An id with no `::` at all, or nothing before it, cannot name a
    /// namespace and is refused with a concrete example rather than silently
    /// placed in a default.
    #[tokio::test]
    async fn a_function_id_with_no_namespace_is_refused() {
        let (m, _) = manager(NodeEngineConfig::default());
        for id in ["bare-id", "::save"] {
            let err = m
                .register(reg(id, "export function handler() {}", None))
                .await
                .unwrap_err();
            assert_eq!(err.code(), "node-engine::invalid_request", "id {id:?}");
            assert!(
                err.to_string().contains("my-app::greet"),
                "must show a working example: {err}"
            );
            assert_eq!(m.live_runtime_count(), 0, "id {id:?} leaked a runtime");
        }
    }

    #[tokio::test]
    async fn register_rejects_once_the_runtime_holds_the_registration_cap() {
        let (m, _) = manager(NodeEngineConfig::default());
        for i in 0..crate::ops::MAX_REGISTRATIONS_PER_RUNTIME {
            m.register(reg(
                &format!("app::f{i}"),
                "export function handler() { return 1 }",
                None,
            ))
            .await
            .unwrap_or_else(|e| panic!("registration {i} failed: {e}"));
        }
        assert_eq!(m.live_runtime_count(), 1, "one namespace, one runtime");

        let err = m
            .register(reg(
                "app::over",
                "export function handler() { return 1 }",
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "node-engine::eval_failed");
        assert!(
            err.to_string()
                .contains(&crate::ops::MAX_REGISTRATIONS_PER_RUNTIME.to_string()),
            "must name the cap: {err}"
        );
        assert_eq!(m.live_runtime_count(), 1, "the namespace runtime survives");
    }

    /// Evaluated code that wants to register has to satisfy a prefix it had no
    /// way to read. Guessing costs a `namespace_denied` after the work is done.
    #[tokio::test]
    async fn evaluated_code_can_read_its_own_namespace() {
        let (m, _) = manager(NodeEngineConfig::default());

        let res = m
            .run(RunRequest {
                code: "return iii.namespace".into(),
                runtime_id: None,
                namespace: Some("app::".into()),
                keep: false,
                timeout_ms: None,
            })
            .await
            .expect("evaluates");
        assert_eq!(res.result, serde_json::json!("app::"));

        // The default runtime namespace is equally unguessable from inside, so
        // it has to be readable too. `keep: true`: its id is reused below.
        let derived = m
            .run(RunRequest {
                code: "return iii.namespace".into(),
                runtime_id: None,
                namespace: None,
                keep: true,
                timeout_ms: None,
            })
            .await
            .expect("evaluates");
        let ns = derived.result.as_str().expect("a string namespace");
        assert!(
            ns.starts_with("code-runner::rt-") && ns.ends_with("::"),
            "default namespace should be readable: {ns}"
        );

        // And it is the value the op actually enforces: registering under it
        // succeeds where a guess would not.
        let ok = m
            .run(RunRequest {
                code: "iii.registerFunction(iii.namespace + 'x', () => 1); return 'ok'".into(),
                runtime_id: derived.runtime_id.clone(),
                namespace: None,
                keep: false,
                timeout_ms: None,
            })
            .await
            .expect("registers under the published namespace");
        assert_eq!(ok.registered, vec![format!("{ns}x")]);
    }

    /// `heap_limits` bounds the object heap only. An `ArrayBuffer` is backed
    /// by V8's array-buffer allocator, accounted separately, so before the cap
    /// a runtime could hold hundreds of megabytes resident while its object
    /// heap stayed healthy — one tenant exhausting the host for all of them.
    #[tokio::test]
    async fn off_heap_memory_is_capped_independently_of_the_object_heap() {
        let cfg = NodeEngineConfig {
            external_mb: 8,
            ..NodeEngineConfig::default()
        };
        let (m, _) = manager(cfg);

        // Under the cap: allowed, and the bytes are real. `keep: true`: its
        // id is reused below to push the same runtime over the cap.
        let ok = m
            .run(RunRequest {
                code: "const b = new ArrayBuffer(4*1024*1024); return b.byteLength".into(),
                runtime_id: None,
                namespace: None,
                keep: true,
                timeout_ms: None,
            })
            .await
            .expect("4 MiB is under the 8 MiB cap");
        assert_eq!(ok.result, serde_json::json!(4 * 1024 * 1024));

        // Over the cap: refused. V8 treats a null from the array-buffer
        // allocator as unrecoverable rather than throwing a catchable
        // RangeError, so this surfaces as `oom` and the isolate dies — the
        // same outcome as exceeding `heap_mb`, which is the point: the tenant
        // that asked for too much loses its runtime and nobody else does.
        let err = m
            .run(RunRequest {
                code: "return new ArrayBuffer(64*1024*1024).byteLength".into(),
                runtime_id: ok.runtime_id.clone(),
                namespace: None,
                keep: false,
                timeout_ms: None,
            })
            .await
            .expect_err("64 MiB must be refused");
        assert_eq!(err.code(), "node-engine::oom");

        // Reaped like any OOM, so the slot is not stranded, and the WORKER is
        // unharmed — a fresh runtime still works. `keep: true` so the
        // `teardown` below has something to address.
        assert_eq!(
            m.live_runtime_count(),
            0,
            "a killed runtime must not hold its slot"
        );
        let fresh = m
            .run(RunRequest {
                code: "return 'alive'".into(),
                runtime_id: None,
                namespace: None,
                keep: true,
                timeout_ms: None,
            })
            .await
            .expect("the worker survives a tenant blowing its own cap");
        assert_eq!(fresh.result, serde_json::json!("alive"));
        m.teardown_runtime(addr(&fresh)).await.unwrap();
    }

    /// WebAssembly.Memory is backed by V8's own reservation, not the capped
    /// array-buffer allocator, so it was the one remaining way to take memory
    /// the cap cannot see. It is also not part of what this worker offers.
    #[tokio::test]
    async fn webassembly_is_not_reachable() {
        let (m, _) = manager(NodeEngineConfig::default());
        let res = m
            .run(RunRequest {
                code: "return [typeof WebAssembly, typeof globalThis.WebAssembly]".into(),
                runtime_id: None,
                namespace: None,
                keep: false,
                timeout_ms: None,
            })
            .await
            .expect("evaluates");
        assert_eq!(res.result, serde_json::json!(["undefined", "undefined"]));
    }

    /// `description` is published metadata — it is what the next caller reads
    /// out of `engine::functions::info`. The SDK carries it only at
    /// registration time, so a re-bind that kept the old registration
    /// reported success and left the catalog showing the old text. There is
    /// no `runtime_id` to reuse anymore, so "re-bind" is just calling
    /// `register` again with the same `function_id`.
    #[tokio::test]
    async fn re_registering_with_a_new_description_updates_the_published_one() {
        let (m, fake) = manager(NodeEngineConfig::default());

        m.register(reg(
            "app::x",
            "export function handler() { return 1 }",
            Some("ORIGINAL"),
        ))
        .await
        .expect("registers");

        m.register(reg(
            "app::x",
            "export function handler() { return 2 }",
            Some("CHANGED"),
        ))
        .await
        .expect("re-binds");

        assert_eq!(
            fake.registered_descriptions(),
            vec![("app::x".to_string(), Some("CHANGED".to_string()))],
            "the published description must follow the re-bind"
        );
        // Still exactly one live registration, and the handler swapped too.
        assert_eq!(fake.registered_ids(), vec!["app::x".to_string()]);
        assert_eq!(
            fake.invoke("app::x", serde_json::json!({})).await.unwrap(),
            2
        );

        // Omitting the description on a re-bind leaves the published one alone
        // rather than blanking it.
        m.register(reg(
            "app::x",
            "export function handler() { return 3 }",
            None,
        ))
        .await
        .expect("re-binds again");
        assert_eq!(
            fake.registered_descriptions(),
            vec![("app::x".to_string(), Some("CHANGED".to_string()))],
            "an omitted description must not erase the published one"
        );
        assert_eq!(m.live_runtime_count(), 1);
    }

    /// The schema promises `result` is null for a value JSON cannot
    /// represent. `JSON.stringify` has TWO failure modes and only one throws:
    /// a function, a symbol, or a `toJSON` returning undefined is DROPPED, so
    /// stringifying `{ok: v}` in one step produced `{}` — an envelope with
    /// neither key, which surfaced to the caller as `eval_failed`.
    #[tokio::test]
    async fn unrepresentable_return_values_come_back_as_null_not_an_error() {
        let (m, _) = manager(NodeEngineConfig::default());
        for code in [
            "return function foo(){}",
            "return Symbol('s')",
            "return {toJSON(){return undefined}}",
            "const a={};a.self=a;return a",
            "return 1n",
            "return undefined",
            "return [function(){}, Symbol('x'), 1]",
        ] {
            let res = m
                .run(RunRequest {
                    code: code.into(),
                    runtime_id: None,
                    namespace: None,
                    keep: false,
                    timeout_ms: None,
                })
                .await
                .unwrap_or_else(|e| panic!("{code:?} must not error: {e}"));
            assert!(
                res.result.is_null() || res.result.is_array(),
                "{code:?} gave {:?}",
                res.result
            );
        }
    }

    /// `settle`'s return value IS the envelope Rust parses, so a global it
    /// resolves at call time is a way to forge one. Overwriting
    /// `JSON.stringify` reported a THROWN error as a successful result.
    ///
    /// Each case names the ONE pre-captured primitive in `prelude.js` it
    /// guards, and each was checked by reverting that capture to the global
    /// and confirming this test then fails. That check found two of these
    /// cases had gone dead: cases 2 and 3 hand `settle` a FAKE thenable, and
    /// since `thenPromise` is `Promise.prototype.then` pre-bound, a fake
    /// thenable now dies on the real `then` whatever `resolvePromise` is —
    /// so they passed with `resolvePromise` reverted. Case 4 is the one that
    /// actually exercises it: a `Promise.resolve` replacement that returns a
    /// REAL promise, which the real `then` is happy to consume.
    #[tokio::test]
    async fn tenant_code_cannot_forge_the_result_envelope() {
        let (m, _) = manager(NodeEngineConfig::default());
        for forge in [
            // 1. `stringify`, ok arm.
            r#"JSON.stringify = () => '{"ok":"FORGED"}'; throw new Error("real failure");"#,
            // 2. `thenPromise`, via a fake thenable off a replaced Promise.
            r#"globalThis.Promise = { resolve: () => ({ then: () => '{"ok":"FORGED"}' }) }; throw new Error("real failure");"#,
            // 3. `thenPromise`, replaced directly on the prototype.
            r#"Promise.prototype.then = () => '{"ok":"FORGED"}'; throw new Error("real failure");"#,
            // 4. `resolvePromise`. The forged value rides a REAL promise, so
            //    the real `Promise.prototype.then` resolves it normally and
            //    only the pre-captured `Promise.resolve` stands between a
            //    throw and a reported success.
            r#"const R = Promise.resolve.bind(Promise); globalThis.Promise = { resolve: () => R({ pwned: "FORGED" }) }; throw new Error("real failure");"#,
        ] {
            let err = m
                .run(RunRequest {
                    code: forge.into(),
                    runtime_id: None,
                    namespace: None,
                    keep: false,
                    timeout_ms: None,
                })
                .await
                .expect_err("a throw must stay an error");
            assert_eq!(err.code(), "node-engine::eval_failed", "forge: {forge}");
            assert!(
                !err.to_string().contains("FORGED"),
                "forged value surfaced: {err}"
            );
            // The tenant's REAL error has to be what reaches the caller. This
            // is what pins `settle`'s error-arm `stringify`: reverting that
            // one capture to the global lets case 1's replacement rewrite the
            // `{"err":…}` payload, and while the template cannot be closed
            // into a success, the caller stops being told what actually
            // failed.
            assert!(
                err.to_string().contains("real failure"),
                "the tenant's own error must survive the forge: {err}"
            );
        }
    }

    /// A create-path eval that fails leaves a runtime nothing can address —
    /// the caller never got the id and the error does not carry one — so it
    /// would hold a slot until the idle sweep. A syntax error is the ordinary
    /// case while someone iterates, so this leaked one slot per typo.
    #[tokio::test]
    async fn a_failed_eval_that_created_its_runtime_does_not_strand_it() {
        let (m, _) = manager(NodeEngineConfig::default());

        for code in ["this is not javascript", "throw new Error('boom')"] {
            let err = m
                .run(RunRequest {
                    code: code.into(),
                    runtime_id: None,
                    namespace: None,
                    keep: false,
                    timeout_ms: None,
                })
                .await
                .unwrap_err();
            assert_eq!(err.code(), "node-engine::eval_failed");
            assert_eq!(m.live_runtime_count(), 0, "{code:?} stranded a runtime");
        }

        // The caller's OWN runtime is never reaped out from under them: they
        // hold the id and may still be using the isolate. `keep: true`: its
        // id is reused for the two calls below.
        let mine = m
            .run(RunRequest {
                code: "globalThis.keep = 1; return 'ok'".into(),
                runtime_id: None,
                namespace: None,
                keep: true,
                timeout_ms: None,
            })
            .await
            .expect("creates");
        let _ = m
            .run(RunRequest {
                code: "throw new Error('boom')".into(),
                runtime_id: mine.runtime_id.clone(),
                namespace: None,
                keep: false,
                timeout_ms: None,
            })
            .await
            .unwrap_err();
        assert_eq!(
            m.live_runtime_count(),
            1,
            "a caller-owned runtime must survive"
        );
        let still = m
            .run(RunRequest {
                code: "return globalThis.keep".into(),
                runtime_id: mine.runtime_id.clone(),
                namespace: None,
                keep: false,
                timeout_ms: None,
            })
            .await
            .expect("still alive");
        assert_eq!(still.result, serde_json::json!(1));
        m.teardown_runtime(addr(&mine)).await.unwrap();
    }

    #[tokio::test]
    async fn register_rejects_an_over_long_id() {
        let (m, _) = manager(NodeEngineConfig::default());
        let long = format!("app::{}", "x".repeat(600));
        let err = m
            .register(reg(&long, "export function handler() { return 1 }", None))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "node-engine::invalid_request");
        assert_eq!(m.live_runtime_count(), 0);
    }

    /// A bad handler is the likeliest caller error. Unlike the old per-call
    /// runtime, this one is the namespace's PERSISTENT runtime — a failed
    /// registration must not be the thing that tears it down out from under
    /// whatever else that namespace already holds.
    #[tokio::test]
    async fn a_failed_registration_leaves_the_namespace_runtime_alive() {
        let (m, _) = manager(NodeEngineConfig::default());
        let err = m.register(reg("app::bad", "42", None)).await.unwrap_err();
        assert_eq!(err.code(), "node-engine::eval_failed");
        assert_eq!(
            m.live_runtime_count(),
            1,
            "a failed registration must not tear down the namespace runtime"
        );
    }

    /// The headroom that matters is the Rust-heap claim map, not V8's
    /// `heap_limits` — so before the failure arm released them, every failed
    /// registration into an already-live namespace left a fresh claim behind,
    /// and a caller could loop forever growing it.
    #[tokio::test]
    async fn failed_registrations_on_a_live_namespace_do_not_grow_the_claim_map() {
        let (m, _) = manager(NodeEngineConfig::default());
        m.register(reg(
            "app::a",
            "export function handler() { return 1 }",
            None,
        ))
        .await
        .unwrap();
        let baseline = format!("{:?}", m.ids);

        for i in 0..3 {
            let id = format!("app::bad{i}");
            let err = m.register(reg(&id, "42", None)).await.unwrap_err();
            assert_eq!(err.code(), "node-engine::eval_failed");
        }

        assert_eq!(
            format!("{:?}", m.ids),
            baseline,
            "claims for ids that never reached the bus were not released"
        );
        // And the successful registration keeps its claim: it IS live on the
        // bus, so releasing it would reopen the duplicate-id abort.
        assert!(
            !m.ids.claim("app::a", "rt-other"),
            "released a claim for an id that is live on the bus"
        );
        assert_eq!(m.live_runtime_count(), 1);
    }

    /// The bug this guards: unconditionally releasing the claim whenever
    /// `define_handler` fails, without checking whether the id is already
    /// live on the bus from an EARLIER, successful call. An ordinary
    /// typo'd redeploy fails before `iii.registerFunction` ever runs, and on
    /// the reuse path the claim's owner IS the namespace runtime that still
    /// actively serves the id — so a release here is not a release of THIS
    /// call's own (never-claimed) attempt, it is freeing a live one.
    /// Checked two ways: the id must still be unclaimable by a third party,
    /// AND a second runtime's `iii.registerFunction` for the same id — the
    /// actual guard against `Engine::register`'s panic on a duplicate — must
    /// still be refused, not just the Rust-side registry.
    #[tokio::test]
    async fn a_failed_redeploy_does_not_release_the_live_claim() {
        let (m, fake) = manager(NodeEngineConfig::default());
        m.register(reg(
            "app::x",
            "export function handler() { return 1 }",
            None,
        ))
        .await
        .expect("first registration");

        let err = m.register(reg("app::x", "42", None)).await.unwrap_err();
        assert_eq!(err.code(), "node-engine::eval_failed");
        assert!(
            !m.ids.claim("app::x", "rt-someone-else"),
            "a failed redeploy released a claim that is still live on the bus"
        );
        assert_eq!(fake.registered_ids(), vec!["app::x".to_string()]);

        // A different runtime under the same namespace (an ad-hoc `run()`,
        // not the persistent one) must still be refused the id.
        let denied = m
            .run(RunRequest {
                code: "try { iii.registerFunction('app::x', () => 2); return 'registered' } \
                       catch (e) { return 'refused' }"
                    .into(),
                runtime_id: None,
                namespace: Some("app::".into()),
                keep: false,
                timeout_ms: None,
            })
            .await
            .expect("evaluates");
        assert_eq!(denied.result, json!("refused"));
        assert_eq!(
            fake.invoke("app::x", json!({})).await,
            Ok(json!(1)),
            "the original handler must still be the one that answers"
        );
    }

    /// `define_handler` routes through `run`, whose `RuntimeNotFound` /
    /// `RuntimeGone` messages quote the runtime_id verbatim (error.rs) — the
    /// right behaviour for `run`'s own caller, who supplied that id, but
    /// wrong here: `register_function`'s caller never receives the
    /// namespace runtime's id and must never learn it through an error
    /// either. Reachable if the runtime vanishes (idle sweep, a concurrent
    /// teardown) between `namespace_runtime` resolving it and `run`'s own
    /// re-check — reproduced directly by tearing it down and calling
    /// `define_handler` with the now-stale id, rather than racing the sweep.
    #[tokio::test]
    async fn a_vanished_namespace_runtime_does_not_leak_its_id_through_the_error() {
        let (m, _fake) = manager(NodeEngineConfig::default());
        let (runtime_id, runtime) = m.namespace_runtime("app::").unwrap();
        m.teardown_runtime(&runtime_id).await.unwrap();

        let err = m
            .define_handler(
                &runtime_id,
                &runtime,
                &reg("app::x", "export function handler() { return 1 }", None),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code(), "node-engine::runtime_not_found");
        assert!(
            !err.to_string().contains(&runtime_id),
            "the namespace runtime's id leaked into the error: {err}"
        );
        assert!(
            !err.to_string().contains("rt-"),
            "a runtime-id shape leaked into the error: {err}"
        );
    }

    #[tokio::test]
    async fn a_one_shot_run_disposes_its_runtime_and_returns_no_id() {
        let (m, _fake) = manager(NodeEngineConfig {
            max_runtimes: 4,
            ..Default::default()
        });
        let out = m
            .run(RunRequest {
                code: "return 1 + 1".into(),
                runtime_id: None,
                namespace: None,
                keep: false,
                timeout_ms: None,
            })
            .await
            .unwrap();
        assert_eq!(out.result, serde_json::json!(2));
        assert!(
            out.runtime_id.is_none(),
            "a one-shot run must not return an id"
        );
        assert_eq!(m.live_runtime_count(), 0, "the runtime was not disposed");
    }

    #[tokio::test]
    async fn keep_true_returns_an_addressable_runtime() {
        let (m, _fake) = manager(NodeEngineConfig {
            max_runtimes: 4,
            ..Default::default()
        });
        let out = m
            .run(RunRequest {
                code: "globalThis.x = 41; return 1".into(),
                runtime_id: None,
                namespace: None,
                keep: true,
                timeout_ms: None,
            })
            .await
            .unwrap();
        let id = out.runtime_id.expect("keep: true must return an id");
        assert_eq!(m.live_runtime_count(), 1);

        let again = m
            .run(RunRequest {
                code: "return globalThis.x + 1".into(),
                runtime_id: Some(id),
                namespace: None,
                keep: false,
                timeout_ms: None,
            })
            .await
            .unwrap();
        assert_eq!(
            again.result,
            serde_json::json!(42),
            "globals were not shared"
        );
        assert_eq!(
            m.live_runtime_count(),
            1,
            "a run into a caller-owned runtime must not dispose it"
        );
    }

    /// The failure path is where the sibling worker leaked: a runtime created
    /// by this call and then failed is unaddressable, so nothing can ever
    /// reclaim it but the idle sweep.
    #[tokio::test]
    async fn a_failing_one_shot_run_leaves_no_runtime_behind() {
        let (m, _fake) = manager(NodeEngineConfig {
            max_runtimes: 4,
            ..Default::default()
        });
        let err = m
            .run(RunRequest {
                code: "throw new Error('boom')".into(),
                runtime_id: None,
                namespace: None,
                keep: false,
                timeout_ms: None,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), "node-engine::eval_failed");
        assert_eq!(m.live_runtime_count(), 0);
    }

    #[tokio::test]
    async fn one_runtime_serves_a_namespace_across_calls() {
        let (m, _fake) = manager(NodeEngineConfig {
            max_runtimes: 4,
            ..Default::default()
        });
        let (first, _) = m.namespace_runtime("app::").unwrap();
        let (second, _) = m.namespace_runtime("app::").unwrap();
        assert_eq!(first, second, "a namespace must reuse its runtime");
        assert_eq!(m.live_runtime_count(), 1);

        let (other, _) = m.namespace_runtime("other::").unwrap();
        assert_ne!(other, first);
        assert_eq!(m.live_runtime_count(), 2);
    }

    #[tokio::test]
    async fn concurrent_first_calls_create_one_runtime_not_two() {
        let (m, _fake) = manager(NodeEngineConfig {
            max_runtimes: 4,
            ..Default::default()
        });
        let a = m.clone();
        let b = m.clone();
        let (ra, rb) = tokio::join!(
            tokio::task::spawn_blocking(move || a.namespace_runtime("app::").map(|(id, _)| id)),
            tokio::task::spawn_blocking(move || b.namespace_runtime("app::").map(|(id, _)| id)),
        );
        assert_eq!(ra.unwrap().unwrap(), rb.unwrap().unwrap());
        assert_eq!(
            m.live_runtime_count(),
            1,
            "the thundering herd created two runtimes"
        );
    }

    #[tokio::test]
    async fn tearing_down_a_namespace_runtime_lets_the_next_call_recreate_it() {
        let (m, _fake) = manager(NodeEngineConfig {
            max_runtimes: 4,
            ..Default::default()
        });
        let (first, _) = m.namespace_runtime("app::").unwrap();
        m.teardown_runtime(&first).await.unwrap();
        assert_eq!(m.live_runtime_count(), 0);
        let (second, _) = m.namespace_runtime("app::").unwrap();
        assert_ne!(second, first, "a stale map entry was handed back");
        assert_eq!(m.live_runtime_count(), 1);
    }

    /// `declared.rs` existed so `eject` could write handler sources to disk
    /// later. With eject gone, retaining tenant code buys nothing and keeps a
    /// secret alive in host memory for the life of the runtime.
    #[test]
    fn the_manager_type_retains_no_tenant_source() {
        let src = include_str!("manager.rs");
        // Built at runtime, not written as one literal: `include_str!` reads
        // this very file, so spelling the deleted type's name out whole,
        // right here, would make the assertion below match its own source and
        // fail forever, deleted or not.
        let needle = format!("{}{}", "Declared", "Function");
        assert!(
            !src.contains(&needle),
            "manager.rs still references {needle}"
        );
        assert!(
            !std::path::Path::new("src/declared.rs").exists(),
            "src/declared.rs still exists"
        );
    }
}
