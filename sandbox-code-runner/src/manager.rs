//! Ownership and lifecycle for sandbox-backed runtimes.
//!
//! Two kinds of runtime exist, distinguished by who addresses them:
//!
//! - A KEPT-RUN runtime: `sandbox-code-runner::run keep=true` (or a caller-supplied
//!   `runtime_id`) mints or reuses one, and the caller holds its
//!   `runtime_id` — the capability to run into or tear it down.
//! - A NAMESPACE runtime: `sandbox-code-runner::register_function` creates or reuses
//!   one per `(namespace, lang)`, entirely as an implementation detail — the
//!   caller never sees or manages its `runtime_id`, only its namespace.
//!
//! Both kinds share one `runtimes` map and the same per-runtime async mutex
//! discipline: the daemon REJECTS concurrent execs into one sandbox (S003)
//! rather than queueing them, so serialization is this module's job — the
//! mutex covers each whole write+exec sequence, giving the same
//! one-command-at-a-time semantics node-engine's runtimes have.
//!
//! Every runtime boots with outbound network and `III_URL` in its
//! environment: guest code's `iii` global is the real iii-sdk client,
//! connected straight to the engine over the sandbox gateway (the guest's
//! /etc/hosts maps `localhost` to it — see `guest_engine_url`). That is
//! also why EVERY run path boots through `sandbox::create` + the
//! guest-file plant rather than the daemon's one-call `sandbox::run`:
//! `sandbox::run` can neither enable networking nor set create-time env.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::config::CodeRunnerConfig;
use crate::engine::{Engine, UnregisterFn};
use crate::error::{
    classify_probe_error, classify_sandbox_error, CodeRunnerError, ProbeOutcome, SandboxFailure,
};
use crate::events::{Emitter, Event};
use crate::functions::list_runtimes::{ListRuntimesResponse, RuntimeSummary};
use crate::functions::register::{RegisterRequest, RegisterResponse};
use crate::functions::run::{RunRequest, RunResponse};
use crate::functions::teardown::{TeardownRequest, TeardownResponse};
use crate::runner::Lang;

/// Trigger timeout for `sandbox::create` — a cold image pull can take tens
/// of seconds; the daemon docs recommend 300s.
const CREATE_TIMEOUT_MS: u64 = 300_000;
/// Trigger timeout for `sandbox::fs::*` and `sandbox::stop` — local to the
/// daemon, no meaningful timeout pressure.
const FS_TIMEOUT_MS: u64 = 30_000;
/// In-guest deadline for `create`'s `pip install iii-sdk` step — a cold
/// PyPI fetch of pydantic-core and friends can take a while on a slow
/// link, and killing it just corrupts a partial site-packages.
const SDK_INSTALL_TIMEOUT_MS: u64 = 120_000;
/// Added to the exec's in-daemon deadline for the bus round trip, so the
/// daemon's timeout (which carries the real diagnostic) fires first.
const EXEC_MARGIN_MS: u64 = 5_000;
/// Ceiling on run `code` and register `source`: they travel as
/// `sandbox::fs::write`'s (or `sandbox::run`'s) inline UTF-8 `content`,
/// whose documented inline comfort zone is 1 MiB.
pub const MAX_SOURCE_BYTES: usize = 1_048_576;
pub const MAX_FUNCTION_ID_BYTES: usize = 256;
pub const MAX_DESCRIPTION_BYTES: usize = 4096;
pub const MAX_FUNCTIONS_PER_RUNTIME: usize = 64;
const PROBE_TIMEOUT_MS: u64 = 5_000;

/// Ported from node-engine's `validate_worker_name`: a namespace's first
/// segment IS a worker name on the bus (the engine splits `a::b::c` into
/// service `a`), so it is held to the same rule.
fn validate_worker_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("name must not be empty".into());
    }
    if name.len() > 64 {
        return Err(format!("name {name:?} is longer than 64 bytes"));
    }
    if name.contains("::") {
        return Err(format!("name {name:?} must not contain \"::\""));
    }
    if name.contains("..") {
        return Err(format!("name {name:?} must not contain \"..\""));
    }
    if name.starts_with('.') {
        return Err(format!("name {name:?} must not start with '.'"));
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')))
    {
        return Err(format!(
            "name {name:?} contains {bad:?}; allowed: lowercase letters, digits, '.', '_', '-'"
        ));
    }
    Ok(())
}

/// `app::greet` → `app::`, with the first segment held to worker-name rules.
fn namespace_of(function_id: &str) -> Result<String, String> {
    let Some((head, rest)) = function_id.split_once("::") else {
        return Err(format!(
            "function id {function_id:?} must look like \"app::name\""
        ));
    };
    if rest.is_empty() {
        return Err(format!(
            "function id {function_id:?} has nothing after its namespace"
        ));
    }
    validate_worker_name(head)?;
    Ok(format!("{head}::"))
}

/// Normalize a `sandbox-code-runner::teardown` `namespace` field — "app" or
/// "app::" both accepted — to the canonical `"app::"` form `namespace_of`
/// produces, validated the same way a function id's namespace segment is.
fn normalize_namespace(namespace: &str) -> Result<String, CodeRunnerError> {
    let head = namespace.strip_suffix("::").unwrap_or(namespace);
    validate_worker_name(head).map_err(CodeRunnerError::InvalidRequest)?;
    Ok(format!("{head}::"))
}

/// Last ~500 bytes of stderr — enough to diagnose, small enough for an
/// error message.
fn stderr_tail(stderr: &str) -> &str {
    let start = stderr.len().saturating_sub(500);
    // Don't split a UTF-8 char.
    let mut i = start;
    while i < stderr.len() && !stderr.is_char_boundary(i) {
        i += 1;
    }
    &stderr[i..]
}

pub(crate) struct RegisteredFn {
    pub(crate) id: String,
    pub(crate) unregister: UnregisterFn,
}

/// No `Debug` impl on purpose: `sandbox_id` must never reach logs.
pub(crate) struct RuntimeRecord {
    pub(crate) sandbox_id: String,
    pub(crate) lang: Lang,
    /// Wall-clock creation time, reported by `list_runtimes`.
    pub(crate) created_at: SystemTime,
    /// One in-flight exec per runtime — see the module doc.
    pub(crate) exec_lock: tokio::sync::Mutex<()>,
    /// Claimed by the first registered function id: `app::greet` claims
    /// `app::`, and later ids on this runtime must share it. Always
    /// `None` on a kept-run runtime — nothing is ever registered onto one,
    /// `register_function` no longer accepts a `runtime_id` at all.
    pub(crate) namespace: Mutex<Option<String>>,
    pub(crate) functions: Mutex<Vec<RegisteredFn>>,
}

/// What a namespace runtime is keyed by. The language is part of the key
/// because a runtime is single-language (mixing is refused), so a
/// namespace with functions in both node and python holds one runtime of
/// each.
type NamespaceKey = (String, Lang);

pub struct RuntimeManager {
    cfg: Arc<CodeRunnerConfig>,
    engine: Arc<dyn Engine>,
    /// The engine URL guest SDK clients connect to, set as `III_URL` in
    /// every runtime's create-time env — see [`guest_engine_url`].
    guest_engine_url: String,
    runtimes: Mutex<HashMap<String, Arc<RuntimeRecord>>>,
    /// `(namespace, lang)` → the runtime backing it, so every
    /// `register_function` call in one namespace (and language) shares one
    /// microVM instead of needing a caller-managed `runtime_id`. Populated
    /// only by `register`; a kept-run runtime (`run keep=true`, or an
    /// explicit `runtime_id`) never appears here.
    namespaces: Mutex<HashMap<NamespaceKey, String>>,
    /// Held ACROSS `create()` on the namespace path, which is why it is a
    /// `tokio` mutex: two concurrent first registrations in one namespace
    /// must mint ONE VM, and the create is a network round trip, so a `std`
    /// guard could not span it (nor be `Send`). Taken only when a
    /// namespace's lookup finds no live binding — the steady-state reuse
    /// path checks `namespaces` under its own `std` lock and never touches
    /// this one.
    ///
    /// ponytail: one process-wide lock, so concurrent COLD starts across
    /// different namespaces serialize (a warm boot is ~a second; the
    /// ceiling is `CREATE_TIMEOUT_MS` against a wedged daemon). Upgrade path
    /// if that ever shows up: a per-`NamespaceKey` mutex map, at the cost of
    /// a second map to keep alive.
    namespace_create_lock: tokio::sync::Mutex<()>,
    /// Function ids this process has locally claimed, mapped to the
    /// runtime that holds each one. `Engine::register`'s underlying SDK
    /// registry PANICS on a duplicate id (see its `# Panics` doc) — this
    /// map is what makes two concurrent `register()` calls for the same id
    /// impossible in the first place: checked and reserved atomically
    /// BEFORE either one ever reaches the bus, so the panic is unreachable.
    /// Guards a single process only — `engine::functions::info` is what
    /// covers a collision across two sandbox-code-runner processes on one bus.
    claims: Mutex<HashMap<String, String>>,
    /// `sandbox-code-runner::event` emitter, wired once by `main` via
    /// [`Self::set_events`]. Unset in unit tests that don't observe
    /// emissions — every emit is then a no-op.
    events: OnceLock<Emitter>,
}

/// Owner recorded in `claims` for this worker's own statically registered
/// ids (see `RuntimeManager::seed_static_ids`). `create` mints runtime ids as
/// `rt-<uuid>`, so no real `runtime_id` can ever equal this constant — and
/// since no `RuntimeRecord` is ever created for it, `expire`/`teardown`
/// (which only clear claims found in a specific record's own `functions`
/// list) can never release these entries.
const STATIC_OWNER: &str = "<worker>";

fn str_field(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

/// The engine URL as a GUEST must dial it. A networked sandbox's
/// /etc/hosts maps the NAME `localhost` to the per-sandbox gateway (which
/// the daemon proxies to the host's loopback), but an IP LITERAL bypasses
/// /etc/hosts entirely and lands on the guest's own empty loopback — so a
/// loopback-IP engine address (the common `--url ws://127.0.0.1:<port>`)
/// must travel as `localhost`. Non-loopback addresses pass through
/// untouched: the guest reaches them over its outbound network like any
/// other host.
fn guest_engine_url(engine_url: &str) -> String {
    engine_url
        .replace("://127.0.0.1", "://localhost")
        .replace("://[::1]", "://localhost")
}

impl RuntimeManager {
    /// `engine_url` is the address THIS worker was pointed at (`--url`);
    /// guests get the [`guest_engine_url`] form of it as `III_URL`.
    pub fn new(cfg: Arc<CodeRunnerConfig>, engine: Arc<dyn Engine>, engine_url: &str) -> Arc<Self> {
        Arc::new(Self {
            cfg,
            engine,
            guest_engine_url: guest_engine_url(engine_url),
            runtimes: Mutex::new(HashMap::new()),
            namespaces: Mutex::new(HashMap::new()),
            namespace_create_lock: tokio::sync::Mutex::new(()),
            claims: Mutex::new(HashMap::new()),
            events: OnceLock::new(),
        })
    }

    /// Wire the `sandbox-code-runner::event` emitter — once, from `main`,
    /// before any caller-facing function is invoked. A second call is
    /// ignored.
    pub fn set_events(&self, events: Emitter) {
        let _ = self.events.set(events);
    }

    /// Fire a lifecycle event when an emitter is wired. Fan-out inside the
    /// emitter is fire-and-forget — this never blocks or fails the calling
    /// handler.
    fn emit(&self, event: Event) {
        if let Some(events) = self.events.get() {
            events.emit(event);
        }
    }

    /// Seed `claims` with this worker's own statically registered ids
    /// (`functions::STATIC_IDS`) before any caller-facing function is
    /// invoked, so `reserve` refuses a caller who tries to register over one
    /// of them the same way it refuses any other already-claimed id — this
    /// protection no longer depends on the `engine::functions::info` probe
    /// (a network round trip) to catch the collision. Idempotent: re-seeding
    /// the same ids just overwrites their owner with the same sentinel.
    pub fn seed_static_ids(&self, ids: &[&str]) {
        let mut claims = self.claims.lock().unwrap();
        for id in ids {
            claims.insert((*id).to_string(), STATIC_OWNER.to_string());
        }
    }

    fn get(&self, runtime_id: &str) -> Result<Arc<RuntimeRecord>, CodeRunnerError> {
        self.runtimes
            .lock()
            .unwrap()
            .get(runtime_id)
            .cloned()
            .ok_or_else(|| CodeRunnerError::RuntimeNotFound(runtime_id.to_string()))
    }

    /// This namespace's runtime for `lang`, but ONLY if it is still live.
    /// The liveness check is what makes a stale binding structurally
    /// harmless: a runtime that died without its binding being dropped just
    /// reads as "not bound", and the next registration creates and rebinds.
    fn bound_namespace(&self, ns: &str, lang: Lang) -> Option<(String, Arc<RuntimeRecord>)> {
        let id = self
            .namespaces
            .lock()
            .unwrap()
            .get(&(ns.to_string(), lang))
            .cloned()?;
        let record = self.runtimes.lock().unwrap().get(&id).cloned()?;
        Some((id, record))
    }

    /// Drop every namespace binding pointing at `runtime_id`. Called from
    /// each path that removes a record (`teardown`, `expire`) so the map
    /// stays proportional to live runtimes; a kept-run runtime is never in
    /// this map, so this is a harmless no-op scan for one.
    fn unbind_namespace(&self, runtime_id: &str) {
        self.namespaces
            .lock()
            .unwrap()
            .retain(|_, id| id != runtime_id);
    }

    /// Resolve the runtime for a `register_function` call: reuse this
    /// namespace's live runtime for `lang`, or create one and bind it.
    /// Mirrors the now-deleted session-binding create path's
    /// double-checked-locking shape: two concurrent first registrations in
    /// one namespace must mint ONE VM, not two.
    async fn namespace_runtime(
        &self,
        ns: &str,
        lang: Lang,
    ) -> Result<(String, Arc<RuntimeRecord>), CodeRunnerError> {
        if let Some((id, record)) = self.bound_namespace(ns, lang) {
            return Ok((id, record));
        }

        // Nothing bound (yet). Serialize the create so two concurrent first
        // registrations in one namespace cannot both boot a VM — and
        // re-check the binding on the way in, because the other one may
        // have finished while this call waited for the lock.
        let _creating = self.namespace_create_lock.lock().await;
        if let Some((id, record)) = self.bound_namespace(ns, lang) {
            return Ok((id, record));
        }

        let (id, record) = self.create(lang).await?;
        self.namespaces
            .lock()
            .unwrap()
            .insert((ns.to_string(), lang), id.clone());
        tracing::info!(namespace = %ns, lang = ?lang, "created a runtime for this namespace");
        self.emit(Event::runtime_created(&id, lang, &record.sandbox_id));
        Ok((id, record))
    }

    /// Map a failed engine call when there is no live record to expire —
    /// the create path. `Gone` CAN occur here: `idle_ttl_secs` has no floor
    /// (see `CodeRunnerConfig::effective_idle_ttl_secs`), so an
    /// operator-set low value can make the daemon reap the sandbox between
    /// `sandbox::create` returning and the runner plant's `sandbox::fs::write`
    /// landing — proven with a scratch test failing the plant with S002.
    /// Like `sandbox_call`'s `Gone` arm, this must NOT pass the daemon's raw
    /// message through: it embeds `sandbox_id` ("no sandbox with that id
    /// {id}"), and the caller here never received an id and cannot act on
    /// it either way — a fixed, id-free message is strictly more useful.
    fn map_failure(raw: &str) -> CodeRunnerError {
        match classify_sandbox_error(raw) {
            SandboxFailure::Gone => CodeRunnerError::Engine(
                "the sandbox was reaped or lost during creation; retry".to_string(),
            ),
            SandboxFailure::Timeout => CodeRunnerError::Timeout,
            SandboxFailure::Capacity(m) => CodeRunnerError::Capacity(m),
            SandboxFailure::Other(m) => CodeRunnerError::Engine(m),
        }
    }

    /// Every `sandbox::*` call against a LIVE runtime goes through here: on
    /// `Gone` (S002/S004 — the daemon reaped or lost the VM) the runtime is
    /// expired — bus functions unregistered, record forgotten — before the
    /// error returns.
    pub(crate) async fn sandbox_call(
        &self,
        runtime_id: &str,
        fn_id: &str,
        payload: Value,
        timeout_ms: u64,
    ) -> Result<Value, CodeRunnerError> {
        match self
            .engine
            .call(fn_id.to_string(), payload, timeout_ms)
            .await
        {
            Ok(v) => Ok(v),
            Err(raw) => Err(match classify_sandbox_error(&raw) {
                SandboxFailure::Gone => {
                    self.expire(runtime_id);
                    CodeRunnerError::Expired(runtime_id.to_string())
                }
                SandboxFailure::Timeout => CodeRunnerError::Timeout,
                SandboxFailure::Capacity(m) => CodeRunnerError::Capacity(m),
                SandboxFailure::Other(m) => CodeRunnerError::Engine(m),
            }),
        }
    }

    /// The VM is gone: unregister the runtime's bus functions and forget the
    /// record. Idempotent — a second Gone for the same id finds nothing. Does
    /// NOT call `sandbox::stop` — the whole premise is that the daemon
    /// already reaped or lost the VM, so there is nothing left to stop.
    /// Emits the same `teardown` event a real teardown does: subscribers
    /// must learn about reaper-driven removal too.
    pub(crate) fn expire(&self, runtime_id: &str) {
        let record = self.runtimes.lock().unwrap().remove(runtime_id);
        self.unbind_namespace(runtime_id);
        if let Some(r) = record {
            let mut claims = self.claims.lock().unwrap();
            for f in r.functions.lock().unwrap().drain(..) {
                // A leaked claim would be a function id that can never be
                // registered again for the life of the process.
                claims.remove(&f.id);
                (f.unregister)();
                tracing::warn!(id = %f.id, "unregistered: its runtime's VM expired");
            }
            drop(claims);
            self.emit(Event::teardown(runtime_id));
        }
    }

    /// Boot a sandbox, plant this language's guest files (runner, iii
    /// library, run wrapper — plus the embedded SDK bundle for Node),
    /// install the Python SDK where applicable, mint the record. Used by
    /// `namespace_runtime` AND by every `run` that has no `runtime_id` —
    /// one-shot and `keep: true` alike — since only `sandbox::create` can
    /// enable networking and set the create-time env the guest SDK link
    /// (`III_URL`) depends on.
    ///
    /// Always creates WITH network: the guest `iii` global is a real
    /// iii-sdk client dialing the engine through the sandbox gateway, and
    /// the gateway only exists on a networked VM. (Outbound internet —
    /// npm/pip installs included — comes with that NIC; there is no
    /// engine-only network mode in the daemon.) `OTEL_ENABLED=false`
    /// keeps guest SDK clients from starting telemetry exporters whose
    /// timers and console prints would pollute run output and delay
    /// process exit.
    async fn create(&self, lang: Lang) -> Result<(String, Arc<RuntimeRecord>), CodeRunnerError> {
        let created = self
            .engine
            .call(
                "sandbox::create".to_string(),
                json!({
                    "image": lang.image(),
                    "idle_timeout_secs": self.cfg.effective_idle_ttl_secs(),
                    "network": true,
                    "env": {
                        "III_URL": self.guest_engine_url,
                        "OTEL_ENABLED": "false",
                    },
                }),
                CREATE_TIMEOUT_MS,
            )
            .await
            .map_err(|raw| Self::map_failure(&raw))?;
        let sandbox_id = created
            .get("sandbox_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                CodeRunnerError::Engine("sandbox::create returned no sandbox_id".into())
            })?
            .to_string();

        // Plant the guest files. On failure, stop the sandbox rather than
        // leak it: the caller never received an id, so nothing could ever
        // address this VM again — it would sit in a daemon slot until the
        // idle reaper. (node-engine leaked a slot per failed create until
        // the same guard was added.)
        let mut planted = Ok(Value::Null);
        for (path, content) in lang.guest_files() {
            planted = self
                .engine
                .call(
                    "sandbox::fs::write".to_string(),
                    json!({
                        "sandbox_id": sandbox_id,
                        "path": path,
                        "content": content,
                        "parents": true,
                    }),
                    FS_TIMEOUT_MS,
                )
                .await;
            if planted.is_err() {
                break;
            }
        }
        if let Err(raw) = planted {
            if let Err(stop_raw) = self
                .engine
                .call(
                    "sandbox::stop".to_string(),
                    json!({ "sandbox_id": sandbox_id, "wait": false }),
                    FS_TIMEOUT_MS,
                )
                .await
            {
                if !matches!(classify_sandbox_error(&stop_raw), SandboxFailure::Gone) {
                    // No caller ever received this runtime's id, so a
                    // failed stop here leaks a daemon slot for the full
                    // idle TTL silently.
                    tracing::warn!(
                        error = %stop_raw,
                        "sandbox::stop failed after a failed runner plant; it will hold a \
                         daemon slot until its own idle TTL"
                    );
                }
            }
            return Err(Self::map_failure(&raw));
        }

        // Python's SDK cannot be planted (pydantic-core is compiled
        // per-platform), so it is pip-installed once per runtime, here,
        // while nothing else can be running in the VM. DEGRADES rather
        // than fails: a dead registry must not take plain `run` down
        // with it — the guest's `iii` then raises a clear
        // "not installed" error on first use instead.
        if lang == Lang::Python {
            let install = self
                .engine
                .call(
                    "sandbox::exec".to_string(),
                    json!({
                        "sandbox_id": sandbox_id,
                        "cmd": "python3",
                        "args": [
                            "-m", "pip", "install",
                            "--quiet", "--disable-pip-version-check",
                            "iii-sdk",
                        ],
                        "timeout_ms": SDK_INSTALL_TIMEOUT_MS,
                    }),
                    SDK_INSTALL_TIMEOUT_MS + EXEC_MARGIN_MS,
                )
                .await;
            let ok = matches!(
                &install,
                Ok(v) if v.get("exit_code").and_then(|c| c.as_i64()) == Some(0)
            );
            if !ok {
                let detail = match &install {
                    Ok(v) => str_field(v, "stderr"),
                    Err(raw) => raw.clone(),
                };
                tracing::warn!(
                    error = %stderr_tail(&detail),
                    "pip install iii-sdk failed; this Python runtime's `iii` global will \
                     error on first use"
                );
            }
        }

        let runtime_id = format!("rt-{}", uuid::Uuid::new_v4());
        let record = Arc::new(RuntimeRecord {
            sandbox_id,
            lang,
            created_at: SystemTime::now(),
            exec_lock: tokio::sync::Mutex::new(()),
            namespace: Mutex::new(None),
            functions: Mutex::new(Vec::new()),
        });
        self.runtimes
            .lock()
            .unwrap()
            .insert(runtime_id.clone(), record.clone());
        Ok((runtime_id, record))
    }

    /// `run` has three paths, gated on `req.runtime_id` and `req.keep`:
    ///
    /// 1. `runtime_id` present → reuse that VM via write+exec. NOT stopped —
    ///    the caller owns it. `lang` mismatch is refused; `network` stays
    ///    documented-as-ignored (the caller already chose this runtime).
    /// 2. `runtime_id` absent, `keep: true` → boot a runtime (`create`, so
    ///    the guest files are planted), run into it, leave it running; the
    ///    minted `runtime_id` is recorded and returned. A FAILED run
    ///    destroys the fresh runtime instead: an `Err` carries no
    ///    `runtime_id`, so keeping the VM would strand it in a daemon slot
    ///    nobody can ever address.
    /// 3. `runtime_id` absent, default → same boot, destroyed after the
    ///    run, success or failure; the response carries no `runtime_id` —
    ///    there is nothing left to address, and returning a dead id would
    ///    be worse than none.
    ///
    /// Every path runs the code under the run wrapper, so the code gets the
    /// lazy `iii` SDK global (`III_URL` is in the runtime's env from
    /// `create`).
    pub async fn run(self: &Arc<Self>, req: RunRequest) -> Result<RunResponse, CodeRunnerError> {
        if req.code.is_empty() {
            return Err(CodeRunnerError::InvalidRequest(
                "code must not be empty".into(),
            ));
        }
        if req.code.len() > MAX_SOURCE_BYTES {
            return Err(CodeRunnerError::InvalidRequest(format!(
                "code is {} bytes; the limit is {MAX_SOURCE_BYTES}",
                req.code.len()
            )));
        }
        let timeout_ms = self.cfg.clamp_timeout(req.timeout_ms).as_millis() as u64;

        if let Some(id) = &req.runtime_id {
            let record = self.get(id)?;
            if let Some(lang) = req.lang {
                if lang != record.lang {
                    return Err(CodeRunnerError::InvalidRequest(format!(
                        "this runtime runs {:?}; omit `lang` or pass the matching one — \
                         languages cannot be mixed in one runtime",
                        record.lang
                    )));
                }
            }
            let mut resp = self.run_into(id, &record, &req.code, timeout_ms).await?;
            resp.runtime_id = Some(id.clone());
            self.emit(Event::run_settled(resp.runtime_id.as_deref(), record.lang));
            return Ok(resp);
        }

        let lang = req.lang.ok_or_else(|| {
            CodeRunnerError::InvalidRequest(
                "`lang` is required when there is no `runtime_id`: \"node\" or \"python\"".into(),
            )
        })?;

        let (id, record) = self.create(lang).await?;
        let result = self.run_into(&id, &record, &req.code, timeout_ms).await;

        // The caller receives the id ONLY on a kept, successful run; on
        // every other outcome the runtime must not outlive this call.
        let keep = req.keep && result.is_ok();
        if !keep {
            match self.destroy_runtime(&id).await {
                // Already gone — the run itself discovered the VM reaped
                // and expired the record. That IS the destroyed outcome.
                Ok(_) | Err(CodeRunnerError::RuntimeNotFound(_)) => {}
                Err(e) => tracing::warn!(
                    error = %e,
                    "one-shot run cleanup failed; the daemon's idle reaper is the backstop"
                ),
            }
        }

        let mut resp = result.map_err(Self::redact_boot_run_error)?;
        if keep {
            self.emit(Event::runtime_created(&id, lang, &record.sandbox_id));
        }
        resp.runtime_id = keep.then_some(id);
        self.emit(Event::run_settled(resp.runtime_id.as_deref(), lang));
        Ok(resp)
    }

    /// Freshly-booted run runtimes are internal until the response hands
    /// the id out, so an `Expired`/`RuntimeNotFound` raced mid-run must
    /// not quote an id this caller never held (`error.rs`'s id-quoting
    /// exception covers only ids the caller supplied). Mirrors
    /// `redact_register_error`.
    fn redact_boot_run_error(e: CodeRunnerError) -> CodeRunnerError {
        match e {
            CodeRunnerError::Expired(_) | CodeRunnerError::RuntimeNotFound(_) => {
                CodeRunnerError::Engine(
                    "the run's VM was reaped or lost mid-run; retry".to_string(),
                )
            }
            other => other,
        }
    }

    /// Write `code` into the runtime and run it under the run wrapper.
    /// Returns `runtime_id: None` — each caller decides what id, if any,
    /// its own caller may see.
    async fn run_into(
        self: &Arc<Self>,
        runtime_id: &str,
        record: &Arc<RuntimeRecord>,
        code: &str,
        timeout_ms: u64,
    ) -> Result<RunResponse, CodeRunnerError> {
        let _guard = record.exec_lock.lock().await;

        let file = format!(
            "/tmp/sandbox-code-runner/run-{}.{}",
            uuid::Uuid::new_v4(),
            record.lang.ext()
        );
        self.sandbox_call(
            runtime_id,
            "sandbox::fs::write",
            json!({
                "sandbox_id": record.sandbox_id,
                "path": file,
                "content": code,
                "parents": true,
            }),
            FS_TIMEOUT_MS,
        )
        .await?;

        let out = self
            .exec_guest(
                runtime_id,
                record,
                vec![record.lang.run_wrapper_path().to_string(), file],
                None,
                "sandbox-code-runner:run",
                timeout_ms,
            )
            .await?;

        if out
            .get("timed_out")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Err(CodeRunnerError::Timeout);
        }
        Ok(RunResponse {
            runtime_id: None,
            stdout: str_field(&out, "stdout"),
            stderr: str_field(&out, "stderr"),
            exit_code: out.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1),
            success: out
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            duration_ms: out.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0),
        })
    }

    /// One guest exec. `III_URL` (and the SDK itself) is already in the
    /// runtime from `create`; the only per-exec env is a worker name, so
    /// a guest that does use `iii` shows up in the engine's worker list
    /// as something identifiable instead of `hostname:pid`. The caller
    /// MUST already hold `record.exec_lock` — this is the exec half of
    /// the write+exec sequence that lock serializes.
    async fn exec_guest(
        self: &Arc<Self>,
        runtime_id: &str,
        record: &RuntimeRecord,
        args: Vec<String>,
        stdin_b64: Option<String>,
        worker_name: &str,
        timeout_ms: u64,
    ) -> Result<Value, CodeRunnerError> {
        let mut payload = json!({
            "sandbox_id": record.sandbox_id,
            "cmd": record.lang.interpreter(),
            "args": args,
            "env": { "III_WORKER_NAME": worker_name },
            "timeout_ms": timeout_ms,
        });
        if let Some(stdin) = stdin_b64 {
            payload["stdin"] = json!(stdin);
        }

        self.sandbox_call(
            runtime_id,
            "sandbox::exec",
            payload,
            timeout_ms + EXEC_MARGIN_MS,
        )
        .await
    }

    /// Destroy one runtime (whatever kind it is): drain in-flight work,
    /// unregister its bus functions, best-effort stop its sandbox, forget
    /// the record. Shared by `teardown`'s `runtime_id` arm and, once per
    /// matching runtime, its `namespace` arm.
    async fn destroy_runtime(&self, runtime_id: &str) -> Result<Vec<String>, CodeRunnerError> {
        // Remove first: new calls see NotFound immediately, and a failure
        // below cannot resurrect the record.
        let record = self
            .runtimes
            .lock()
            .unwrap()
            .remove(runtime_id)
            .ok_or_else(|| CodeRunnerError::RuntimeNotFound(runtime_id.to_string()))?;
        self.unbind_namespace(runtime_id);

        // Wait for whatever is already in flight to drain before touching
        // the sandbox out from under it: a run already past the lookup
        // above holds its own `Arc<RuntimeRecord>` clone made before this
        // removal, so it runs to completion on the SAME `exec_lock` — this
        // await genuinely waits for that call to finish rather than racing
        // it, and only then do we unregister and stop. Bounded: a run
        // holds this lock for at most its own clamped timeout.
        let _guard = record.exec_lock.lock().await;

        let mut unregistered = Vec::new();
        {
            let mut claims = self.claims.lock().unwrap();
            for f in record.functions.lock().unwrap().drain(..) {
                claims.remove(&f.id);
                (f.unregister)();
                unregistered.push(f.id);
            }
        }

        // Best-effort: Gone means the daemon already reaped it — that IS the
        // requested outcome. Anything else is logged; the daemon's idle
        // reaper is the backstop.
        if let Err(raw) = self
            .engine
            .call(
                "sandbox::stop".to_string(),
                json!({ "sandbox_id": record.sandbox_id, "wait": false }),
                FS_TIMEOUT_MS,
            )
            .await
        {
            if !matches!(classify_sandbox_error(&raw), SandboxFailure::Gone) {
                tracing::warn!(
                    error = %raw,
                    "sandbox::stop failed during teardown; the daemon's idle reaper is the backstop"
                );
            }
        }

        Ok(unregistered)
    }

    /// Snapshot every live runtime — kept-run and namespace runtimes alike
    /// — newest first. Reads only in-process state, no daemon round trip:
    /// a runtime the daemon reaped behind our back still lists until
    /// something touches it (`sandbox_call`'s Gone handling) or tears it
    /// down.
    pub async fn list_runtimes(&self) -> ListRuntimesResponse {
        let records: Vec<(String, Arc<RuntimeRecord>)> = self
            .runtimes
            .lock()
            .unwrap()
            .iter()
            .map(|(id, r)| (id.clone(), r.clone()))
            .collect();
        // Reconcile against the daemon so a reaped VM reads as `vm_gone`
        // instead of listing as alive until the next use fails. Best-effort:
        // a failed read means "unknown", never a claim either way.
        let live_vms: Option<std::collections::HashSet<String>> = self
            .engine
            .call(
                "sandbox::list".to_string(),
                serde_json::json!({}),
                FS_TIMEOUT_MS,
            )
            .await
            .ok()
            .and_then(|v| {
                let ids = v
                    .get("sandboxes")?
                    .as_array()?
                    .iter()
                    .filter(|s| !s.get("stopped").and_then(|b| b.as_bool()).unwrap_or(false))
                    .filter_map(|s| s.get("sandbox_id")?.as_str().map(str::to_string))
                    .collect();
                Some(ids)
            });
        let mut runtimes: Vec<RuntimeSummary> = records
            .into_iter()
            .map(|(runtime_id, r)| RuntimeSummary {
                runtime_id,
                lang: r.lang,
                sandbox_id: r.sandbox_id.clone(),
                created_at_ms: r
                    .created_at
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                registered_functions: r
                    .functions
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|f| f.id.clone())
                    .collect(),
                vm_gone: live_vms.as_ref().map(|live| !live.contains(&r.sandbox_id)),
            })
            .collect();
        runtimes.sort_by(|a, b| {
            b.created_at_ms
                .cmp(&a.created_at_ms)
                .then_with(|| a.runtime_id.cmp(&b.runtime_id))
        });
        ListRuntimesResponse { runtimes }
    }

    /// Accepts exactly one of `runtime_id` (a kept-run runtime) or
    /// `namespace` (every runtime — one per language — backing a
    /// `register_function` namespace). A namespace teardown unregisters
    /// every function under it, exactly as a by-id teardown does today.
    pub async fn teardown(
        &self,
        req: TeardownRequest,
    ) -> Result<TeardownResponse, CodeRunnerError> {
        match (req.runtime_id, req.namespace) {
            (Some(_), Some(_)) => Err(CodeRunnerError::InvalidRequest(
                "pass exactly one of runtime_id or namespace, not both: runtime_id tears down \
                 a single kept-run runtime (from sandbox-code-runner::run keep=true), namespace tears \
                 down every runtime backing a register_function namespace"
                    .into(),
            )),
            (None, None) => Err(CodeRunnerError::InvalidRequest(
                "pass exactly one of runtime_id (a kept run's runtime) or namespace (a \
                 register_function namespace, e.g. \"app\" for ids like app::greet)"
                    .into(),
            )),
            (Some(id), None) => {
                let unregistered = self.destroy_runtime(&id).await?;
                self.emit(Event::teardown(&id));
                Ok(TeardownResponse {
                    runtime_id: Some(id),
                    namespace: None,
                    torn_down: true,
                    unregistered,
                })
            }
            (None, Some(raw_ns)) => {
                let ns = normalize_namespace(&raw_ns)?;
                let ids: Vec<String> = {
                    let namespaces = self.namespaces.lock().unwrap();
                    namespaces
                        .iter()
                        .filter(|((n, _), _)| n == &ns)
                        .map(|(_, id)| id.clone())
                        .collect()
                };
                if ids.is_empty() {
                    return Err(CodeRunnerError::NamespaceNotFound(ns));
                }
                let mut unregistered = Vec::new();
                for id in ids {
                    match self.destroy_runtime(&id).await {
                        Ok(mut u) => {
                            unregistered.append(&mut u);
                            self.emit(Event::teardown(&id));
                        }
                        // Already gone (e.g. a concurrent teardown/expire
                        // beat this loop to it) reads as already torn
                        // down, not a failure — same as the by-id path's
                        // "reaped sandbox" success case.
                        Err(CodeRunnerError::RuntimeNotFound(_)) => {}
                        Err(e) => return Err(e),
                    }
                }
                Ok(TeardownResponse {
                    runtime_id: None,
                    namespace: Some(ns),
                    torn_down: true,
                    unregistered,
                })
            }
        }
    }

    /// Atomically check-and-reserve everything two concurrent `register()`
    /// calls could otherwise race on: the runtime's namespace (must match
    /// what's already claimed, or be unclaimed), this process's own local
    /// claim on `function_id`, and the per-runtime function cap. All three
    /// checks and both writes happen under one lock acquisition, so only
    /// one caller can ever win a given id — synchronously, before either
    /// caller makes a network call. This is what makes
    /// `Engine::register`'s duplicate-id panic (its underlying SDK
    /// registry's documented `# Panics` behavior) unreachable: two callers
    /// can no longer both pass the `engine::functions::info` probe and both
    /// reach it for the same id.
    fn reserve(
        &self,
        runtime_id: &str,
        record: &RuntimeRecord,
        function_id: &str,
        ns: &str,
    ) -> Result<(), CodeRunnerError> {
        let mut claims = self.claims.lock().unwrap();
        let mut namespace = record.namespace.lock().unwrap();
        let functions = record.functions.lock().unwrap();

        if let Some(existing) = namespace.as_ref() {
            if existing != ns {
                return Err(CodeRunnerError::InvalidRequest(format!(
                    "registered id {function_id:?} must start with this runtime's namespace \
                     {existing:?} — rename the id, or use a runtime whose namespace covers it"
                )));
            }
        }
        if claims.contains_key(function_id) {
            return Err(CodeRunnerError::InvalidRequest(format!(
                "function id {function_id} is already registered on the bus"
            )));
        }
        if functions.len() >= MAX_FUNCTIONS_PER_RUNTIME {
            return Err(CodeRunnerError::InvalidRequest(format!(
                "this runtime already holds {MAX_FUNCTIONS_PER_RUNTIME} functions"
            )));
        }

        if namespace.is_none() {
            *namespace = Some(ns.to_string());
        }
        claims.insert(function_id.to_string(), runtime_id.to_string());
        Ok(())
    }

    /// Undo a `reserve` that a later step (the probe, the plant, or the
    /// publish) failed to follow through on: release the local claim, and
    /// clear the runtime's namespace ONLY if nothing else — a concurrent
    /// reservation, or an already-committed function — still depends on it.
    /// A refused registration must never leave a namespace pinned with
    /// nothing behind it; a namespace another registration relies on must
    /// never be cleared out from under it.
    fn release(&self, function_id: &str, record: &RuntimeRecord, runtime_id: &str) {
        let mut claims = self.claims.lock().unwrap();
        claims.remove(function_id);
        if !claims.values().any(|owner| owner == runtime_id) {
            *record.namespace.lock().unwrap() = None;
        }
    }

    /// No `runtime_id` on the wire: sandbox-code-runner resolves (creating if
    /// needed) the persistent runtime for `(namespace_of(function_id),
    /// req.lang)` itself — see `namespace_runtime`.
    pub async fn register(
        self: &Arc<Self>,
        req: RegisterRequest,
    ) -> Result<RegisterResponse, CodeRunnerError> {
        if req.source.is_empty() {
            return Err(CodeRunnerError::InvalidRequest(
                "source must not be empty; it must define handler(payload)".into(),
            ));
        }
        if req.source.len() > MAX_SOURCE_BYTES {
            return Err(CodeRunnerError::InvalidRequest(format!(
                "source is {} bytes; the limit is {MAX_SOURCE_BYTES}",
                req.source.len()
            )));
        }
        if req.function_id.len() > MAX_FUNCTION_ID_BYTES {
            return Err(CodeRunnerError::InvalidRequest(format!(
                "function id is longer than {MAX_FUNCTION_ID_BYTES} bytes"
            )));
        }
        if let Some(d) = &req.description {
            if d.len() > MAX_DESCRIPTION_BYTES {
                return Err(CodeRunnerError::InvalidRequest(format!(
                    "description is longer than {MAX_DESCRIPTION_BYTES} bytes"
                )));
            }
        }
        let ns = namespace_of(&req.function_id).map_err(CodeRunnerError::InvalidRequest)?;

        // Cheap, synchronous fail-fast: an id already claimed (including a
        // seeded static one — `sandbox-code-runner::*` is never a legitimate
        // namespace to register into) is refused before `namespace_runtime`
        // ever creates or reuses a VM for it. Not the authoritative check —
        // `reserve` still does that, atomically, once a record exists — this
        // purely avoids booting a doomed namespace runtime for a request
        // that cannot possibly succeed.
        if self.claims.lock().unwrap().contains_key(&req.function_id) {
            return Err(CodeRunnerError::InvalidRequest(format!(
                "function id {} is already registered on the bus",
                req.function_id
            )));
        }

        let (runtime_id, record) = self.namespace_runtime(&ns, req.lang).await?;

        // Reserve the id (and, if this is the runtime's first, its
        // namespace) BEFORE any network call — see `reserve`'s doc for the
        // race this closes.
        self.reserve(&runtime_id, &record, &req.function_id, &ns)?;

        let weak = Arc::downgrade(self);
        match self.publish(&req, &runtime_id, &record, weak).await {
            Ok(resp) => {
                self.emit(Event::function_registered(
                    &req.function_id,
                    &runtime_id,
                    req.lang,
                ));
                Ok(resp)
            }
            Err(e) => {
                self.release(&req.function_id, &record, &runtime_id);
                Err(Self::redact_register_error(e))
            }
        }
    }

    /// `register_function`'s caller never supplies or receives a
    /// `runtime_id` — sandbox-code-runner resolves the namespace runtime
    /// internally (`namespace_runtime`) — so unlike the DIRECT `run` /
    /// `teardown` paths, where `error.rs`'s id-quoting `Expired` /
    /// `RuntimeNotFound` messages are a documented exception (the id goes
    /// back to the caller who already supplied it), this caller has no
    /// business receiving one either. The only way either variant can
    /// reach here is `publish`'s post-plant re-check racing a concurrent
    /// teardown of this namespace — folds to a generic, id-free message
    /// rather than a stable `expired`/`runtime_not_found` code quoting an
    /// id nobody on this call ever held. Mirrors `redact_proxy_error`'s
    /// intent for the proxy-invocation caller; kept separate because that
    /// one returns a bare `String` (the `ProxyHandler` contract) where this
    /// one must stay a `CodeRunnerError` (this function's own return type).
    fn redact_register_error(e: CodeRunnerError) -> CodeRunnerError {
        match e {
            CodeRunnerError::Expired(_) | CodeRunnerError::RuntimeNotFound(_) => {
                CodeRunnerError::Engine(
                    "this namespace's runtime was torn down while the registration was in \
                     flight; register again"
                        .into(),
                )
            }
            other => other,
        }
    }

    /// The probe → plant → publish sequence, run only once `reserve` has
    /// already won this id locally. Any `Err` here is rolled back by the
    /// caller (`register`) via `release`.
    async fn publish(
        &self,
        req: &RegisterRequest,
        runtime_id: &str,
        record: &Arc<RuntimeRecord>,
        weak: std::sync::Weak<RuntimeManager>,
    ) -> Result<RegisterResponse, CodeRunnerError> {
        // Probe the bus. Found = taken; NOT_FOUND-style error = free; any
        // other answer fails CLOSED — an unverifiable id is not published.
        match self
            .engine
            .call(
                "engine::functions::info".to_string(),
                json!({ "function_id": req.function_id }),
                PROBE_TIMEOUT_MS,
            )
            .await
        {
            Ok(_) => {
                return Err(CodeRunnerError::InvalidRequest(format!(
                    "function id {} is already registered on the bus",
                    req.function_id
                )))
            }
            Err(raw) => {
                // `classify_probe_error` distinguishes "the target id is
                // free" from "this engine cannot dispatch the probe at all"
                // — a lowercase substring match on "not found" alone cannot
                // tell those apart (both raw strings contain it), and
                // reading the latter as "free" would invert this
                // deliberately fail-CLOSED gate: it is the ONLY cross-process
                // guard against two sandbox-code-runner workers colliding on one bus
                // id (the SDK's own registry panics on a duplicate id with
                // nothing serializing two concurrent registrations across
                // processes). See `classify_probe_error`'s doc.
                if classify_probe_error(&raw, &req.function_id) != ProbeOutcome::Free {
                    return Err(CodeRunnerError::Engine(format!(
                        "could not verify that {} is free: {raw}",
                        req.function_id
                    )));
                }
            }
        }

        // Hold `exec_lock` across the plant AND the push-plus-bus-register
        // below, not just the plant: `teardown()` removes the runtime from
        // `self.runtimes` FIRST (no lock needed for that) and only THEN
        // waits on this same lock before draining `record.functions`.
        // Releasing early (right after the plant, as a prior version of
        // this code did) let teardown's drain run, find nothing to
        // unregister, and finish — while this call then pushed and
        // published anyway: a function live on the bus with no
        // `functions` entry left to ever unregister it, and a `claims`
        // entry that could never be released. Holding the lock across both
        // steps rules that out: teardown's drain cannot happen in between
        // "planted" and "published" — it can only land fully before (in
        // which case the re-check below catches it) or fully after (in
        // which case it correctly finds and unregisters what we just
        // pushed).
        let _guard = record.exec_lock.lock().await;

        let path = format!(
            "/opt/sandbox-code-runner/fns/{}.{}",
            uuid::Uuid::new_v4(),
            record.lang.ext()
        );
        self.sandbox_call(
            runtime_id,
            "sandbox::fs::write",
            json!({
                "sandbox_id": record.sandbox_id,
                "path": path,
                "content": req.source,
                "parents": true,
            }),
            FS_TIMEOUT_MS,
        )
        .await?;
        let source_path = path;

        // Re-verify, still under `exec_lock`: `teardown()` may have removed
        // the runtime from `self.runtimes` while the plant's network round
        // trip was in flight. If it did, refuse to publish onto a runtime
        // that is already gone — the caller gets the same `Expired` any
        // other call against a torn-down runtime gets, not a stale `Ok`.
        self.get(runtime_id)
            .map_err(|_| CodeRunnerError::Expired(runtime_id.to_string()))?;

        // Publish the proxy. `Weak` breaks the cycle manager → record →
        // (engine's registry) → proxy → manager, and lets a proxy that
        // outlives the manager answer cleanly instead of keeping it alive.
        let proxy_runtime_id = runtime_id.to_string();
        let proxy_source_path = source_path.clone();
        let proxy_function_id = req.function_id.clone();
        let handler: crate::engine::ProxyHandler = Arc::new(move |payload| {
            let weak = weak.clone();
            let runtime_id = proxy_runtime_id.clone();
            let source_path = proxy_source_path.clone();
            let function_id = proxy_function_id.clone();
            Box::pin(async move {
                let Some(m) = weak.upgrade() else {
                    return Err("sandbox-code-runner is shutting down".to_string());
                };
                m.invoke_registered(&runtime_id, &function_id, &source_path, payload)
                    .await
                    .map_err(Self::redact_proxy_error)
            })
        });
        let unregister =
            self.engine
                .register(req.function_id.clone(), req.description.clone(), handler);

        // The namespace and the local claim were already set by `reserve`;
        // only the committed function list is new here.
        record.functions.lock().unwrap().push(RegisteredFn {
            id: req.function_id.clone(),
            unregister,
        });

        Ok(RegisterResponse {
            function_id: req.function_id.clone(),
            registered: true,
        })
    }

    /// `error.rs`'s "deliberate exception" — `Display`ing a runtime id
    /// verbatim — is justified for the DIRECT `sandbox-code-runner::run` /
    /// `teardown` paths: the id goes back to the caller who already
    /// supplied it. A registered function's PROXY is a different caller
    /// entirely — whoever calls `app::greet` never held `runtime_id` and
    /// has no business receiving it. Strip it from the two variants that
    /// quote it (`Expired`, `RuntimeNotFound`) before stringifying; the
    /// code stays stable so a caller can still branch on it, only the
    /// id-bearing message is replaced.
    fn redact_proxy_error(e: CodeRunnerError) -> String {
        let code = e.code();
        match e {
            CodeRunnerError::Expired(_) | CodeRunnerError::RuntimeNotFound(_) => {
                format!("{code}: this function is no longer backed by a live runtime")
            }
            other => other.to_string(),
        }
    }

    /// One bus call of a registered function: exec the runner against the
    /// planted source with the payload on stdin. Handler prints are logged
    /// at debug, not returned — the caller gets exactly what `handler`
    /// returned. Handlers get the same lazy `iii` global runs do.
    async fn invoke_registered(
        self: &Arc<Self>,
        runtime_id: &str,
        function_id: &str,
        source_path: &str,
        payload: Value,
    ) -> Result<Value, CodeRunnerError> {
        // A proxy can be invoked in the window between expiry/teardown and
        // its unregistration landing; answer "expired", not "not found".
        let record = self
            .get(runtime_id)
            .map_err(|_| CodeRunnerError::Expired(runtime_id.to_string()))?;

        let timeout_ms = self.cfg.default_timeout_ms;
        let _guard = record.exec_lock.lock().await;

        // The sentinel rides in the stdin envelope, NOT in argv: the handler
        // is loaded into the runner's own process, so argv is ambient state
        // it can read — and a handler that can read the sentinel can print a
        // forged frame ahead of the runner's real one. The runner consumes
        // stdin before the handler loads, so by the time handler code runs
        // the envelope is gone.
        let sentinel = uuid::Uuid::new_v4().to_string();
        use base64::Engine as _;
        let envelope = json!({ "sentinel": sentinel, "payload": payload });
        let stdin_b64 = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_vec(&envelope).expect("a Value serializes"));

        let out = self
            .exec_guest(
                runtime_id,
                &record,
                vec![
                    record.lang.runner_path().to_string(),
                    source_path.to_string(),
                ],
                Some(stdin_b64),
                &format!("sandbox-code-runner:{function_id}"),
                timeout_ms,
            )
            .await?;

        if out
            .get("timed_out")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Err(CodeRunnerError::Timeout);
        }
        let stdout = str_field(&out, "stdout");
        let stderr = str_field(&out, "stderr");
        let split = crate::runner::split_sentinel(&stdout, &sentinel);
        if !split.logs.is_empty() {
            tracing::debug!(function_id = %function_id, logs = %split.logs, "handler prints");
        }
        let exit_ok = out.get("exit_code").and_then(|v| v.as_i64()) == Some(0);

        match (exit_ok, split.result) {
            (true, Some(raw)) => serde_json::from_str(&raw).map_err(|_| {
                CodeRunnerError::HandlerError(
                    "handler result is not valid JSON — return only JSON-serializable values"
                        .into(),
                )
            }),
            (false, Some(raw)) => {
                let msg = serde_json::from_str::<Value>(&raw)
                    .ok()
                    .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
                    .unwrap_or(raw);
                Err(CodeRunnerError::HandlerError(msg))
            }
            (_, None) => Err(CodeRunnerError::HandlerError(format!(
                "the runner produced no result (interpreter crash?); stderr: {}",
                stderr_tail(&stderr)
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::FakeEngine;
    use serde_json::json;

    fn cfg() -> Arc<CodeRunnerConfig> {
        Arc::new(CodeRunnerConfig::default())
    }

    /// A realistic daemon error as it arrives through the bus framing.
    fn wrapped(code: &str, msg: &str) -> String {
        format!(
            r#"remote error (invocation_failed): handler error: {{"type":"X","code":"{code}","message":"{msg}","retryable":false}}"#
        )
    }

    fn ok_exec() -> serde_json::Value {
        json!({ "stdout": "4\n", "stderr": "", "exit_code": 0, "timed_out": false,
                "duration_ms": 12, "success": true })
    }

    /// The daemon calls every eval and registration path goes through:
    /// create + guest-file plant (+ pip install for Python), then write +
    /// exec, then stop.
    fn happy_fake() -> Arc<FakeEngine> {
        let fake = FakeEngine::new();
        fake.with_response(
            "sandbox::create",
            Ok(json!({ "sandbox_id": "sb-1", "image": "node" })),
        );
        fake.with_response(
            "sandbox::fs::write",
            Ok(json!({ "bytes_written": 1, "path": "p" })),
        );
        fake.with_response("sandbox::exec", Ok(ok_exec()));
        fake.with_response(
            "sandbox::stop",
            Ok(json!({ "sandbox_id": "sb-1", "stopped": true })),
        );
        fake
    }

    fn eval_req(code: &str, lang: Option<Lang>, runtime_id: Option<String>) -> RunRequest {
        RunRequest {
            code: code.into(),
            runtime_id,
            lang,
            keep: false,
            timeout_ms: None,
        }
    }

    /// Directly insert a `RuntimeRecord` bypassing every daemon call — the
    /// fixture for tests that only care about the REUSE path (write+exec
    /// against an already-live runtime), not how it came to exist.
    fn seed_runtime(m: &RuntimeManager, lang: Lang, sandbox_id: &str) -> String {
        let id = format!("rt-{}", uuid::Uuid::new_v4());
        let record = Arc::new(RuntimeRecord {
            sandbox_id: sandbox_id.to_string(),
            lang,
            created_at: SystemTime::now(),
            exec_lock: tokio::sync::Mutex::new(()),
            namespace: Mutex::new(None),
            functions: Mutex::new(Vec::new()),
        });
        m.runtimes.lock().unwrap().insert(id.clone(), record);
        id
    }

    // ---------------------------------------------------------------
    // eval: the boot paths (no runtime_id) — one-shot and keep: true.
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn an_ephemeral_eval_boots_evals_and_destroys_its_vm() {
        let fake = happy_fake();
        fake.with_responder("sandbox::exec", |payload| {
            assert_eq!(payload["cmd"], "node");
            assert_eq!(payload["timeout_ms"], 5_000);
            let args = payload["args"].as_array().expect("argv array");
            assert_eq!(
                args[0], "/opt/sandbox-code-runner/run.mjs",
                "code runs UNDER THE RUN WRAPPER, never the bare interpreter"
            );
            assert!(args[1]
                .as_str()
                .unwrap()
                .starts_with("/tmp/sandbox-code-runner/run-"));
            Ok(ok_exec())
        });
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let out = m
            .run(eval_req("console.log(2+2)", Some(Lang::Node), None))
            .await
            .expect("eval succeeds");
        assert_eq!(out.runtime_id, None, "nothing left to address");
        assert_eq!(out.stdout, "4\n");
        assert!(out.success);
        assert!(
            m.runtimes.lock().unwrap().is_empty(),
            "an ephemeral eval must not leave an addressable runtime behind"
        );

        let ids: Vec<String> = fake.calls().into_iter().map(|(id, _)| id).collect();
        assert_eq!(
            ids,
            vec![
                "sandbox::create",
                "sandbox::fs::write", // invoke.mjs
                "sandbox::fs::write", // iii.mjs
                "sandbox::fs::write", // run.mjs
                "sandbox::fs::write", // /node_modules/iii-sdk/package.json
                "sandbox::fs::write", // /node_modules/iii-sdk/dist/index.mjs
                "sandbox::fs::write", // the code
                "sandbox::exec",
                "sandbox::stop", // the one-shot VM is destroyed
            ]
        );
        let create = fake
            .calls()
            .into_iter()
            .find(|(id, _)| id == "sandbox::create")
            .unwrap();
        assert_eq!(create.1["image"], "node");
    }

    /// Every runtime boots networked, with the guest-facing engine URL and
    /// the OTel kill-switch in its create-time env. The URL is the
    /// NORMALIZED one: a loopback IP would dodge the guest's /etc/hosts
    /// gateway mapping, so `127.0.0.1` must travel as `localhost`.
    #[tokio::test]
    async fn create_boots_with_network_and_the_guest_engine_url() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:4912");
        m.run(eval_req("1", Some(Lang::Node), None))
            .await
            .expect("eval succeeds");
        let create = fake
            .calls()
            .into_iter()
            .find(|(id, _)| id == "sandbox::create")
            .expect("created");
        assert_eq!(create.1["network"], true);
        assert_eq!(create.1["env"]["III_URL"], "ws://localhost:4912");
        assert_eq!(create.1["env"]["OTEL_ENABLED"], "false");
    }

    #[test]
    fn guest_engine_url_rewrites_loopback_ips_only() {
        assert_eq!(
            guest_engine_url("ws://127.0.0.1:49134"),
            "ws://localhost:49134"
        );
        assert_eq!(guest_engine_url("ws://[::1]:49134"), "ws://localhost:49134");
        assert_eq!(
            guest_engine_url("ws://localhost:49134"),
            "ws://localhost:49134"
        );
        assert_eq!(
            guest_engine_url("wss://engine.prod.example:443"),
            "wss://engine.prod.example:443",
            "a remote engine address must pass through untouched"
        );
    }

    /// A Python boot has one extra step between the plant and the eval:
    /// `pip install iii-sdk` (the SDK's compiled deps cannot be planted).
    /// The eval itself still runs under the run wrapper.
    #[tokio::test]
    async fn a_python_ephemeral_eval_pip_installs_the_sdk_then_runs_the_wrapper() {
        let fake = happy_fake();
        fake.with_responder("sandbox::exec", |payload| {
            let args = payload["args"].as_array().expect("argv array");
            assert_eq!(payload["cmd"], "python3");
            if args[0] == "-m" {
                assert_eq!(args[1], "pip");
                assert!(args.iter().any(|a| a == "iii-sdk"), "{args:?}");
            } else {
                assert_eq!(args[0], "/opt/sandbox-code-runner/run.py");
            }
            Ok(ok_exec())
        });
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.run(eval_req("print(2+2)", Some(Lang::Python), None))
            .await
            .expect("eval succeeds");
        let create = fake
            .calls()
            .into_iter()
            .find(|(id, _)| id == "sandbox::create")
            .unwrap();
        assert_eq!(create.1["image"], "python");
        let execs = fake
            .calls()
            .iter()
            .filter(|(id, _)| id == "sandbox::exec")
            .count();
        assert_eq!(execs, 2, "one pip install + one eval exec");
    }

    /// A failed pip install DEGRADES the runtime (its `iii` errors on
    /// first use, guest-side) — it must never take `eval` itself down.
    #[tokio::test]
    async fn a_failed_sdk_install_degrades_but_does_not_fail_the_eval() {
        let fake = happy_fake();
        fake.with_responder("sandbox::exec", |payload| {
            if payload["args"][0] == "-m" {
                return Ok(json!({ "stdout": "", "stderr": "no route to pypi",
                                   "exit_code": 1, "timed_out": false,
                                   "duration_ms": 5, "success": false }));
            }
            Ok(ok_exec())
        });
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let out = m
            .run(eval_req("print(2+2)", Some(Lang::Python), None))
            .await
            .expect("the eval must still run");
        assert!(out.success);
    }

    /// `create` plants the full guest-file table for the language —
    /// runner, iii library (NOT named iii.py — sys.path[0] shadowing),
    /// run wrapper — before anything can exec.
    #[tokio::test]
    async fn create_plants_the_guest_file_table() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.run(eval_req("print(1)", Some(Lang::Python), None))
            .await
            .expect("eval succeeds");
        let planted: Vec<(String, String)> = fake
            .calls()
            .iter()
            .filter(|(id, _)| id == "sandbox::fs::write")
            .map(|(_, p)| {
                (
                    p["path"].as_str().unwrap().to_string(),
                    p["content"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        let find = |path: &str| {
            planted
                .iter()
                .find(|(p, _)| p == path)
                .unwrap_or_else(|| panic!("{path} was not planted"))
                .1
                .clone()
        };
        assert_eq!(
            find("/opt/sandbox-code-runner/invoke.py"),
            crate::runner::INVOKE_PY
        );
        assert_eq!(
            find("/opt/sandbox-code-runner/sandbox_code_runner_iii.py"),
            crate::runner::III_PY
        );
        assert_eq!(
            find("/opt/sandbox-code-runner/run.py"),
            crate::runner::RUN_PY
        );
    }

    /// The Node table additionally carries the embedded SDK at root
    /// /node_modules, where the ESM upward walk from ANY tenant file
    /// ends.
    #[tokio::test]
    async fn a_node_create_plants_the_sdk_bundle_at_the_root() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.run(eval_req("1", Some(Lang::Node), None))
            .await
            .expect("eval succeeds");
        let planted: Vec<(String, String)> = fake
            .calls()
            .iter()
            .filter(|(id, _)| id == "sandbox::fs::write")
            .map(|(_, p)| {
                (
                    p["path"].as_str().unwrap().to_string(),
                    p["content"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        let find = |path: &str| {
            planted
                .iter()
                .find(|(p, _)| p == path)
                .unwrap_or_else(|| panic!("{path} was not planted"))
                .1
                .clone()
        };
        assert_eq!(
            find("/opt/sandbox-code-runner/invoke.mjs"),
            crate::runner::INVOKE_MJS
        );
        assert_eq!(
            find("/opt/sandbox-code-runner/iii.mjs"),
            crate::runner::III_MJS
        );
        assert_eq!(
            find("/opt/sandbox-code-runner/run.mjs"),
            crate::runner::RUN_MJS
        );
        let bundle = fake
            .calls()
            .into_iter()
            .find(|(id, p)| {
                id == "sandbox::fs::write" && p["path"] == "/node_modules/iii-sdk/dist/index.mjs"
            })
            .expect("SDK bundle planted");
        assert_eq!(bundle.1["content"], crate::runner::SDK_BUNDLE_MJS);
        assert!(
            fake.calls().iter().any(|(id, p)| {
                id == "sandbox::fs::write" && p["path"] == "/node_modules/iii-sdk/package.json"
            }),
            "the SDK's manifest must be planted beside the bundle"
        );
        let execs = fake
            .calls()
            .iter()
            .filter(|(id, _)| id == "sandbox::exec")
            .count();
        assert_eq!(execs, 1, "no install step for Node — the SDK is embedded");
    }

    #[tokio::test]
    async fn keep_true_mints_a_runtime_id_and_it_addresses_the_kept_vm() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");

        let mut req = eval_req("1", Some(Lang::Node), None);
        req.keep = true;
        let out = m.run(req).await.expect("eval succeeds");
        let id = out
            .runtime_id
            .clone()
            .expect("keep: true mints a runtime_id");
        assert!(id.starts_with("rt-"));
        assert!(m.runtimes.lock().unwrap().contains_key(&id));
        assert!(
            !fake.calls().iter().any(|(id, _)| id == "sandbox::stop"),
            "a kept VM must not be stopped"
        );

        // The minted id addresses that VM: a later eval reuses it via
        // write+exec, no second create.
        let before = fake.calls().len();
        let out2 = m
            .run(eval_req("2", None, Some(id.clone())))
            .await
            .expect("reuse succeeds");
        assert_eq!(out2.runtime_id, Some(id));
        let calls = fake.calls();
        assert_eq!(calls.len(), before + 2);
        assert_eq!(calls[before].0, "sandbox::fs::write");
        assert_eq!(calls[before + 1].0, "sandbox::exec");
    }

    /// keep: true hands out the id only on SUCCESS: an `Err` response has
    /// no `runtime_id` field, so keeping the VM would strand it in a
    /// daemon slot nothing can ever address (exactly what the pre-broker
    /// `sandbox::run keep_sandbox` path used to do on a timeout).
    #[tokio::test]
    async fn keep_true_with_a_failed_eval_destroys_the_fresh_runtime() {
        let fake = happy_fake();
        fake.with_response("sandbox::exec", Err(wrapped("S200", "deadline")));
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let mut req = eval_req("1", Some(Lang::Node), None);
        req.keep = true;
        let err = m.run(req).await.unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::timeout");
        assert!(m.runtimes.lock().unwrap().is_empty());
        assert!(
            fake.calls().iter().any(|(id, _)| id == "sandbox::stop"),
            "the unaddressable VM must be destroyed"
        );
    }

    #[tokio::test]
    async fn create_without_a_returned_sandbox_id_is_an_engine_error() {
        let fake = happy_fake();
        // Malformed daemon reply: created, but no sandbox_id came back —
        // must not silently mint an unaddressable runtime.
        fake.with_response("sandbox::create", Ok(json!({ "image": "node" })));
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let err = m
            .run(eval_req("1", Some(Lang::Node), None))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::engine");
        assert!(err.to_string().contains("no sandbox_id"), "{err}");
        assert!(m.runtimes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_timed_out_ephemeral_eval_is_a_timeout_and_still_destroys_the_vm() {
        let fake = happy_fake();
        fake.with_response(
            "sandbox::exec",
            Ok(
                json!({ "stdout": "", "stderr": "", "exit_code": serde_json::Value::Null,
                       "timed_out": true, "duration_ms": 5000, "success": false }),
            ),
        );
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let err = m
            .run(eval_req("while(1);", Some(Lang::Node), None))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::timeout");
        assert!(m.runtimes.lock().unwrap().is_empty());
        assert!(
            fake.calls().iter().any(|(id, _)| id == "sandbox::stop"),
            "the timed-out one-shot VM must still be destroyed"
        );
    }

    #[tokio::test]
    async fn a_null_exit_code_from_the_exec_maps_to_negative_one() {
        let fake = happy_fake();
        fake.with_response(
            "sandbox::exec",
            Ok(
                json!({ "stdout": "", "stderr": "boot noise", "exit_code": serde_json::Value::Null,
                       "timed_out": false, "duration_ms": 1, "success": false }),
            ),
        );
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let out = m
            .run(eval_req("1", Some(Lang::Node), None))
            .await
            .expect("a non-timeout, non-error response is a settled response");
        assert_eq!(out.exit_code, -1);
    }

    #[tokio::test]
    async fn create_gone_maps_to_a_retry_error_and_creates_nothing() {
        let fake = happy_fake();
        fake.with_response(
            "sandbox::create",
            Err(wrapped("S002", "no sandbox with that id sb-9")),
        );
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let err = m
            .run(eval_req("1", Some(Lang::Node), None))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::engine");
        assert!(!err.to_string().contains("sb-9"), "{err}");
        assert!(m.runtimes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_capacity_maps_to_capacity_and_calls_nothing_else() {
        let fake = FakeEngine::new();
        fake.with_response(
            "sandbox::create",
            Err(wrapped("S400", "max_concurrent_sandboxes reached")),
        );
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let err = m
            .run(eval_req("1", Some(Lang::Node), None))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::capacity");
        assert_eq!(fake.calls().len(), 1);
    }

    #[tokio::test]
    async fn create_timeout_wire_error_maps_to_timeout() {
        let fake = FakeEngine::new();
        fake.with_response("sandbox::create", Err(wrapped("S200", "deadline")));
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let err = m
            .run(eval_req("1", Some(Lang::Node), None))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::timeout");
    }

    /// A VM reaped MID-EVAL on the boot path: this caller never held the
    /// freshly-minted runtime id (only a kept SUCCESS hands it out), so
    /// the error must not quote it — `error.rs`'s id-quoting exception
    /// covers only ids the caller supplied. Mirrors
    /// `redact_register_error` / `redact_proxy_error`.
    #[tokio::test]
    async fn a_mid_eval_reap_on_the_boot_path_redacts_the_id_and_leaves_nothing() {
        let fake = happy_fake();
        fake.with_response("sandbox::exec", Err(wrapped("S004", "sandbox stopped")));
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let err = m
            .run(eval_req("1", Some(Lang::Node), None))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::engine");
        assert!(
            !err.to_string().contains("rt-"),
            "a boot-path caller was handed a runtime_id it never held: {err}"
        );
        assert!(err.to_string().contains("retry"), "{err}");
        assert!(m.runtimes.lock().unwrap().is_empty());
    }

    // ---------------------------------------------------------------
    // eval: the `runtime_id` reuse path (unchanged behaviour).
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn eval_with_runtime_id_reuses_the_sandbox_via_write_and_exec() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let id = seed_runtime(&m, Lang::Node, "sb-1");
        let out = m
            .run(eval_req("2", None, Some(id.clone())))
            .await
            .expect("reuse succeeds");
        assert_eq!(out.runtime_id, Some(id));
        let calls = fake.calls();
        let ids: Vec<&str> = calls.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["sandbox::fs::write", "sandbox::exec"],
            "write + exec — no create"
        );
    }

    /// The per-exec env is just the worker NAME the guest SDK client
    /// announces itself with if the code uses `iii` — the engine link
    /// (`III_URL`) is create-time env, already in the VM.
    #[tokio::test]
    async fn an_eval_exec_carries_the_guest_worker_name() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let id = seed_runtime(&m, Lang::Node, "sb-1");
        m.run(eval_req("1", None, Some(id))).await.unwrap();

        let exec = fake
            .calls()
            .into_iter()
            .find(|(id, _)| id == "sandbox::exec")
            .expect("exec happened");
        assert_eq!(exec.1["env"]["III_WORKER_NAME"], "sandbox-code-runner:run");
    }

    #[tokio::test]
    async fn a_mismatched_lang_on_an_existing_runtime_is_refused() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake, "ws://127.0.0.1:1");
        let id = seed_runtime(&m, Lang::Node, "sb-1");
        let err = m
            .run(eval_req("1", Some(Lang::Python), Some(id.clone())))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::invalid_request");
        // The matching lang is fine.
        m.run(eval_req("1", Some(Lang::Node), Some(id)))
            .await
            .expect("matching lang accepted");
    }

    #[tokio::test]
    async fn unknown_runtime_id_is_not_found() {
        let m = RuntimeManager::new(cfg(), happy_fake(), "ws://127.0.0.1:1");
        let err = m
            .run(eval_req("1", None, Some("rt-nope".into())))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::runtime_not_found");
    }

    #[tokio::test]
    async fn empty_and_oversized_code_are_invalid_requests() {
        let m = RuntimeManager::new(cfg(), happy_fake(), "ws://127.0.0.1:1");
        let err = m
            .run(eval_req("", Some(Lang::Node), None))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::invalid_request");
        let big = "x".repeat(MAX_SOURCE_BYTES + 1);
        let err = m
            .run(eval_req(&big, Some(Lang::Node), None))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::invalid_request");
    }

    #[tokio::test]
    async fn create_requires_a_lang() {
        let m = RuntimeManager::new(cfg(), happy_fake(), "ws://127.0.0.1:1");
        let err = m.run(eval_req("1", None, None)).await.unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::invalid_request");
        assert!(err.to_string().contains("lang"), "{err}");
    }

    #[tokio::test]
    async fn requested_timeout_is_clamped_on_the_reuse_path() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let id = seed_runtime(&m, Lang::Node, "sb-1");
        let mut req = eval_req("1", None, Some(id));
        req.timeout_ms = Some(999_999);
        m.run(req).await.unwrap();
        let calls = fake.calls();
        assert_eq!(calls[1].1["timeout_ms"], 30_000);
    }

    #[tokio::test]
    async fn requested_timeout_is_clamped_on_the_ephemeral_path() {
        let fake = happy_fake();
        fake.with_responder("sandbox::exec", |payload| {
            assert_eq!(payload["timeout_ms"], 30_000);
            Ok(ok_exec())
        });
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let mut req = eval_req("1", Some(Lang::Node), None);
        req.timeout_ms = Some(999_999);
        m.run(req).await.unwrap();
    }

    /// The mirror image: a caller-supplied `runtime_id` is a capability the
    /// caller already holds, so a failed eval against it must NOT reap —
    /// they can retry or tear it down themselves.
    #[tokio::test]
    async fn an_eval_failure_on_a_caller_supplied_runtime_does_not_reap() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let id = seed_runtime(&m, Lang::Node, "sb-1");

        fake.with_response("sandbox::exec", Err(wrapped("S200", "deadline")));
        let err = m
            .run(eval_req("2", None, Some(id.clone())))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::timeout");
        assert!(
            m.runtimes.lock().unwrap().contains_key(&id),
            "a caller-supplied runtime must survive a failed eval"
        );
        assert!(
            !fake.calls().iter().any(|(id, _)| id == "sandbox::stop"),
            "must not have been stopped"
        );

        // And it is still usable: a subsequent eval succeeds normally.
        fake.with_response("sandbox::exec", Ok(ok_exec()));
        let out = m
            .run(eval_req("3", None, Some(id)))
            .await
            .expect("the surviving runtime is still usable");
        assert!(out.success);
    }

    /// A failed eval (non-zero exit) is NOT an error: the response carries
    /// exit_code/stderr and the caller iterates. Only infrastructure
    /// failures are errors.
    #[tokio::test]
    async fn a_nonzero_exit_is_a_response_not_an_error() {
        let fake = happy_fake();
        fake.with_response(
            "sandbox::exec",
            Ok(
                json!({ "stdout": "", "stderr": "SyntaxError: x", "exit_code": 1,
                       "timed_out": false, "duration_ms": 3, "success": false }),
            ),
        );
        let m = RuntimeManager::new(cfg(), fake, "ws://127.0.0.1:1");
        let id = seed_runtime(&m, Lang::Node, "sb-1");
        let out = m.run(eval_req("syntax(", None, Some(id))).await.unwrap();
        assert!(!out.success);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("SyntaxError"));
    }

    /// S002/S004 on a live record: the VM was reaped behind our back. The
    /// error names the runtime, and the record is gone afterwards.
    #[tokio::test]
    async fn a_reaped_sandbox_expires_the_runtime() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let id = seed_runtime(&m, Lang::Node, "sb-1");
        fake.with_response(
            "sandbox::fs::write",
            Err(wrapped("S004", "sandbox stopped")),
        );
        let err = m
            .run(eval_req("2", None, Some(id.clone())))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::expired");
        // The record is gone: the same id is now unknown, not expired-again.
        let err = m.run(eval_req("3", None, Some(id))).await.unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::runtime_not_found");
    }

    /// The channel that actually reaches a caller: drives a REAL `S003`
    /// failure through `sandbox_call` exactly as a slow daemon would
    /// produce it, and asserts the sandbox_id the daemon embedded in its
    /// own message never appears in what the caller gets back.
    #[tokio::test]
    async fn a_concurrent_exec_error_never_leaks_the_sandbox_id_to_the_caller() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let id = seed_runtime(&m, Lang::Node, "sb-1");

        fake.with_response(
            "sandbox::exec",
            Err(wrapped(
                "S003",
                "concurrent exec on sandbox sb-1: an exec is already in flight. \
                 Exec is serialized one-at-a-time per sandbox",
            )),
        );
        let err = m
            .run(eval_req("2", None, Some(id.clone())))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::engine");
        assert!(
            !err.to_string().contains("sb-1"),
            "the caller-facing error leaked the sandbox_id: {err}"
        );
    }

    // ---------------------------------------------------------------
    // teardown: request validation (exactly one of runtime_id/namespace).
    // ---------------------------------------------------------------

    fn td_by_id(id: &str) -> TeardownRequest {
        TeardownRequest {
            runtime_id: Some(id.to_string()),
            namespace: None,
        }
    }

    fn td_by_ns(ns: &str) -> TeardownRequest {
        TeardownRequest {
            runtime_id: None,
            namespace: Some(ns.to_string()),
        }
    }

    #[tokio::test]
    async fn teardown_refuses_both_runtime_id_and_namespace() {
        let m = RuntimeManager::new(cfg(), happy_fake(), "ws://127.0.0.1:1");
        let err = m
            .teardown(TeardownRequest {
                runtime_id: Some("rt-x".into()),
                namespace: Some("app".into()),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::invalid_request");
        assert!(err.to_string().contains("not both"), "{err}");
    }

    #[tokio::test]
    async fn teardown_refuses_neither_runtime_id_nor_namespace() {
        let m = RuntimeManager::new(cfg(), happy_fake(), "ws://127.0.0.1:1");
        let err = m
            .teardown(TeardownRequest {
                runtime_id: None,
                namespace: None,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::invalid_request");
        assert!(err.to_string().contains("runtime_id"), "{err}");
        assert!(err.to_string().contains("namespace"), "{err}");
    }

    // ---------------------------------------------------------------
    // teardown: by runtime_id (a kept-eval runtime).
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn teardown_by_id_stops_the_sandbox_and_forgets_the_record() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let id = seed_runtime(&m, Lang::Node, "sb-1");
        let out = m.teardown(td_by_id(&id)).await.unwrap();
        assert!(out.torn_down);
        assert_eq!(out.runtime_id.as_deref(), Some(id.as_str()));
        assert_eq!(out.namespace, None);
        assert!(out.unregistered.is_empty());
        let stop = fake
            .calls()
            .into_iter()
            .find(|(id, _)| id == "sandbox::stop")
            .expect("stop was called");
        assert_eq!(stop.1["sandbox_id"], "sb-1");
        assert_eq!(stop.1["wait"], false);
        let err = m.teardown(td_by_id(&id)).await.unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::runtime_not_found");
    }

    /// Tearing down a runtime whose VM the daemon already reaped is
    /// success, not an error — the caller asked for it to be gone and it
    /// is.
    #[tokio::test]
    async fn teardown_of_an_already_reaped_sandbox_still_succeeds() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let id = seed_runtime(&m, Lang::Node, "sb-1");
        fake.with_response("sandbox::stop", Err(wrapped("S004", "already stopped")));
        let out = m.teardown(td_by_id(&id)).await.unwrap();
        assert!(out.torn_down);
    }

    /// Proves the serialization itself: while something is holding the
    /// runtime's `exec_lock` (standing in for an in-flight eval), a
    /// concurrent `teardown` must block before it unregisters or stops the
    /// sandbox — and must complete, calling `sandbox::stop`, only once that
    /// lock is released.
    #[tokio::test]
    async fn teardown_waits_for_an_in_flight_eval_before_stopping_the_sandbox() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let id = seed_runtime(&m, Lang::Node, "sb-1");

        let record = m.runtimes.lock().unwrap().get(&id).expect("exists").clone();
        let held = record.exec_lock.lock().await;

        let teardown_m = m.clone();
        let teardown_id = id.clone();
        let teardown_task =
            tokio::spawn(async move { teardown_m.teardown(td_by_id(&teardown_id)).await });

        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(
            !fake.calls().iter().any(|(id, _)| id == "sandbox::stop"),
            "teardown must not stop the sandbox while an eval is still in flight"
        );
        assert!(m.runtimes.lock().unwrap().is_empty());

        drop(held);
        let out = teardown_task
            .await
            .unwrap()
            .expect("teardown completes once the in-flight eval drains");
        assert!(out.torn_down);
        assert!(fake.calls().iter().any(|(id, _)| id == "sandbox::stop"));
    }

    // ---------------------------------------------------------------
    // register_function: namespace runtimes.
    // ---------------------------------------------------------------

    use crate::functions::register::RegisterRequest;

    /// Probe answers "free" — the NOT_FOUND-style error every available id
    /// produces.
    fn probe_free(fake: &FakeEngine) {
        fake.with_response(
            "engine::functions::info",
            Err("remote error (invocation_failed): NOT_FOUND: no function app::greet".into()),
        );
    }

    /// Decode a `sandbox::exec` payload's base64 stdin back into the
    /// `{sentinel, payload}` envelope the runner receives.
    fn decode_envelope(exec_payload: &serde_json::Value) -> serde_json::Value {
        use base64::Engine as _;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(
                exec_payload["stdin"]
                    .as_str()
                    .expect("stdin is a b64 string"),
            )
            .expect("stdin decodes");
        serde_json::from_slice(&raw).expect("stdin is the JSON envelope")
    }

    fn reg_req(function_id: &str, lang: Lang) -> RegisterRequest {
        RegisterRequest {
            function_id: function_id.into(),
            source: "export function handler(p) { return p; }".into(),
            description: Some("echoes".into()),
            lang,
        }
    }

    fn creates(fake: &FakeEngine) -> usize {
        fake.calls()
            .iter()
            .filter(|(id, _)| id == "sandbox::create")
            .count()
    }

    #[tokio::test]
    async fn register_creates_a_namespace_runtime_when_none_exists() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let out = m.register(reg_req("app::greet", Lang::Node)).await.unwrap();
        assert_eq!(out.function_id, "app::greet");
        assert!(out.registered);
        assert_eq!(creates(&fake), 1);

        let plant = fake
            .calls()
            .into_iter()
            .find(|(id, p)| {
                id == "sandbox::fs::write"
                    && p["path"]
                        .as_str()
                        .unwrap()
                        .starts_with("/opt/sandbox-code-runner/fns/")
            })
            .expect("source planted under /opt/sandbox-code-runner/fns/");
        assert!(plant.1["path"].as_str().unwrap().ends_with(".mjs"));
        assert_eq!(
            plant.1["content"],
            "export function handler(p) { return p; }"
        );
        assert_eq!(fake.registered_ids(), vec!["app::greet".to_string()]);
        assert_eq!(
            fake.registered_descriptions(),
            vec![("app::greet".to_string(), Some("echoes".to_string()))]
        );
    }

    #[tokio::test]
    async fn a_second_registration_in_the_same_namespace_and_lang_reuses_the_runtime() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.register(reg_req("app::a", Lang::Node)).await.unwrap();
        m.register(reg_req("app::b", Lang::Node)).await.unwrap();
        assert_eq!(creates(&fake), 1, "one namespace runtime, reused");
        assert_eq!(m.runtimes.lock().unwrap().len(), 1);
    }

    /// A runtime is single-language, so one namespace registering both gets
    /// one runtime per language rather than a refusal.
    #[tokio::test]
    async fn the_same_namespace_gets_a_separate_runtime_per_language() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.register(reg_req("app::a", Lang::Node)).await.unwrap();
        m.register(reg_req("app::b", Lang::Python)).await.unwrap();
        assert_eq!(creates(&fake), 2);
        assert_eq!(m.runtimes.lock().unwrap().len(), 2);
    }

    /// Two concurrent FIRST registrations in one namespace must produce
    /// exactly ONE microVM. `create` is a network round trip, so the check
    /// and the create have to be serialized across it — that is what
    /// `namespace_create_lock` is for.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_concurrent_first_registrations_in_one_namespace_create_exactly_one_runtime() {
        let fake = happy_fake();
        probe_free(&fake);
        fake.with_responder("sandbox::create", |_| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            Ok(json!({ "sandbox_id": "sb-1" }))
        });
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");

        let (m1, m2) = (m.clone(), m.clone());
        let (r1, r2) = tokio::join!(
            tokio::spawn(async move { m1.register(reg_req("app::a", Lang::Node)).await }),
            tokio::spawn(async move { m2.register(reg_req("app::b", Lang::Node)).await }),
        );
        r1.expect("task 1 must not panic").expect("registers");
        r2.expect("task 2 must not panic").expect("registers");

        assert_eq!(
            creates(&fake),
            1,
            "one namespace, one microVM — not one per concurrent registration"
        );
        assert_eq!(m.runtimes.lock().unwrap().len(), 1);
    }

    /// The full trigger path: the proxy execs the runner with the payload on
    /// stdin and a per-call sentinel, and returns the JSON after the
    /// sentinel.
    #[tokio::test]
    async fn a_registered_function_call_execs_the_runner_and_parses_the_result() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        fake.with_responder("sandbox::exec", |payload| {
            let args = payload["args"].as_array().expect("argv array");
            assert_eq!(args.len(), 2, "the sentinel must NOT be in argv: {args:?}");
            assert_eq!(args[0], "/opt/sandbox-code-runner/invoke.mjs");
            assert!(args[1]
                .as_str()
                .unwrap()
                .starts_with("/opt/sandbox-code-runner/fns/"));
            let env = decode_envelope(payload);
            let sentinel = env["sentinel"]
                .as_str()
                .expect("envelope carries the sentinel");
            let n = env["payload"]["n"]
                .as_i64()
                .expect("envelope carries the real payload");
            Ok(serde_json::json!({
                "stdout": format!("handler noise\n\n{sentinel}\n{{\"doubled\":{}}}\n", n * 2),
                "stderr": "", "exit_code": 0, "timed_out": false,
                "duration_ms": 3, "success": true
            }))
        });
        m.register(reg_req("app::double", Lang::Node))
            .await
            .unwrap();

        let result = fake
            .invoke("app::double", serde_json::json!({ "n": 21 }))
            .await
            .expect("call succeeds");
        assert_eq!(result, serde_json::json!({ "doubled": 42 }));

        let sentinels: Vec<String> = fake
            .calls()
            .iter()
            .filter(|(id, p)| id == "sandbox::exec" && p.get("stdin").is_some())
            .map(|(_, p)| decode_envelope(p)["sentinel"].as_str().unwrap().to_string())
            .collect();
        let exec = fake
            .calls()
            .into_iter()
            .rev()
            .find(|(id, _)| id == "sandbox::exec")
            .unwrap();
        assert_eq!(
            exec.1["timeout_ms"], 5_000,
            "registered calls run at default_timeout_ms"
        );

        fake.invoke("app::double", serde_json::json!({ "n": 1 }))
            .await
            .expect("second call succeeds");
        let after: Vec<String> = fake
            .calls()
            .iter()
            .filter(|(id, p)| id == "sandbox::exec" && p.get("stdin").is_some())
            .map(|(_, p)| decode_envelope(p)["sentinel"].as_str().unwrap().to_string())
            .collect();
        assert!(after.len() > sentinels.len(), "the second call executed");
        let unique: std::collections::HashSet<&String> = after.iter().collect();
        assert_eq!(
            unique.len(),
            after.len(),
            "sentinels must be per-call: {after:?}"
        );
    }

    /// A registered-function exec carries a worker name derived from the
    /// FUNCTION id, so a handler that uses `iii` shows up in the engine's
    /// worker list as the function it serves.
    #[tokio::test]
    async fn a_registered_invoke_carries_the_function_worker_name() {
        let fake = happy_fake();
        probe_free(&fake);
        fake.with_responder("sandbox::exec", |payload| {
            assert_eq!(
                payload["env"]["III_WORKER_NAME"],
                "sandbox-code-runner:app::env"
            );
            let sentinel = decode_envelope(payload)["sentinel"]
                .as_str()
                .unwrap()
                .to_string();
            Ok(json!({
                "stdout": format!("\n{sentinel}\nnull\n"),
                "stderr": "", "exit_code": 0, "timed_out": false,
                "duration_ms": 1, "success": true
            }))
        });
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.register(reg_req("app::env", Lang::Node)).await.unwrap();
        fake.invoke("app::env", serde_json::json!({}))
            .await
            .expect("invocation succeeds");
    }

    /// Adversarial review, backend leak: `app::greet`'s caller never
    /// supplied a `runtime_id` and never held one — unlike a direct
    /// `sandbox-code-runner::run`/`teardown` call, where `error.rs`'s id-quoting
    /// message is the documented exception.
    #[tokio::test]
    async fn a_proxy_invocation_never_leaks_the_runtime_id_to_its_caller() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.register(reg_req("app::greet", Lang::Node)).await.unwrap();
        let rt = m
            .namespaces
            .lock()
            .unwrap()
            .get(&("app::".to_string(), Lang::Node))
            .cloned()
            .expect("namespace runtime exists");

        // The race `invoke_registered`'s own doc describes: the manager's
        // record is gone (as `expire`/`teardown` leave it) but the bus
        // unregistration has not landed yet, so the proxy is still
        // reachable.
        m.runtimes.lock().unwrap().remove(&rt);

        let err = fake
            .invoke("app::greet", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.starts_with("sandbox-code-runner::expired: "), "{err}");
        assert!(
            !err.contains(rt.as_str()) && !err.contains("rt-"),
            "the proxy handed the runtime_id to a caller who never held it: {err}"
        );
    }

    #[tokio::test]
    async fn a_throwing_handler_surfaces_as_handler_error() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        fake.with_responder("sandbox::exec", |payload| {
            let env = decode_envelope(payload);
            let sentinel = env["sentinel"].as_str().unwrap();
            Ok(serde_json::json!({
                "stdout": format!("\n{sentinel}\n{{\"error\":\"ValueError: boom-3\"}}\n"),
                "stderr": "", "exit_code": 1, "timed_out": false,
                "duration_ms": 3, "success": false
            }))
        });
        m.register(reg_req("app::boom", Lang::Node)).await.unwrap();
        let err = fake
            .invoke("app::boom", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.contains("sandbox-code-runner::handler_error"), "{err}");
        assert!(err.contains("boom-3"), "{err}");
    }

    #[tokio::test]
    async fn a_crashed_runner_is_a_handler_error_naming_the_crash() {
        let fake = happy_fake();
        probe_free(&fake);
        fake.with_responder("sandbox::exec", |_| {
            Ok(serde_json::json!({
                "stdout": "", "stderr": "Killed", "exit_code": 137,
                "timed_out": false, "duration_ms": 3, "success": false
            }))
        });
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.register(reg_req("app::crash", Lang::Node)).await.unwrap();
        let err = fake
            .invoke("app::crash", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.contains("sandbox-code-runner::handler_error"), "{err}");
        assert!(err.contains("Killed"), "stderr tail included: {err}");
    }

    #[tokio::test]
    async fn a_taken_id_is_refused_before_anything_is_planted() {
        let fake = happy_fake();
        fake.with_response(
            "engine::functions::info",
            Ok(serde_json::json!({ "function_id": "app::greet", "description": "exists" })),
        );
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let err = m
            .register(reg_req("app::greet", Lang::Node))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::invalid_request");
        assert!(err.to_string().contains("already registered"), "{err}");
        assert!(fake.registered_ids().is_empty());
        // A namespace runtime WAS created (it happens before the probe) but
        // holds no functions — same shape as before this redesign, where a
        // refused registration could still leave a bare, addressable
        // runtime behind.
        assert_eq!(creates(&fake), 1);
    }

    /// Adversarial review, backend leak (mirrors
    /// `a_proxy_invocation_never_leaks_the_runtime_id_to_its_caller`, on the
    /// OTHER caller `error.rs`'s id-quoting exception was never meant to
    /// cover): a `register_function` caller never supplies or receives a
    /// `runtime_id` — the namespace runtime is resolved internally — so if
    /// a concurrent teardown races the plant and `publish`'s post-plant
    /// re-check hits `Expired`, that id must not reach the direct caller
    /// either. Simulates the race deterministically: the SECOND
    /// `sandbox::fs::write` (the source plant, inside `publish`, as
    /// opposed to the runner plant inside `create`) clears `m.runtimes` as
    /// a concurrent teardown would, right before `publish`'s re-check runs.
    #[tokio::test]
    async fn a_registration_racing_a_teardown_never_leaks_the_runtime_id_to_its_direct_caller() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let m_clone = m.clone();
        fake.with_responder("sandbox::fs::write", move |payload| {
            if payload["path"]
                .as_str()
                .unwrap_or("")
                .starts_with("/opt/sandbox-code-runner/fns/")
            {
                m_clone.runtimes.lock().unwrap().clear();
            }
            Ok(json!({ "bytes_written": 1, "path": "p" }))
        });
        let err = m
            .register(reg_req("app::greet", Lang::Node))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::engine");
        assert!(
            !err.to_string().contains("rt-"),
            "the direct register_function caller was handed a runtime_id it never held: {err}"
        );
    }

    /// An unverifiable id fails CLOSED: a FORBIDDEN probe answer must
    /// refuse, not proceed on an unknown.
    #[tokio::test]
    async fn an_inconclusive_probe_fails_closed() {
        let fake = happy_fake();
        fake.with_response(
            "engine::functions::info",
            Err("remote error: FORBIDDEN: rbac denies functions.info".into()),
        );
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let err = m
            .register(reg_req("app::greet", Lang::Node))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::engine");
        assert!(fake.registered_ids().is_empty());
    }

    #[tokio::test]
    async fn a_probe_that_cannot_dispatch_itself_fails_closed_not_free() {
        let fake = happy_fake();
        fake.with_response(
            "engine::functions::info",
            Err(
                "remote error (function_not_found): Function engine::functions::info not found"
                    .into(),
            ),
        );
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let err = m
            .register(reg_req("app::greet", Lang::Node))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::engine");
        assert!(
            fake.registered_ids().is_empty(),
            "an engine that cannot dispatch the probe must never be treated as \
             'the id is free' — nothing should have been published"
        );
    }

    #[tokio::test]
    async fn the_first_id_claims_the_namespace_for_the_runtime() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.register(reg_req("app::a", Lang::Node)).await.unwrap();
        m.register(reg_req("app::b", Lang::Node))
            .await
            .expect("same namespace ok");
        assert_eq!(creates(&fake), 1);
    }

    #[tokio::test]
    async fn malformed_function_ids_are_refused() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        for bad in [
            "noseparator",
            "::x",
            "app::",
            "My-App::x",
            "a..b::x",
            ".hidden::x",
        ] {
            let err = m.register(reg_req(bad, Lang::Node)).await.unwrap_err();
            assert_eq!(err.code(), "sandbox-code-runner::invalid_request", "{bad}");
        }
    }

    #[tokio::test]
    async fn teardown_unregisters_registered_functions() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.register(reg_req("app::a", Lang::Node)).await.unwrap();
        let out = m.teardown(td_by_ns("app")).await.unwrap();
        assert_eq!(out.unregistered, vec!["app::a".to_string()]);
        assert!(fake.registered_ids().is_empty());
        assert_eq!(fake.unregister_count(), 1);
    }

    /// The expiry path must also unregister — a bus function whose VM is
    /// gone would otherwise error forever instead of disappearing.
    #[tokio::test]
    async fn expiry_unregisters_registered_functions() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.register(reg_req("app::a", Lang::Node)).await.unwrap();
        let rt = m
            .namespaces
            .lock()
            .unwrap()
            .get(&("app::".to_string(), Lang::Node))
            .cloned()
            .unwrap();
        fake.with_response("sandbox::fs::write", Err(wrapped("S004", "reaped")));
        let _ = m.run(eval_req("2", None, Some(rt))).await.unwrap_err();
        assert!(fake.registered_ids().is_empty(), "expiry must unregister");
    }

    /// `invoke_registered` -> `sandbox_call` -> `Gone` -> `expire()` ->
    /// `(f.unregister)()` unregisters a function while ITS OWN handler
    /// future is still executing. Safe only because the caller (mirrored
    /// here by `FakeEngine::invoke`) clones the handler `Arc` out and drops
    /// the registry lock BEFORE calling it.
    #[tokio::test]
    async fn a_call_that_discovers_its_own_runtime_is_gone_unregisters_itself_without_deadlock() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.register(reg_req("app::self_destruct", Lang::Node))
            .await
            .unwrap();

        fake.with_response("sandbox::exec", Err(wrapped("S004", "reaped mid-call")));

        let err = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            fake.invoke("app::self_destruct", serde_json::json!({})),
        )
        .await
        .expect("must not deadlock")
        .expect_err("the runtime is gone");
        assert!(err.contains("sandbox-code-runner::expired"), "{err}");

        assert!(fake.registered_ids().is_empty());
        assert_eq!(fake.unregister_count(), 1);
    }

    #[tokio::test]
    async fn a_torn_down_function_is_uncallable() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.register(reg_req("app::a", Lang::Node)).await.unwrap();
        m.teardown(td_by_ns("app")).await.unwrap();
        assert!(fake.invoke("app::a", serde_json::json!({})).await.is_err());
    }

    #[tokio::test]
    async fn register_caps_are_enforced() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");

        let mut req = reg_req("app::x", Lang::Node);
        req.source = String::new();
        assert_eq!(
            m.register(req).await.unwrap_err().code(),
            "sandbox-code-runner::invalid_request"
        );

        let mut req = reg_req("app::x", Lang::Node);
        req.source = "x".repeat(MAX_SOURCE_BYTES + 1);
        assert_eq!(
            m.register(req).await.unwrap_err().code(),
            "sandbox-code-runner::invalid_request"
        );

        let long_id = format!("app::{}", "x".repeat(MAX_FUNCTION_ID_BYTES));
        assert_eq!(
            m.register(reg_req(&long_id, Lang::Node))
                .await
                .unwrap_err()
                .code(),
            "sandbox-code-runner::invalid_request"
        );

        let mut req = reg_req("app::x", Lang::Node);
        req.description = Some("d".repeat(MAX_DESCRIPTION_BYTES + 1));
        assert_eq!(
            m.register(req).await.unwrap_err().code(),
            "sandbox-code-runner::invalid_request"
        );
    }

    #[tokio::test]
    async fn max_functions_per_runtime_is_enforced() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");

        for i in 0..MAX_FUNCTIONS_PER_RUNTIME {
            m.register(reg_req(&format!("app::f{i}"), Lang::Node))
                .await
                .unwrap_or_else(|e| panic!("function {i} should register: {e}"));
        }
        assert_eq!(fake.registered_ids().len(), MAX_FUNCTIONS_PER_RUNTIME);

        let err = m
            .register(reg_req("app::one_too_many", Lang::Node))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::invalid_request");
        assert!(err.to_string().contains("already holds"), "{err}");
        assert_eq!(fake.registered_ids().len(), MAX_FUNCTIONS_PER_RUNTIME);
    }

    /// Two callers racing to register the SAME id must not both win.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_registrations_of_the_same_id_leave_exactly_one_winner() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");

        let (m1, m2) = (m.clone(), m.clone());
        let (r1, r2) = tokio::join!(
            tokio::spawn(async move { m1.register(reg_req("app::race", Lang::Node)).await }),
            tokio::spawn(async move { m2.register(reg_req("app::race", Lang::Node)).await }),
        );
        let results = [
            r1.expect("task 1 must not panic"),
            r2.expect("task 2 must not panic"),
        ];

        let ok_count = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(
            ok_count, 1,
            "exactly one registration must win: {results:?}"
        );
        let err = results
            .iter()
            .find_map(|r| r.as_ref().err())
            .expect("the loser gets a clean error, not a hang or a panic");
        assert_eq!(err.code(), "sandbox-code-runner::invalid_request");
        assert!(err.to_string().contains("already registered"), "{err}");
        assert_eq!(fake.registered_ids(), vec!["app::race".to_string()]);
    }

    #[tokio::test]
    async fn a_reservation_is_released_when_the_probe_fails() {
        let fake = happy_fake();
        fake.with_response(
            "engine::functions::info",
            Err("remote error: FORBIDDEN: rbac denies functions.info".into()),
        );
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let err = m
            .register(reg_req("app::greet", Lang::Node))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::engine");

        probe_free(&fake);
        m.register(reg_req("app::greet", Lang::Node))
            .await
            .expect("the released claim can be reserved again");
    }

    /// Every path that drops a `RegisteredFn` must also drop its local
    /// claim — otherwise a torn-down function's id is dead for the rest of
    /// the process's life.
    #[tokio::test]
    async fn teardown_releases_the_claim_for_reuse() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.register(reg_req("app::a", Lang::Node)).await.unwrap();
        m.teardown(td_by_ns("app")).await.unwrap();
        m.register(reg_req("app::a", Lang::Node))
            .await
            .expect("teardown released the claim, and a fresh namespace runtime was created");
    }

    /// `seed_static_ids` must make this worker's own ids unclaimable by a
    /// caller from the moment it is called, and that protection must
    /// survive teardown of unrelated namespaces.
    #[tokio::test]
    async fn seeded_static_ids_cannot_be_claimed_and_survive_unrelated_teardown() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.seed_static_ids(&["sandbox-code-runner::run"]);

        let before = fake.calls().len();
        let err = m
            .register(reg_req("sandbox-code-runner::run", Lang::Node))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::invalid_request");
        assert!(err.to_string().contains("already registered"), "{err}");
        // Refused by the local claim, before any probe or runtime creation.
        assert_eq!(fake.calls().len(), before);

        m.register(reg_req("app::a", Lang::Node)).await.unwrap();
        m.teardown(td_by_ns("app")).await.unwrap();
        let err = m
            .register(reg_req("sandbox-code-runner::run", Lang::Node))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::invalid_request");
    }

    // ---------------------------------------------------------------
    // teardown: by namespace.
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn teardown_by_namespace_with_no_runtime_is_not_found() {
        let m = RuntimeManager::new(cfg(), happy_fake(), "ws://127.0.0.1:1");
        let err = m.teardown(td_by_ns("app")).await.unwrap_err();
        assert_eq!(err.code(), "sandbox-code-runner::runtime_not_found");
    }

    #[tokio::test]
    async fn teardown_by_namespace_accepts_the_bare_and_double_colon_forms() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.register(reg_req("app::a", Lang::Node)).await.unwrap();
        let out = m.teardown(td_by_ns("app::")).await.unwrap();
        assert!(out.torn_down);
        assert_eq!(out.namespace.as_deref(), Some("app::"));
    }

    #[tokio::test]
    async fn teardown_by_namespace_tears_down_every_language_and_aggregates_unregistered() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.register(reg_req("app::a", Lang::Node)).await.unwrap();
        m.register(reg_req("app::b", Lang::Python)).await.unwrap();
        assert_eq!(m.runtimes.lock().unwrap().len(), 2);

        let out = m.teardown(td_by_ns("app")).await.unwrap();
        assert!(out.torn_down);
        assert_eq!(out.runtime_id, None);
        let mut got = out.unregistered.clone();
        got.sort();
        assert_eq!(got, vec!["app::a".to_string(), "app::b".to_string()]);
        assert!(m.runtimes.lock().unwrap().is_empty());
        assert!(fake.registered_ids().is_empty());

        // Both stops happened, and the namespace is free to recreate.
        assert_eq!(
            fake.calls()
                .iter()
                .filter(|(id, _)| id == "sandbox::stop")
                .count(),
            2
        );
        m.register(reg_req("app::a", Lang::Node))
            .await
            .expect("the namespace can be reused after a full teardown");
    }

    #[tokio::test]
    async fn a_malformed_teardown_namespace_is_an_invalid_request() {
        let m = RuntimeManager::new(cfg(), happy_fake(), "ws://127.0.0.1:1");
        for bad in ["", "My-App", "a..b", ".hidden", "has::colons"] {
            let err = m.teardown(td_by_ns(bad)).await.unwrap_err();
            assert_eq!(err.code(), "sandbox-code-runner::invalid_request", "{bad}");
        }
    }

    // ---------------------------------------------------------------
    // list_runtimes.
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn list_runtimes_is_empty_with_no_live_runtimes() {
        let m = RuntimeManager::new(cfg(), happy_fake(), "ws://127.0.0.1:1");
        assert!(m.list_runtimes().await.runtimes.is_empty());
    }

    #[tokio::test]
    async fn list_runtimes_carries_each_runtimes_registered_function_ids() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.register(reg_req("app::a", Lang::Node)).await.unwrap();
        m.register(reg_req("app::b", Lang::Node)).await.unwrap();

        let out = m.list_runtimes().await;
        assert_eq!(out.runtimes.len(), 1);
        let rt = &out.runtimes[0];
        assert!(rt.runtime_id.starts_with("rt-"));
        assert_eq!(rt.lang, Lang::Node);
        assert_eq!(rt.sandbox_id, "sb-1");
        assert!(rt.created_at_ms > 0);
        assert_eq!(rt.registered_functions, vec!["app::a", "app::b"]);
    }

    #[tokio::test]
    async fn list_runtimes_shows_kept_runs_and_forgets_torn_down_runtimes() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let mut req = eval_req("1", Some(Lang::Node), None);
        req.keep = true;
        let id = m.run(req).await.unwrap().runtime_id.unwrap();

        let out = m.list_runtimes().await;
        assert_eq!(out.runtimes.len(), 1);
        assert_eq!(out.runtimes[0].runtime_id, id);
        assert!(out.runtimes[0].registered_functions.is_empty());

        m.teardown(td_by_id(&id)).await.unwrap();
        assert!(m.list_runtimes().await.runtimes.is_empty());
    }

    #[tokio::test]
    async fn list_runtimes_reconciles_vm_liveness_against_the_daemon() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let mut req = eval_req("1", Some(Lang::Node), None);
        req.keep = true;
        m.run(req).await.unwrap();

        // No sandbox::list configured on the fake → reconciliation read
        // fails → unknown, never a liveness claim.
        assert_eq!(m.list_runtimes().await.runtimes[0].vm_gone, None);

        fake.with_response(
            "sandbox::list",
            Ok(json!({ "sandboxes": [{ "sandbox_id": "sb-1", "stopped": false }] })),
        );
        assert_eq!(m.list_runtimes().await.runtimes[0].vm_gone, Some(false));

        // The daemon no longer lists the VM (reaped) — flagged, not dropped:
        // teardown still needs the record to unregister and clean up.
        fake.with_response("sandbox::list", Ok(json!({ "sandboxes": [] })));
        assert_eq!(m.list_runtimes().await.runtimes[0].vm_gone, Some(true));

        // A stopped-but-listed VM counts as gone.
        fake.with_response(
            "sandbox::list",
            Ok(json!({ "sandboxes": [{ "sandbox_id": "sb-1", "stopped": true }] })),
        );
        assert_eq!(m.list_runtimes().await.runtimes[0].vm_gone, Some(true));
    }

    #[tokio::test]
    async fn a_one_shot_run_never_appears_in_list_runtimes() {
        let m = RuntimeManager::new(cfg(), happy_fake(), "ws://127.0.0.1:1");
        m.run(eval_req("1", Some(Lang::Node), None)).await.unwrap();
        assert!(m.list_runtimes().await.runtimes.is_empty());
    }

    /// Ordering is pinned with explicit creation times — real runtimes
    /// minted in one test can share a millisecond.
    #[tokio::test]
    async fn list_runtimes_sorts_newest_first() {
        let m = RuntimeManager::new(cfg(), happy_fake(), "ws://127.0.0.1:1");
        for (id, ms) in [("rt-old", 1_000u64), ("rt-new", 2_000), ("rt-mid", 1_500)] {
            let record = Arc::new(RuntimeRecord {
                sandbox_id: format!("sb-{id}"),
                lang: Lang::Node,
                created_at: UNIX_EPOCH + std::time::Duration::from_millis(ms),
                exec_lock: tokio::sync::Mutex::new(()),
                namespace: Mutex::new(None),
                functions: Mutex::new(Vec::new()),
            });
            m.runtimes.lock().unwrap().insert(id.to_string(), record);
        }
        let out = m.list_runtimes().await;
        let ids: Vec<&str> = out.runtimes.iter().map(|r| r.runtime_id.as_str()).collect();
        assert_eq!(ids, vec!["rt-new", "rt-mid", "rt-old"]);
    }

    // ---------------------------------------------------------------
    // events: lifecycle emissions.
    // ---------------------------------------------------------------

    use crate::events::{EventDeliverer, SubscriberSet};

    #[derive(Default)]
    struct RecordingDeliverer {
        seen: Mutex<Vec<Value>>,
    }

    impl EventDeliverer for RecordingDeliverer {
        fn deliver(&self, _function_id: &str, payload: Value) {
            self.seen.lock().unwrap().push(payload);
        }
    }

    /// Wire a recording emitter with one subscriber, so every emission is
    /// captured synchronously — no spawn, no bus.
    fn record_events(m: &RuntimeManager) -> Arc<RecordingDeliverer> {
        let subscribers = SubscriberSet::new();
        subscribers.insert("t1".to_string(), "console::recv".to_string());
        let recorder = Arc::new(RecordingDeliverer::default());
        m.set_events(Emitter::new(subscribers, recorder.clone()));
        recorder
    }

    #[tokio::test]
    async fn a_kept_run_emits_runtime_created_then_run_settled() {
        let m = RuntimeManager::new(cfg(), happy_fake(), "ws://127.0.0.1:1");
        let recorder = record_events(&m);
        let mut req = eval_req("1", Some(Lang::Node), None);
        req.keep = true;
        let id = m.run(req).await.unwrap().runtime_id.unwrap();

        let seen = recorder.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0]["kind"], "runtime_created");
        assert_eq!(seen[0]["runtime_id"], json!(id));
        assert_eq!(seen[0]["lang"], "node");
        assert_eq!(seen[0]["sandbox_id"], "sb-1");
        assert_eq!(seen[1]["kind"], "run_settled");
        assert_eq!(seen[1]["runtime_id"], json!(id));
    }

    /// A one-shot run's VM never persists: no runtime_created, no
    /// teardown, and its run_settled carries no runtime_id.
    #[tokio::test]
    async fn a_one_shot_run_emits_only_run_settled_without_a_runtime_id() {
        let m = RuntimeManager::new(cfg(), happy_fake(), "ws://127.0.0.1:1");
        let recorder = record_events(&m);
        m.run(eval_req("1", Some(Lang::Node), None)).await.unwrap();

        let seen = recorder.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0]["kind"], "run_settled");
        assert_eq!(seen[0]["lang"], "node");
        assert!(seen[0].get("runtime_id").is_none());
    }

    #[tokio::test]
    async fn register_emits_runtime_created_then_function_registered() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let recorder = record_events(&m);
        m.register(reg_req("app::greet", Lang::Node)).await.unwrap();

        let seen = recorder.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0]["kind"], "runtime_created");
        assert_eq!(seen[0]["sandbox_id"], "sb-1");
        assert_eq!(seen[1]["kind"], "function_registered");
        assert_eq!(seen[1]["function_id"], "app::greet");
        assert_eq!(seen[1]["runtime_id"], seen[0]["runtime_id"]);
    }

    /// Reaper-driven removal is a lifecycle change too: `expire` (S002/S004
    /// behind our back) emits the same teardown event a real teardown does.
    #[tokio::test]
    async fn an_expired_runtime_emits_a_teardown_event() {
        let fake = happy_fake();
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        let id = seed_runtime(&m, Lang::Node, "sb-1");
        let recorder = record_events(&m);
        fake.with_response("sandbox::fs::write", Err(wrapped("S004", "reaped")));
        let _ = m
            .run(eval_req("2", None, Some(id.clone())))
            .await
            .unwrap_err();

        let seen = recorder.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0]["kind"], "teardown");
        assert_eq!(seen[0]["runtime_id"], json!(id));
    }

    #[tokio::test]
    async fn teardown_by_namespace_emits_one_event_per_destroyed_runtime() {
        let fake = happy_fake();
        probe_free(&fake);
        let m = RuntimeManager::new(cfg(), fake.clone(), "ws://127.0.0.1:1");
        m.register(reg_req("app::a", Lang::Node)).await.unwrap();
        m.register(reg_req("app::b", Lang::Python)).await.unwrap();

        let recorder = record_events(&m);
        m.teardown(td_by_ns("app")).await.unwrap();

        let seen = recorder.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen.iter().all(|e| e["kind"] == "teardown"));
        assert!(seen
            .iter()
            .all(|e| e["runtime_id"].as_str().unwrap().starts_with("rt-")));
    }
}
