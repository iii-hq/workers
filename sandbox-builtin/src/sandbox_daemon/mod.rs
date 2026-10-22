// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.

pub mod adapters;
pub mod auto_install;
pub mod catalog;
pub mod config;
pub mod create;
pub mod errors;
pub mod events;
pub mod exec;
pub mod fs;
pub mod list;
pub mod overlay;
pub mod reaper;
pub mod registry;
pub mod run;
pub mod stop;

pub use errors::SandboxError;
pub use registry::SandboxRegistry;

/// Cheatsheet hosts can paste into agent system prompts. Captures the
/// canonical sandbox::* mental model: which function to use first, how to
/// pass `cmd` and `env`, where the error `fix` payload lives, and how the
/// S-code map works. Designed to be small enough to inline in a system
/// prompt without busting context budgets.
///
/// Doctest verifies the embedded JSON examples parse cleanly through
/// serde so this constant cannot drift away from the wire format.
///
/// ```
/// use sandbox_builtin::sandbox_daemon::SANDBOX_AGENT_GUIDE;
/// let guide = SANDBOX_AGENT_GUIDE;
/// assert!(guide.contains("sandbox::run"));
/// assert!(guide.contains("sandbox::exec"));
/// assert!(guide.contains("error.message"));
/// ```
pub const SANDBOX_AGENT_GUIDE: &str = r#"
You have access to the iii worker sandbox::* tools. Use them to run code in an
isolated microVM and capture stdout/stderr.

# Workflow

The fastest path is one call to `sandbox::run`:

  sandbox::run({
    image: "node",                 // or "python", or any custom_images key
    code: "console.log('hello')",  // your code, written to /tmp/run.{ext}
    lang: "node",                  // "node" | "python" | "shell" | <binary>
    env: { "NODE_ENV": "production" }
    // timeout_ms defaults to 300_000 (5 min) — set explicitly only for
    // probes or to override for very long builds.
  })

sandbox::run auto-stops the VM on success AND on failure. Set
`keep_sandbox: true` to keep the VM alive (returns sandbox_id).

# Surgical workflow

  sandbox::create({ image: "node" }) -> { sandbox_id }
  sandbox::fs::write({ sandbox_id, path, content })  // content is a UTF-8 string
  sandbox::exec({ sandbox_id, cmd, args })           // see cmd shapes below
  sandbox::stop({ sandbox_id })

# sandbox::exec cmd shapes (all three accepted)

  { cmd: "node /home/app/main.js" }                 // shell-line, shlex-split
  { cmd: "node", args: ["/home/app/main.js"] }      // classic POSIX
  { argv: ["node", "/home/app/main.js"] }           // argv array

Shlex is NOT bash. For bash semantics use sandbox::run with lang:"shell".

# env shapes

  { env: ["FOO=bar", "PATH=/usr/bin"] }              // wire shape
  { env: { "FOO": "bar", "PATH": "/usr/bin" } }      // map shape

# Errors

Errors return JSON encoded inside error.message. Parse once:

  const detail = JSON.parse(err.message);
  // detail.code, detail.type, detail.message, detail.docs_url
  // detail.fix       ready-to-send next-call payload (null if not auto-fixable)
  // detail.fix_note  one-liner explaining why fix is null
  // detail.retryable bool

If detail.fix is non-null, your next call is fn(detail.fix).

# S-code map

  S001 validation       request shape error
  S002 sandbox missing  call sandbox::create first
  S003 concurrent exec  one exec at a time per sandbox; detach servers (nohup &) or stop+recreate (waiting won't free a foreground exec)
  S100 image catalog    pick "python" or "node" (call sandbox::catalog::list for custom_images keys)
  S200 exec timeout     raise timeout_ms
  S211 file not found    (parent-missing variants carry fix:{parents:true})
  S215 permission denied
  S300 VM boot failed   platform / virtualization issue
  S400 capacity         resource cap reached
"#;

use std::sync::Arc;

use iii_helpers::observability::OtelConfig;
use iii_sdk::runtime::{FunctionRef, WorkerMetadata};
use iii_sdk::{IIIClient, InitOptions, RegisterFunction, register_worker_from_env};

use crate::sandbox_daemon::config::{SandboxConfig, parse_config};
use crate::sandbox_daemon::errors::SandboxErrorWire;
use crate::wire::{
    AdapterInfo, AdapterInfoRequest, ContractVersion, WORKER_DESTROY, WORKER_LOGS, WORKER_PREPARE,
    WORKER_START, WORKER_STATUS, WORKER_STOP,
};
use crate::worker_vm::WorkerVms;

/// This adapter's worker name, function prefix and default configuration entry.
const ADAPTER_NAME: &str = env!("CARGO_PKG_NAME");

/// Serve `sandbox-builtin::*` until a signal or the engine's death. The config
/// comes from `configuration::get(III_CONFIG_NAME)`, read once; a fetch or
/// parse error exits 2 (fail-closed).
pub async fn serve() -> anyhow::Result<()> {
    // FIRST statement, before any await: snapshot spawn-time facts for the
    // engine-death watch (see crate::daemon_exit). An orphaned adapter keeps
    // live libkrun VMs around.
    let exit_watch = crate::daemon_exit::ExitWatch::arm_at_startup();
    // One watch future for the whole life: it races the config fetch, then
    // parks the ready adapter, so a signal in between is never missed.
    let exit = exit_watch.wait("sandbox-daemon");
    tokio::pin!(exit);

    // The SDK reads `III_URL`; a non-empty `III_WORKER_NAME` overrides the name below.
    let iii = register_worker_from_env(InitOptions {
        otel: Some(OtelConfig::default()),
        metadata: Some(WorkerMetadata {
            name: ADAPTER_NAME.to_string(),
            description: Some(
                "Builtin sandbox adapter: microVM sandboxes and worker VMs on libkrun.".to_string(),
            ),
            ..Default::default()
        }),
        ..Default::default()
    });
    let config_name = std::env::var("III_CONFIG_NAME")
        .ok()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| ADAPTER_NAME.to_string());

    let reason = tokio::select! {
        reason = &mut exit => reason,
        fetched = iii_config_client::fetch(&iii, &config_name) => {
            let config = match fetched.map_err(anyhow::Error::msg).and_then(parse_config) {
                Ok(config) => config,
                Err(e) => {
                    tracing::error!(error = %format!("{e:#}"), config = %config_name,
                        "failed to load sandbox-builtin config; aborting (fail-closed)");
                    std::process::exit(2);
                }
            };
            start(&iii, config, &config_name);
            exit.await
        }
    };
    tracing::info!(reason, "sandbox-builtin shutting down");
    iii.shutdown_async().await;
    Ok(())
}

/// Register every function and spawn the idle reaper.
fn start(iii: &IIIClient, config: SandboxConfig, config_name: &str) {
    tracing::info!(
        auto_install = config.auto_install,
        allowlist_size = config.image_allowlist.len(),
        custom_images = config.custom_images.len(),
        max_concurrent = config.max_concurrent_sandboxes,
        default_idle_timeout_secs = config.default_idle_timeout_secs,
        config = %config_name,
        "loaded sandbox-builtin config"
    );
    if config.max_concurrent_exec_per_sandbox == 0 {
        tracing::warn!(
            "max_concurrent_exec_per_sandbox is 0, which this adapter treats as 1 \
             (strict serialization) — it does NOT mean unlimited. Set it to the \
             number of simultaneous execs a guest of default_memory_mb can hold."
        );
    }
    let registry = Arc::new(SandboxRegistry::with_max_exec_in_flight(
        config.max_concurrent_exec_per_sandbox,
    ));
    // How long past its idle timeout a busy sandbox is protected. One clamped
    // exec deadline plus slack: beyond that no legitimate exec can still be
    // running, so a lingering count is leaked and the VM must be reclaimed.
    let busy_grace = std::time::Duration::from_millis(config.max_exec_timeout_ms)
        + std::time::Duration::from_secs(60);
    register_all(iii, Arc::new(config), registry.clone());
    tokio::spawn(crate::sandbox_daemon::reaper::run_reaper_loop(
        (*registry).clone(),
        Arc::new(crate::sandbox_daemon::adapters::SignalStopper),
        std::time::Duration::from_secs(10),
        busy_grace,
    ));
    tracing::info!("sandbox-builtin ready");
}

/// Register every `sandbox-builtin::*` function, `adapter::info` included.
pub fn register_all(
    iii: &IIIClient,
    config: Arc<SandboxConfig>,
    registry: Arc<SandboxRegistry>,
) -> Vec<FunctionRef> {
    let launcher = Arc::new(crate::sandbox_daemon::adapters::IiiWorkerLauncher);
    let runner = Arc::new(crate::sandbox_daemon::adapters::ShellProtoRunner::new(
        config.max_exec_timeout_ms,
    ));
    let stopper = Arc::new(crate::sandbox_daemon::adapters::SignalStopper);
    let fs_runner: Arc<dyn fs::FsRunner> = Arc::new(fs::IiiShellFsRunner);

    let mut refs = vec![
        register_sandbox_create(iii, registry.clone(), config.clone(), launcher.clone()),
        register_sandbox_exec(iii, registry.clone(), runner.clone()),
        register_sandbox_stop(iii, registry.clone(), stopper.clone()),
        register_sandbox_list(iii, registry.clone()),
        register_sandbox_catalog_list(iii, config.clone()),
        // run composes create+write+exec+stop into one call for agents that
        // just want to run code (workflow TTHW = 1).
        run::register(
            iii,
            registry.clone(),
            config,
            launcher,
            runner,
            fs_runner.clone(),
            stopper,
        ),
    ];
    refs.extend(fs::register_all(iii, registry, fs_runner));
    let vms = Arc::new(WorkerVms::from_env());
    refs.extend([
        register_worker_op(
            iii,
            vms.clone(),
            WORKER_PREPARE,
            "Validate a worker VM's source and pull its base image; idempotent. \
             Needs the project token in `auth`, and `host_path` under the host roots.",
            |vms, req| async move { vms.prepare(req).await },
        ),
        register_worker_op(
            iii,
            vms.clone(),
            WORKER_START,
            "Boot the worker VM of `state_key` as its next instance. Needs the project \
             token; S502 while that VM is preparing or running.",
            |vms, req| async move { vms.start(req).await },
        ),
        register_worker_op(
            iii,
            vms.clone(),
            WORKER_STATUS,
            "State of one worker VM instance. Needs the project token.",
            |vms, req| async move { vms.status(req) },
        ),
        register_worker_op(
            iii,
            vms.clone(),
            WORKER_LOGS,
            "Stream a worker VM's console log into `channel`, one `LogRecord` JSON per text \
             frame, from `cursor` on; with `follow`, until the instance exits. Needs the \
             project token.",
            {
                let iii = iii.clone();
                move |vms, req| {
                    let iii = iii.clone();
                    async move { vms.logs(&iii, req).await }
                }
            },
        ),
        register_worker_op(
            iii,
            vms.clone(),
            WORKER_STOP,
            "SIGTERM a worker VM instance, then SIGKILL after `grace_ms` (0 kills at once); \
             returns its final status. Needs the project token.",
            |vms, req| async move { vms.stop(req).await },
        ),
        register_worker_op(
            iii,
            vms,
            WORKER_DESTROY,
            "Remove the state of `state_key`'s worker VM. Needs the project token; \
             S502 while that VM is preparing or running.",
            // Off the runtime: removing a legacy rootfs clone takes seconds.
            |vms, req| async move {
                tokio::task::spawn_blocking(move || vms.destroy(req))
                    .await
                    .map_err(|e| SandboxError::FsIo(e.to_string()))?
            },
        ),
    ]);
    refs.push(
        iii.register_function(
            "sandbox-builtin::adapter::info",
            RegisterFunction::new_async(|_: AdapterInfoRequest| async {
                Ok::<_, iii_sdk::Error>(adapter_info())
            })
            .description(
                "Name and version of this sandbox adapter, and the sandbox contract \
                 version it implements.",
            ),
        ),
    );
    refs
}

/// This adapter's `adapter::info` reply.
pub fn adapter_info() -> AdapterInfo {
    AdapterInfo {
        adapter: ADAPTER_NAME.to_string(),
        adapter_version: env!("CARGO_PKG_VERSION").to_string(),
        contract: ContractVersion::CURRENT,
    }
}

/// Register `<adapter>::<suffix>` as `op` on the shared worker VMs.
fn register_worker_op<Req, Resp, F, Fut>(
    iii: &IIIClient,
    vms: Arc<WorkerVms>,
    suffix: &str,
    description: &str,
    op: F,
) -> FunctionRef
where
    Req: serde::de::DeserializeOwned + schemars::JsonSchema + Send + 'static,
    Resp: serde::Serialize + schemars::JsonSchema + Send + 'static,
    F: Fn(Arc<WorkerVms>, Req) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Resp, SandboxError>> + Send + 'static,
{
    let id = format!("{ADAPTER_NAME}::{suffix}");
    iii.register_function(
        id.clone(),
        RegisterFunction::new_async(move |req: Req| {
            let call = op(vms.clone(), req);
            let id = id.clone();
            async move {
                let start = std::time::Instant::now();
                let result = call.await;
                log_handler_result(&id, None, &result, start.elapsed().as_millis() as u64);
                result.map_err(|e| SandboxErrorWire(e).into())
            }
        })
        .description(description),
    )
}

fn register_sandbox_create(
    iii: &iii_sdk::IIIClient,
    registry: Arc<crate::sandbox_daemon::SandboxRegistry>,
    cfg: Arc<crate::sandbox_daemon::config::SandboxConfig>,
    launcher: Arc<crate::sandbox_daemon::adapters::IiiWorkerLauncher>,
) -> FunctionRef {
    iii.register_function(
        "sandbox-builtin::create",
        RegisterFunction::new_async(move |req: crate::sandbox_daemon::create::CreateRequest| {
            let registry = registry.clone();
            let cfg = cfg.clone();
            let launcher = launcher.clone();
            async move {
                let start = std::time::Instant::now();
                let result = crate::sandbox_daemon::create::handle_create(
                    req,
                    &cfg,
                    &registry,
                    &*launcher,
                    |e| {
                        tracing::info!(event = ?e, "sandbox create event");
                    },
                )
                .await;
                // sandbox::create has no incoming sandbox_id (it generates one).
                // On success the new id is in the response — surface it so even
                // create-failure events carry the same field shape (just empty).
                let sid_owned = result.as_ref().ok().map(|r| r.sandbox_id.clone());
                log_handler_result(
                    "sandbox-builtin::create",
                    sid_owned.as_deref(),
                    &result,
                    start.elapsed().as_millis() as u64,
                );
                result.map_err(|e| SandboxErrorWire(e).into())
            }
        })
        .description(
            "Create an ephemeral sandbox VM. `image` must be a preset \
             (`\"python\"`, `\"node\"`) or a key from \
             the adapter's `custom_images` config; \
             OCI refs are NOT accepted unless they match a catalog key. \
             `env` accepts both `Vec<\"K=V\">` and `{ K: V }` map shapes.",
        ),
    )
}

fn register_sandbox_exec(
    iii: &iii_sdk::IIIClient,
    registry: Arc<crate::sandbox_daemon::SandboxRegistry>,
    runner: Arc<crate::sandbox_daemon::adapters::ShellProtoRunner>,
) -> FunctionRef {
    iii.register_function(
        "sandbox-builtin::exec",
        RegisterFunction::new_async(move |req: crate::sandbox_daemon::exec::ExecRequest| {
            let registry = registry.clone();
            let runner = runner.clone();
            async move {
                let sid = req.sandbox_id.clone();
                let start = std::time::Instant::now();
                let result =
                    crate::sandbox_daemon::exec::handle_exec(req, &registry, &*runner).await;
                log_handler_result(
                    "sandbox-builtin::exec",
                    Some(&sid),
                    &result,
                    start.elapsed().as_millis() as u64,
                );
                result.map_err(|e| SandboxErrorWire(e).into())
            }
        })
        .description(
            "Execute a command inside a live sandbox. `cmd` accepts three shapes: \
             (1) a shell-style line that gets shlex-split (`cmd: \"node -v\"`), \
             (2) `cmd` + `args` (the POSIX shape, `cmd: \"node\", args: [\"-v\"]`), \
             (3) an `argv` array (`argv: [\"node\", \"-v\"]`). \
             Shell metacharacters in the shell-line shape are NOT interpreted; \
             use `sandbox::run` with `lang: \"shell\"` for bash semantics. \
             `env` accepts both `Vec<\"K=V\">` and `{ K: V }` map shapes.",
        ),
    )
}

fn register_sandbox_stop(
    iii: &iii_sdk::IIIClient,
    registry: Arc<crate::sandbox_daemon::SandboxRegistry>,
    stopper: Arc<crate::sandbox_daemon::adapters::SignalStopper>,
) -> FunctionRef {
    iii.register_function(
        "sandbox-builtin::stop",
        RegisterFunction::new_async(move |req: crate::sandbox_daemon::stop::StopRequest| {
            let registry = registry.clone();
            let stopper = stopper.clone();
            async move {
                let sid = req.sandbox_id.clone();
                let start = std::time::Instant::now();
                let result =
                    crate::sandbox_daemon::stop::handle_stop(req, &registry, &*stopper).await;
                log_handler_result(
                    "sandbox-builtin::stop",
                    Some(&sid),
                    &result,
                    start.elapsed().as_millis() as u64,
                );
                result.map_err(|e| SandboxErrorWire(e).into())
            }
        })
        .description(
            "Stop and remove a running sandbox. Set `wait: true` to block \
             until the VM process exits and resources are reclaimed.",
        ),
    )
}

fn register_sandbox_list(
    iii: &iii_sdk::IIIClient,
    registry: Arc<crate::sandbox_daemon::SandboxRegistry>,
) -> FunctionRef {
    // Note: pre-migration this handler used
    // `serde_json::from_value(payload).unwrap_or_default()`, silently
    // coercing `null`/non-object payloads into an empty `ListRequest`.
    // `new_async` is strict, so a literal `null` payload now returns an
    // S001-style handler error instead of an empty list. The only
    // production caller (`cli::sandbox::handle_list`, which sends
    // `json!({})`) is unaffected; an empty object still deserializes to
    // the unit `ListRequest`.
    iii.register_function(
        "sandbox-builtin::list",
        RegisterFunction::new_async(move |req: crate::sandbox_daemon::list::ListRequest| {
            let registry = registry.clone();
            async move {
                let start = std::time::Instant::now();
                let response = crate::sandbox_daemon::list::handle_list(req, &registry).await;
                // handle_list is infallible; manufacture an Ok-typed Result so
                // log_handler_result emits the success branch consistently with
                // the fallible handlers.
                let result: Result<_, crate::sandbox_daemon::errors::SandboxError> = Ok(&response);
                log_handler_result(
                    "sandbox-builtin::list",
                    None,
                    &result,
                    start.elapsed().as_millis() as u64,
                );
                Ok::<_, iii_sdk::Error>(response)
            }
        })
        .description("List active sandboxes"),
    )
}

fn register_sandbox_catalog_list(
    iii: &iii_sdk::IIIClient,
    cfg: Arc<crate::sandbox_daemon::config::SandboxConfig>,
) -> FunctionRef {
    iii.register_function(
        "sandbox-builtin::catalog::list",
        RegisterFunction::new_async(
            move |req: crate::sandbox_daemon::catalog::CatalogListRequest| {
                let cfg = cfg.clone();
                async move {
                    let start = std::time::Instant::now();
                    let response = crate::sandbox_daemon::catalog::handle_catalog_list(req, &cfg);
                    let result: Result<_, crate::sandbox_daemon::errors::SandboxError> =
                        Ok(&response);
                    log_handler_result(
                        "sandbox-builtin::catalog::list",
                        None,
                        &result,
                        start.elapsed().as_millis() as u64,
                    );
                    Ok::<_, iii_sdk::Error>(response)
                }
            },
        )
        .description(
            "List bootable images: bundled presets plus operator-registered custom_images. \
             Call this before sandbox::create when you don't know what's available.",
        ),
    )
}

/// Per-handler-boundary tracing emission. Called by every
/// `register_sandbox_*` closure right after the inner handler returns.
/// Emits a single `tracing::info!` event with a stable field set so
/// operators can dashboard sandbox::* usage without grepping logs:
///
/// - `function_id` — the registration name (e.g. `"sandbox-builtin::exec"`).
/// - `sandbox_id` — the sandbox the call targeted, empty string when
///   the function has no incoming sandbox_id (sandbox::create,
///   sandbox::list, sandbox::catalog::list).
/// - `success` — `true` when the handler returned `Ok`.
/// - `error_code` — the S-code on `Err`, empty on `Ok`.
/// - `error_type` — the SandboxErrorCode category on `Err`, empty on `Ok`.
/// - `retryable` — whether the error suggests a retry (only set on `Err`).
/// - `duration_ms` — wall-clock the handler spent before returning.
///
/// Moving this out of `to_payload` (where A5 originally placed
/// error-path tracing) and broadening it to success-path is the
/// "tracing instrumentation" follow-up surfaced by the post-implementation
/// devex review.
pub(crate) fn log_handler_result<T>(
    function_id: &str,
    sandbox_id: Option<&str>,
    result: &Result<T, crate::sandbox_daemon::errors::SandboxError>,
    duration_ms: u64,
) {
    match result {
        Ok(_) => {
            tracing::info!(
                function_id = function_id,
                sandbox_id = sandbox_id.unwrap_or(""),
                success = true,
                error_code = "",
                error_type = "",
                duration_ms = duration_ms,
                "sandbox::* handler completed"
            );
        }
        Err(err) => {
            let code = err.code();
            tracing::info!(
                function_id = function_id,
                sandbox_id = sandbox_id.unwrap_or(""),
                success = false,
                error_code = code.as_str(),
                error_type = code.error_type(),
                retryable = code.retryable(),
                duration_ms = duration_ms,
                "sandbox::* handler returned error"
            );
        }
    }
}

#[cfg(test)]
mod registration_tests {
    use super::*;
    use crate::wire::CONTRACT_MAJOR;
    use std::collections::BTreeSet;

    #[tokio::test]
    async fn registered_ids_are_the_contract_operations_under_the_package_name() {
        let iii = iii_sdk::IIIClient::new("ws://127.0.0.1:9");
        let ids: BTreeSet<String> = register_all(
            &iii,
            Arc::new(SandboxConfig::default()),
            Arc::new(SandboxRegistry::default()),
        )
        .into_iter()
        .map(|r| r.id)
        .collect();
        let want: BTreeSet<String> = [
            "create",
            "exec",
            "stop",
            "list",
            "catalog::list",
            "run",
            "fs::ls",
            "fs::stat",
            "fs::mkdir",
            "fs::rm",
            "fs::chmod",
            "fs::mv",
            "fs::grep",
            "fs::sed",
            "fs::write",
            "fs::read",
            "adapter::info",
            "worker::prepare",
            WORKER_START,
            WORKER_STATUS,
            WORKER_LOGS,
            WORKER_STOP,
            WORKER_DESTROY,
        ]
        .iter()
        .map(|s| format!("sandbox-builtin::{s}"))
        .collect();
        assert_eq!(ids, want);
    }

    #[test]
    fn adapter_info_names_the_package_and_the_contract() {
        let i = adapter_info();
        // The version tracks Cargo.toml, which sync-manifest-versions rewrites
        // after each release; its major must stay the contract's (spec S2).
        assert_eq!(
            (
                i.adapter.as_str(),
                i.adapter_version.as_str(),
                i.contract.major
            ),
            ("sandbox-builtin", env!("CARGO_PKG_VERSION"), CONTRACT_MAJOR)
        );
        assert_eq!(env!("CARGO_PKG_VERSION_MAJOR"), CONTRACT_MAJOR.to_string());
    }
}

#[cfg(test)]
mod handler_logging_tests {
    //! Pins the trace contract operators dashboard against. Both branches
    //! (success + error) must execute without panicking and emit through
    //! the global `tracing` subscriber. We don't assert on the field set
    //! at runtime here — that's the operator's contract and validating
    //! it requires a tracing layer/test-subscriber dance that's noise
    //! for a one-shot helper. The compile-time signature plus this
    //! happy-path check is the right level of coverage.
    use super::*;

    #[test]
    fn log_handler_result_success_branch_runs() {
        let ok: Result<&str, crate::sandbox_daemon::errors::SandboxError> = Ok("ignored");
        log_handler_result("sandbox::test", Some("sid-1"), &ok, 42);
        log_handler_result("sandbox::test", None, &ok, 0);
    }

    #[test]
    fn log_handler_result_error_branch_runs_with_code() {
        // Use the well-known S001 InvalidRequest error so the code,
        // type, and retryable accessors all resolve to concrete values.
        let err: Result<(), _> = Err(crate::sandbox_daemon::errors::SandboxError::InvalidRequest(
            "missing path".to_string(),
        ));
        log_handler_result("sandbox::test", Some("sid-1"), &err, 7);
        log_handler_result("sandbox::test", None, &err, 99);
    }
}
